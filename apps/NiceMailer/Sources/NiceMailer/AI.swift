import Foundation

// The AI assistance in the app (`specifications/ia.md`, section 11): the
// types of the contract, and the rules of the AI panel as pure functions,
// tested without a window (`AITests`). The core does the work (`ai_run`,
// `ai_apply`…, `docs/ai.md`); the app shows the menu, the text as it comes
// and the proposal, and passes the user's answer on.

// MARK: - The contract

/// A prompt of an AI menu (`ai_list_prompts`).
struct AIPromptView: Decodable, Equatable {
    let id: String
    /// Its letter in the menu.
    let key: String
    let label: String
    let scope: String
    let output: String
    /// A submenu (rewrite: more formal, shorter…); empty when none.
    let variants: [AIVariantView]
    /// `builtin` or `user`.
    let source: String
    /// Whether this version of the core can run it yet.
    let available: Bool
}

struct AIVariantView: Decodable, Equatable {
    let key: String
    let label: String
}

/// `ai_status` (only what the app uses).
struct AIStatusResult: Decodable {
    let enabled: Bool
    let ready: Bool
    let reason: String?
}

/// `ai_run`.
struct AIRunResult: Decodable {
    let runId: Int
    /// Whether text comes as `AiDelta` events.
    let streamed: Bool
}

/// A piece of a word diff: `same`, `removed` or `added`.
struct AIDiffPiece: Decodable, Equatable {
    let kind: String
    let text: String
}

/// One segment of the draft rewritten.
struct AISegmentChange: Decodable, Equatable {
    let old: String
    let new: String
    let diff: [AIDiffPiece]
}

/// What a finished run proposes.
///
/// Swift note: the JSON carries its kind in a `kind` field
/// (`{"kind": "replace", …}`); a hand-written `init(from:)` reads it first,
/// then the fields of that kind, since `Decodable` cannot derive that for an
/// enum with associated values in this shape.
enum AIProposal: Decodable, Equatable {
    case replace(segments: [AISegmentChange], unchanged: Bool)
    case subjects([String])

    private enum Keys: String, CodingKey {
        case kind, segments, unchanged, items
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "replace":
            self = .replace(segments: try c.decode([AISegmentChange].self, forKey: .segments),
                            unchanged: try c.decode(Bool.self, forKey: .unchanged))
        case "subjects":
            self = .subjects(try c.decode([String].self, forKey: .items))
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unknown proposal \(other)")
        }
    }
}

/// `ai_get_run` (only what the app uses).
struct AIRunView: Decodable {
    let state: String
    let proposal: AIProposal?
    let error: String?
}

/// `ai_apply`.
struct AIApplyResult: Decodable {
    let draft: DraftView
}

/// The error codes of the AI (`nm-api`, `error_code`).
enum AIError {
    /// `ai.enabled` is off.
    static let disabled = 1017
    /// Any AI error; its own code is in `data["code"]`.
    static let ai = 1018
}

// MARK: - A run in the panel

/// The run shown in the AI panel of the reply, from its start to the
/// user's answer.
struct AIRun: Equatable {
    let runId: Int
    /// The prompt's label (and its variant's), the title of the panel.
    let title: String
    /// The opening of the reply it was started from (`Composer.opened`):
    /// what comes for another reply is dropped.
    let composer: UUID
    /// The text received so far.
    var text = ""
    var phase: Phase = .running

    enum Phase: Equatable {
        case running
        case proposal(AIProposal)
        case failed(String)
    }
}

/// What the keys do while the AI panel is open. The panel takes every key
/// (the text cannot change under a proposal): the answer, or nothing.
enum AIKeyAction: Equatable {
    case accept
    /// A subject of the list, from 0.
    case choose(Int)
    case refuse
    /// Stop the run still going.
    case cancel
    /// Close a panel that has nothing to apply (a failure, no change).
    case close
    /// Not an answer: the status bar says which keys are.
    case hint
}

