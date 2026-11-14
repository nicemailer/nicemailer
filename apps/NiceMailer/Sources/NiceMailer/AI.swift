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
    /// For a prompt of several conversations, what it takes when none is
    /// marked: `cursor` or `folder`.
    var unmarked: String? = nil
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
    /// A text to read: a translation, the dates of a mail.
    case text(String)
    case summary(String, actions: [AIAction])
    /// The phishing risk (`low`, `medium`, `high`) and its reasons;
    /// `raised` when the core raised the model's verdict.
    case verdict(risk: String, reasons: [String], raised: Bool)
    /// Texts to choose from; `then` is `reply` (start a reply with it).
    case choices([String], then: String)
    /// A target per conversation sorted; the batches that gave none.
    case targets([AISuggestion], failedBatches: Int)

    private enum Keys: String, CodingKey {
        case kind, segments, unchanged, items, text, summary, actions, risk, reasons, raised, then, failedBatches
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        switch try c.decode(String.self, forKey: .kind) {
        case "replace":
            self = .replace(segments: try c.decode([AISegmentChange].self, forKey: .segments),
                            unchanged: try c.decode(Bool.self, forKey: .unchanged))
        case "subjects":
            self = .subjects(try c.decode([String].self, forKey: .items))
        case "text":
            self = .text(try c.decode(String.self, forKey: .text))
        case "summary":
            self = .summary(try c.decode(String.self, forKey: .summary), actions: try c.decode([AIAction].self, forKey: .actions))
        case "verdict":
            self = .verdict(risk: try c.decode(String.self, forKey: .risk), reasons: try c.decode([String].self, forKey: .reasons),
                            raised: try c.decode(Bool.self, forKey: .raised))
        case "choices":
            self = .choices(try c.decode([String].self, forKey: .items), then: try c.decode(String.self, forKey: .then))
        case "targets":
            self = .targets(try c.decode([AISuggestion].self, forKey: .items), failedBatches: try c.decode(Int.self, forKey: .failedBatches))
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unknown proposal \(other)")
        }
    }
}

/// An action item of a summary; `owner` and `due` may be empty.
struct AIAction: Decodable, Equatable {
    let task: String
    let owner: String
    let due: String
}

/// A target suggested for a conversation (`ai_suggestions`).
struct AISuggestion: Decodable, Equatable {
    let runId: Int
    let threadId: Int
    /// A folder role.
    let target: String
    let reason: String
}

/// A sorting of several conversations under way (`a` `s` in Quarantine):
/// it does not take the keys; its progress is in the status bar.
struct AISort: Equatable {
    let runId: Int
    let title: String
    var done = 0
    var total = 0
}

/// `ai_get_run` (only what the app uses).
struct AIRunView: Decodable {
    let state: String
    let proposal: AIProposal?
    let error: String?
}

/// `ai_apply`: the draft as saved, or the text chosen for the app to
/// use (a quick reply).
struct AIApplyResult: Decodable {
    let draft: DraftView?
    let text: String?
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
    /// What it works on.
    let target: Target
    /// The text received so far.
    var text = ""
    var phase: Phase = .running

    enum Target: Equatable {
        /// The reply being written, by its opening (`Composer.opened`):
        /// what comes for another reply is dropped.
        case reply(UUID)
        /// A conversation, and the mail of it under the reader's cursor.
        case mail(thread: Int, message: Int?)
    }

    /// The opening of the reply it works on, if it works on one.
    var composer: UUID? {
        if case .reply(let id) = target { return id }
        return nil
    }

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
        case .proposal(.subjects(let items)), .proposal(.choices(let items, _)):
            if isEscape || key == "n" { return .refuse }
            if let d = Int(key), d >= 1, d <= items.count { return .choose(d - 1) }
            return .hint
        case .proposal(.text), .proposal(.summary), .proposal(.verdict), .proposal(.targets):
            // Only read: nothing to accept.
            return isEscape || isReturn ? .close : .hint
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
        case .proposal(.choices(let items, _)): L("1–%lld reply with it · esc or n refuse", items.count)
        case .proposal(.text), .proposal(.summary), .proposal(.verdict), .proposal(.targets): L("esc close")
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

    /// A proposal that is only read, as text: for the UI scripts, and for
    /// copying. A summary lists its action items under it; a verdict gives
    /// its risk, then its reasons.
    static func resultText(_ p: AIProposal) -> String {
        switch p {
        case .text(let t): return t
        case .summary(let s, let actions):
            return ([s] + actions.map { a in
                var line = "- " + a.task
                if !a.owner.isEmpty { line += " (\(a.owner))" }
                if !a.due.isEmpty { line += " · " + a.due }
                return line
            }).joined(separator: "\n")
        case .verdict(let risk, let reasons, _):
            return ([riskLabel(risk)] + reasons.map { "- " + $0 }).joined(separator: "\n")
        case .choices(let items, _), .subjects(let items):
            return items.enumerated().map { "\($0.offset + 1). \($0.element)" }.joined(separator: "\n")
        case .replace(let segments, _):
            return diffText(segments)
        case .targets(let items, _):
            return items.map { "\($0.threadId) → \($0.target)" }.joined(separator: "\n")
        }
    }

    /// The status bar while a sorting goes on.
    static func sortLine(_ s: AISort) -> String {
        s.total > 0 ? L("%@: %lld of %lld sorted · esc stop", s.title, s.done, s.total) : L("%@: sorting… · esc stop", s.title)
    }

    /// The status bar for the conversation under the cursor, when the AI
    /// suggested a target for it: the target, then why.
    static func suggestionLine(_ s: AISuggestion) -> String {
        L("AI: %@ (↩) · %@", Role.name(s.target), s.reason)
    }

    /// The end of a sorting, in the status bar.
    static func sortedFlash(found: Int, failedBatches: Int) -> String {
        failedBatches == 0
            ? L("AI: %lld suggestions · ↩ on a row moves it there", found)
            : L("AI: %lld suggestions, %lld batches without an answer · ↩ on a row moves it there", found, failedBatches)
    }

    /// The risk of a verdict, in the app's language.
    static func riskLabel(_ risk: String) -> String {
        switch risk {
        case "high": L("Risk: high")
        case "medium": L("Risk: medium")
        default: L("Risk: low")
        }
    }

    /// A quick reply put at the start of a reply: alone above the
    /// quotation of a new reply; an empty line apart from the words of a
    /// reply already started. The cursor right after it.
    static func insertReply(_ text: String, into buffer: TextBuffer) -> TextBuffer {
        var b = buffer
        b.moveToStart()
        b.insert(text)
        if !b.text.dropFirst(text.count).hasPrefix("\n") {
            b.insert("\n\n")
            b.moveLeft()
            b.moveLeft()
        }
        return b
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
