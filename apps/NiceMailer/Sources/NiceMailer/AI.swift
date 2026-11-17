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
    /// Texts to choose from; `then` is `reply` (start a reply with it) or
    /// `replace` (the passage selected in the reply).
    case choices([String], then: String)
    /// A target per conversation sorted; the batches that gave none.
    case targets([AISuggestion], failedBatches: Int)
    /// The appointments of a mail (`dates`): Return saves the one
    /// highlighted as a calendar file; none when the mail names no date.
    case events([AIEvent])
    /// The sender's card (`contact`): Return saves it as a vCard file,
    /// ⌘Return saves it and opens it in Contacts.
    case contact(AIContact)

    private enum Keys: String, CodingKey {
        case kind, segments, unchanged, items, text, summary, actions, risk, reasons, raised, then, failedBatches, card
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
        case "events":
            self = .events(try c.decode([AIEvent].self, forKey: .items))
        case "contact":
            self = .contact(try c.decode(AIContact.self, forKey: .card))
        case "targets":
            self = .targets(try c.decode([AISuggestion].self, forKey: .items), failedBatches: try c.decode(Int.self, forKey: .failedBatches))
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unknown proposal \(other)")
        }
    }
}

/// An appointment of a mail (`ia.md`, section 8, "Dates et rendez-vous"),
/// its dates as the core checked them: `YYYY-MM-DD` for a whole day,
/// `YYYY-MM-DDTHH:MM` for a time, floating (read in the user's zone).
struct AIEvent: Decodable, Equatable {
    let title: String
    let start: String
    /// Of the form of `start`; empty when the mail gives no end.
    let end: String
    let location: String
    /// How the date was read, what to bring; may be empty.
    let details: String
}

/// The contact card of the sender of a mail (`ia.md`, section 8, "Fiche
/// contact"), as the core checked it: every text may be empty, the From
/// address is the first of `emails`.
struct AIContact: Decodable, Equatable {
    let name: String
    let given: String
    let family: String
    let organization: String
    /// The job title.
    let title: String
    let emails: [String]
    let phones: [AIPhone]
    let address: AIAddress
    let urls: [String]
    /// What the model gave that the core left out, not being an address,
    /// a number or a link: shown under the card, never in it.
    var dropped: [String] = []
}

/// A phone number of a card; `kind` is `mobile`, `work`, `home`, `fax`
/// or `other`.
struct AIPhone: Decodable, Equatable {
    let kind: String
    let number: String
}

/// The postal address of a card; every part may be empty.
struct AIAddress: Decodable, Equatable {
    let street: String
    let locality: String
    let postcode: String
    let region: String
    let country: String
}

/// A file the core made for the app to save (`ai_apply`): the calendar
/// file of an appointment, the vCard of a contact. The app writes it, as
/// the core has no access to Downloads.
struct AIFile: Decodable, Equatable {
    let filename: String
    let content: String
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
    /// An appointment chosen: its calendar file.
    var calendar: AIFile? = nil
    /// The sender's card: its vCard file.
    var contact: AIFile? = nil
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
    /// The text of a list proposal (other wordings, subjects, quick
    /// replies) that Return takes, highlighted in the panel. The first by
    /// default: the prompts give the best first, so ⌘I ↩ ↩ takes it, and
    /// ↑ ↓ move to another (`AI.action`, `.move`).
    var choice = 0

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
    /// A text of the list (a subject, a wording, a reply), from 0.
    case choose(Int)
    /// Highlight another text of the list: the index it moves to, kept
    /// within the list.
    case move(Int)
    /// An appointment or a contact card saved as a file, then opened (⌘↩):
    /// the file's app, Calendar or Contacts, imports it.
    case open(Int)
    case refuse
    /// Stop the run still going.
    case cancel
    /// Close a panel that has nothing to apply (a failure, no change).
    case close
    /// Copy the whole text of the result to the clipboard (`c`); the panel
    /// stays open, so the result can still be accepted or chosen.
    case copy
    /// Not an answer: the status bar says which keys are.
    case hint
}