enum AI {
    /// The answer a key gives in the panel (`ia.md`, section 11, "Panneau
    /// IA"): ↩ or `y` accepts, Escape or `n` refuses, a digit chooses a
    /// subject, Escape stops a run still going.
    static func action(_ phase: AIRun.Phase, key: String, isReturn: Bool, isEscape: Bool) -> AIKeyAction {
        switch phase {
        case .running:
            return isEscape ? .cancel : .hint
        case .failed:
            return isEscape || isReturn ? .close : .hint
        case .proposal(.replace(_, let unchanged)):
            if unchanged { return isEscape || isReturn ? .close : .hint }
            if isReturn || key == "y" { return .accept }
            if isEscape || key == "n" { return .refuse }
            return .hint
        case .proposal(.subjects(let items)):
            if isEscape || key == "n" { return .refuse }
            if let d = Int(key), (1...items.count).contains(d) { return .choose(d - 1) }
            return .hint
        }
    }

    /// The keys the panel answers to, under its title.
    static func hint(_ phase: AIRun.Phase) -> String {
        switch phase {
        case .running: L("esc stop")
        case .failed: L("esc close")
        case .proposal(.replace(_, true)): L("No change to make · esc close")
        case .proposal(.replace): L("↩ or y accept · esc or n refuse · ⌘Z after accepting undoes")
        case .proposal(.subjects(let items)): L("1–%lld choose · esc or n refuse", items.count)
        }
    }

    /// A replacement as text, the `wdiff` style of the CLI: `[-removed-]`,
    /// `{+added+}`; the segments one per line block. For the UI scripts,
    /// which read the panel as text.
    static func diffText(_ segments: [AISegmentChange]) -> String {
        segments.map { s in
            s.diff.map { p in
                switch p.kind {
                case "removed": "[-\(p.text)-]"
                case "added": "{+\(p.text)+}"
                default: p.text
                }
            }.joined()
        }.joined(separator: "\n---\n")
    }

    /// Where the cursor goes when the AI's text replaces `old`: before
    /// what changed, it stays; after it, it keeps its distance to the end;
    /// inside it, it goes to the end of the new words (the user was typing
    /// there). Positions in `Character`s, those of `TextBuffer`.
    static func cursorAfter(old: String, new: String, cursor: Int) -> Int {
        let a = Array(old), b = Array(new)
        var prefix = 0
        while prefix < a.count, prefix < b.count, a[prefix] == b[prefix] { prefix += 1 }
        var suffix = 0
        while suffix < a.count - prefix, suffix < b.count - prefix, a[a.count - 1 - suffix] == b[b.count - 1 - suffix] {
            suffix += 1
        }
        let c = min(max(0, cursor), a.count)
        // The end first: a cursor right where words were added (just after
        // "merci", which became "merci.") goes after them.
        if c >= a.count - suffix { return b.count - (a.count - c) }
        if c <= prefix { return c }
        return b.count - suffix
    }

    /// The title of the panel: the prompt, and its variant.
    static func title(_ p: AIPromptView, variant: AIVariantView?) -> String {
        guard let v = variant else { return p.label }
        return "\(p.label) · \(v.label)"
    }

    /// The app's language as the core wants it (`fr`): the language of the
    /// answers that do not follow the text's (`ia.md`, section 7).
    static var language: String {
        appLocale.language.languageCode?.identifier ?? "en"
    }

    /// The status bar for an error of `ai_run` or `ai_apply`.
    static func errorFlash(_ error: Error) -> String {
        guard let e = error as? CoreClient.RPCError else { return error.localizedDescription }
        switch e.code {
        case AIError.disabled: return L("The AI is off: set ai.enabled = true in config.toml")
        case ComposeError.revisionConflict: return L("The reply changed since the AI read it: run it again")
        default: return L("AI: %@", e.message)
        }
    }
}