enum AI {
    /// The answer a key gives in the panel (`ia.md`, section 11, "Panneau
    /// IA"): ↩ or `y` accepts, Escape or `n` refuses, Escape stops a run
    /// still going. In a list, `choice` is the text highlighted: ↩ takes
    /// it, ↑ ↓ (`arrow` -1, +1) move it, a digit takes a text directly.
    static func action(_ phase: AIRun.Phase, key: String, isReturn: Bool, isEscape: Bool,
                       arrow: Int = 0, choice: Int = 0, isCommand: Bool = false) -> AIKeyAction {
        // `c` copies the result, whatever its kind (`ia.md`, section 11,
        // "Panneau IA"), before the keys of each kind: none of them uses
        // `c`, and digits or `y` / `n` keep their meaning.
        if key == "c", !isCommand, case .proposal(let p) = phase, copyText(p) != nil { return .copy }
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
        case .proposal(.events(let items)) where items.isEmpty:
            // No date in the mail: only read.
            return isEscape || isReturn ? .close : .hint
        case .proposal(.events(let items)):
            // Escape closes rather than refuses (`ai_dismiss` records no
            // refusal once a file was saved); the rest as every list.
            if isEscape { return .refuse }
            // ⌘↩ as on an attachment: saved, then opened.
            if isReturn && isCommand { return .open(min(max(0, choice), items.count - 1)) }
            if isReturn { return .choose(min(max(0, choice), items.count - 1)) }
            if arrow != 0 { return .move(min(max(0, choice + arrow), items.count - 1)) }
            if let d = Int(key), d >= 1, d <= items.count { return .choose(d - 1) }
            return .hint
        case .proposal(.contact):
            // One card, as one appointment: ↩ saves it, ⌘↩ saves and opens
            // it, Escape closes without saving.
            if isEscape { return .refuse }
            if isReturn && isCommand { return .open(0) }
            if isReturn { return .accept }
            return .hint
        case .proposal(.subjects(let items)), .proposal(.choices(let items, _)):
            // Every list works the same way, as a picker does: the text
            // highlighted (the first, the best, until ↑ ↓ move it) is the
            // one Return takes (⌘I ↩ ↩ on a passage).
            if isEscape || key == "n" { return .refuse }
            if isReturn { return .choose(min(max(0, choice), items.count - 1)) }
            if arrow != 0 { return .move(min(max(0, choice + arrow), items.count - 1)) }
            if let d = Int(key), d >= 1, d <= items.count { return .choose(d - 1) }
            return .hint
        case .proposal(.text), .proposal(.summary), .proposal(.verdict), .proposal(.targets):
            // Only read: nothing to accept.
            return isEscape || isReturn ? .close : .hint
        }
    }

    /// The keys the panel answers to, under its title: those of its kind,
    /// then `c` when there is a text to copy.
    static func hint(_ phase: AIRun.Phase) -> String {
        let base = baseHint(phase)
        guard case .proposal(let p) = phase, copyText(p) != nil else { return base }
        return base + " · " + L("c copy")
    }

    /// The text `c` copies: the result as it reads in the panel, without
    /// numbers to strip by hand. A replacement gives the new text (the
    /// segments rewritten, a blank line apart), not its differences; a list
    /// gives its texts one per line, numbered as shown. `nil` when there is
    /// nothing to copy: no change proposed, no date in the mail, the
    /// targets of a sort (shown on the rows, not in the panel). A pure function, tested alone.
    static func copyText(_ p: AIProposal) -> String? {
        switch p {
        case .replace(_, true), .targets, .events([]): return nil
        case .replace(let segments, false): return segments.map(\.new).joined(separator: "\n\n")
        // The card alone: what was left out is a remark, not part of it.
        case .contact(let c): return contactLines(c).joined(separator: "\n")
        default: return resultText(p)
        }
    }

    private static func baseHint(_ phase: AIRun.Phase) -> String {
        switch phase {
        case .running: L("esc stop")
        case .failed: L("esc close")
        case .proposal(.replace(_, true)): L("No change to make · esc close")
        case .proposal(.replace): L("↩ or y accept · esc or n refuse · ⌘Z after accepting undoes")
        case .proposal(.subjects(let items)): L("↩ or 1–%lld choose · ↑↓ move · esc or n refuse", items.count)
        case .proposal(.choices(let items, "replace")):
            L("↩ or 1–%lld replace the selection · ↑↓ move · esc or n refuse · ⌘Z after choosing undoes", items.count)
        case .proposal(.choices(let items, _)): L("↩ or 1–%lld reply with it · ↑↓ move · esc or n refuse", items.count)
        case .proposal(.events(let items)) where items.isEmpty: L("esc close")
        case .proposal(.events(let items)): L("↩ or 1–%lld save (.ics) · ⌘↩ add to the calendar · ↑↓ move · esc close", items.count)
        case .proposal(.contact): L("↩ save (.vcf) · ⌘↩ add to Contacts · esc close")
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
    /// there). Positions in `Character`s.
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
        case .events(let items):
            if items.isEmpty { return L("No date or appointment in this mail.") }
            return items.enumerated().map { i, e in
                ["\(i + 1). \(eventWhen(e))", e.title, e.location, e.details].filter { !$0.isEmpty }.joined(separator: " · ")
            }.joined(separator: "\n")
        case .contact(let c):
            return (contactLines(c) + [droppedLine(c)].compactMap { $0 }).joined(separator: "\n")
        }
    }

    /// A card as the panel shows it, one line per piece, the empty ones
    /// left out: the name, the job and company, the addresses, the
    /// numbers with their label, the postal address, the links. A pure
    /// function, shared by the panel, the copy (`c`) and the UI scripts.
    static func contactLines(_ c: AIContact) -> [String] {
        var lines: [String] = []
        let name = c.name.isEmpty ? [c.given, c.family].filter { !$0.isEmpty }.joined(separator: " ") : c.name
        if !name.isEmpty { lines.append(name) }
        let job = [c.title, c.organization].filter { !$0.isEmpty }.joined(separator: " · ")
        if !job.isEmpty { lines.append(job) }
        lines += c.emails
        lines += c.phones.map { phoneLabel($0.kind) + " " + $0.number }
        let a = c.address
        // The postcode before the town, as most of Europe writes it; the
        // street may hold several lines, joined so one line stays one.
        let town = [a.postcode, a.locality].filter { !$0.isEmpty }.joined(separator: " ")
        let place = [a.street.replacingOccurrences(of: "\n", with: ", "), town, a.region, a.country].filter { !$0.isEmpty }
        if !place.isEmpty { lines.append(place.joined(separator: ", ")) }
        lines += c.urls
        return lines
    }

    /// What the core left out of a card, in one line ("Left out, not
    /// valid: www.acme.fr, 06 12 34 56 78 (direct)"); `nil` when nothing
    /// was. The user sees why a number of the signature is missing.
    static func droppedLine(_ c: AIContact) -> String? {
        c.dropped.isEmpty ? nil : L("Left out, not valid: %@", c.dropped.joined(separator: ", "))
    }

    /// The label of a kind of phone number, in the user's language.
    static func phoneLabel(_ kind: String) -> String {
        switch kind {
        case "mobile": L("Mobile")
        case "work": L("Work")
        case "home": L("Home")
        case "fax": L("Fax")
        default: L("Phone")
        }
    }

    /// When an appointment is, in the interface's locale (`uiLocale`): "Thursday 25 June
    /// 2020 · 14:00–15:00", "Saturday 28 November 2026 – Monday 30
    /// November 2026". The dates are floating (`AIEvent`): read and written
    /// in one fixed zone (UTC), so that no zone shift, nor a change to
    /// summer time, moves them. A date the app cannot read is shown as the
    /// core gave it.
    static func eventWhen(_ e: AIEvent, locale: Locale = uiLocale) -> String {
        var calendar = Calendar(identifier: .gregorian)
        let utc = TimeZone(identifier: "UTC")!
        calendar.timeZone = utc
        func parse(_ s: String) -> (date: Date, timed: Bool)? {
            let f = DateFormatter()
            f.calendar = calendar
            f.timeZone = utc
            f.locale = Locale(identifier: "en_US_POSIX")
            f.dateFormat = s.contains("T") ? "yyyy-MM-dd'T'HH:mm" : "yyyy-MM-dd"
            return f.date(from: s).map { ($0, s.contains("T")) }
        }
        func formatter(_ template: String) -> DateFormatter {
            let f = DateFormatter()
            f.calendar = calendar
            f.timeZone = utc
            f.locale = locale
            f.setLocalizedDateFormatFromTemplate(template)
            return f
        }
        guard let start = parse(e.start) else { return e.start }
        let day = formatter("EEEEdMMMMy"), time = formatter("jjmm")
        let end = parse(e.end)
        if start.timed {
            var s = day.string(from: start.date) + " · " + time.string(from: start.date)
            if let end, end.timed {
                // Another day (a night shift, a trip): its day too.
                s += calendar.isDate(end.date, inSameDayAs: start.date)
                    ? "–" + time.string(from: end.date)
                    : " – " + day.string(from: end.date) + " · " + time.string(from: end.date)
            }
            return s
        }
        if let end, !end.timed, end.date > start.date {
            return day.string(from: start.date) + " – " + day.string(from: end.date)
        }
        return day.string(from: start.date)
    }

    /// The status bar while a sorting goes on.
    static func sortLine(_ s: AISort) -> String {
        s.total > 0 ? L("%@: %lld of %lld sorted · esc stop", s.title, s.done, s.total) : L("%@: sorting… · esc stop", s.title)
    }

    /// The target the AI suggests, on its row: "→ Newsletter", cut to its
    /// first letters in a narrow list ("→ Newsl…"), where the full name
    /// took the room of the subject. A name of `short` letters or fewer
    /// stays whole ("Inbox", "Junk", "Banned"); the status bar gives the
    /// full name with the reason.
    static func suggestionTag(_ name: String, narrow: Bool, short: Int = 6) -> String {
        guard narrow, name.count > short else { return "→ " + name }
        return "→ " + name.prefix(short - 1) + "…"
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
    static func insertReply(_ text: String, into body: String) -> (body: String, cursor: Int) {
        var out = text
        if !body.hasPrefix("\n") { out += "\n\n" }
        out += body
        return (out, text.utf16.count)
    }

    /// The files of a conversation the AI can read (`AttachmentView.readable`),
    /// in the reader's order, with the mail each is attached to; a file
    /// attached again to a reply (same name, type and size) once, as `sa`
    /// saves it. The AI palette of the mail offers a row per file and per
    /// prompt of an attachment (`ia.md`, section 11, "Contexte").
    static func readableAttachments(_ messages: [MessageView]) -> [(message: Int, file: AttachmentView)] {
        var seen = Set<String>()
        return messages.flatMap { m in m.attachments.map { (message: m.id, file: $0) } }.filter { pair in
            let a = pair.file
            return a.readable == true && seen.insert("\(a.filename ?? "")|\(a.mimeType)|\(a.size)").inserted
        }
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
