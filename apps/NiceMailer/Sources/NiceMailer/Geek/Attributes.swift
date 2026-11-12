import Foundation

/// Attributes narrowing the mail list (`ui.md`, attribute selector, key
/// `F`): the app's mirror of the core's `ThreadFilter`. The core does the
/// filtering; this holds the user's choices, turns them into request
/// parameters and into the chips of the status bar.
///
/// Pure value type, tested without any view
/// (`Tests/NiceMailerTests/AttributesTests.swift`).
struct ThreadFilter: Equatable {
    var seen: Bool?
    var flagged: Bool?
    var attachments: Bool?
    var answered: Bool?
    var conversation: Bool?
    /// From one of these (OR-ed); a value starting with `@` is a domain
    /// (the core matches it with its subdomains).
    var from: [String] = []
    /// From none of these (`ui.md`, "Négation").
    var notFrom: [String] = []
    /// To or Cc one of these.
    var to: [String] = []
    /// To or Cc none of these.
    var notTo: [String] = []
    /// Words anywhere, addresses included: the `/` search, the widest one.
    var contains: String?
    /// Words of the subject or body (Text…); `-word` excludes a word.
    var text: String?
    /// Words of the subject only (Subject…).
    var subject: String?
    /// The two bounds of Conversation date…, the date of the thread's
    /// latest mail, independent of each other so that they make an
    /// interval (`ui.md`, "Deux critères de date"): "after" is the lower
    /// bound (included), "before" the upper one (excluded).
    var after: Int?
    var before: Int?
    /// How each bound is shown ("last 7 days", "after 2026-10-01").
    var afterLabel: String?
    var beforeLabel: String?
    /// The two bounds of Mail date…: a mail of the thread received in the
    /// interval (a thread from 2016 to 2026 is "before 2020"). Heavier for
    /// the core, hence a criterion of its own.
    var mailAfter: Int?
    var mailBefore: Int?
    var mailAfterLabel: String?
    var mailBeforeLabel: String?

    var isEmpty: Bool { self == ThreadFilter() }

    /// The yes / no attributes.
    enum Flag: String, CaseIterable {
        case seen, flagged, attachments, answered, conversation
    }

    func value(_ flag: Flag) -> Bool? {
        switch flag {
        case .seen: seen
        case .flagged: flagged
        case .attachments: attachments
        case .answered: answered
        case .conversation: conversation
        }
    }

    /// Sets `flag` to `value`, or clears it if it already has that value
    /// (choosing an active attribute again removes it). Choosing the
    /// opposite value replaces it ("read" after "unread").
    mutating func toggle(_ flag: Flag, _ value: Bool) {
        let new: Bool? = self.value(flag) == value ? nil : value
        switch flag {
        case .seen: seen = new
        case .flagged: flagged = new
        case .attachments: attachments = new
        case .answered: answered = new
        case .conversation: conversation = new
        }
    }

    /// Name of a yes / no attribute with its value, as in the selector and
    /// the chips.
    static func label(_ flag: Flag, _ value: Bool) -> String {
        switch (flag, value) {
        case (.seen, false): L("unread")
        case (.seen, true): L("read")
        case (.flagged, true): L("flagged")
        case (.flagged, false): L("not flagged")
        case (.attachments, true): L("with attachments")
        case (.attachments, false): L("without attachments")
        case (.answered, true): L("answered")
        case (.answered, false): L("not answered")
        case (.conversation, true): L("conversation")
        case (.conversation, false): L("single mail")
        }
    }

    /// Request parameters (`ThreadFilter` of the core, snake_case keys).
    var parameters: [String: Any] {
        var p: [String: Any] = [:]
        for flag in Flag.allCases {
            if let v = value(flag) { p[flag.rawValue] = v }
        }
        if !from.isEmpty { p["from"] = from }
        if !notFrom.isEmpty { p["not_from"] = notFrom }
        if !to.isEmpty { p["to"] = to }
        if !notTo.isEmpty { p["not_to"] = notTo }
        if let contains, !contains.isEmpty { p["contains"] = contains }
        if let text, !text.isEmpty { p["text"] = text }
        if let subject, !subject.isEmpty { p["subject"] = subject }
        if let after { p["after"] = after }
        if let before { p["before"] = before }
        if let mailAfter { p["mail_after"] = mailAfter }
        if let mailBefore { p["mail_before"] = mailBefore }
        return p
    }

    /// One removable chip of the status bar.
    struct Chip: Identifiable, Equatable {
        let id: String
        let label: String
    }

    var chips: [Chip] {
        var out: [Chip] = []
        for flag in Flag.allCases {
            if let v = value(flag) { out.append(Chip(id: flag.rawValue, label: Self.label(flag, v))) }
        }
        for (i, v) in from.enumerated() { out.append(Chip(id: "from:\(i)", label: L("from: %@", v))) }
        for (i, v) in notFrom.enumerated() { out.append(Chip(id: "not_from:\(i)", label: L("not from: %@", v))) }
        for (i, v) in to.enumerated() { out.append(Chip(id: "to:\(i)", label: L("to: %@", v))) }
        for (i, v) in notTo.enumerated() { out.append(Chip(id: "not_to:\(i)", label: L("not to: %@", v))) }
        // Excluded words keep their dash: "text: renouvellement -facture".
        if let text { out.append(Chip(id: "text", label: L("text: %@", text))) }
        if let subject { out.append(Chip(id: "subject", label: L("subject: %@", subject))) }
        if let contains { out.append(Chip(id: "contains", label: L("anywhere: %@", contains))) }
        // One chip per bound, so that either can be removed alone.
        if after != nil { out.append(Chip(id: "after", label: afterLabel ?? L("date"))) }
        if before != nil { out.append(Chip(id: "before", label: beforeLabel ?? L("date"))) }
        // Mail date…'s chips say so: "mail: last 7 days".
        if mailAfter != nil { out.append(Chip(id: "mail_after", label: L("mail: %@", mailAfterLabel ?? L("date")))) }
        if mailBefore != nil { out.append(Chip(id: "mail_before", label: L("mail: %@", mailBeforeLabel ?? L("date")))) }
        return out
    }

    /// Removes the criterion of a chip.
    mutating func remove(_ chip: String) {
        if let flag = Flag(rawValue: chip) {
            if let v = value(flag) { toggle(flag, v) }
            return
        }
        let parts = chip.split(separator: ":").map(String.init)
        switch parts.first {
        case "from": if let i = parts.last.flatMap(Int.init), from.indices.contains(i) { from.remove(at: i) }
        case "not_from": if let i = parts.last.flatMap(Int.init), notFrom.indices.contains(i) { notFrom.remove(at: i) }
        case "to": if let i = parts.last.flatMap(Int.init), to.indices.contains(i) { to.remove(at: i) }
        case "not_to": if let i = parts.last.flatMap(Int.init), notTo.indices.contains(i) { notTo.remove(at: i) }
        case "contains": contains = nil
        case "text": text = nil
        case "subject": subject = nil
        case "after": after = nil; afterLabel = nil
        case "before": before = nil; beforeLabel = nil
        case "mail_after": mailAfter = nil; mailAfterLabel = nil
        case "mail_before": mailBefore = nil; mailBeforeLabel = nil
        default: break
        }
    }

    /// What was typed in a value panel: the value, and whether it is
    /// excluded (typed with a leading `-`, `ui.md`, "Négation"). A lone
    /// `-` is an empty value.
    static func parse(_ typed: String) -> (value: String, excluded: Bool) {
        let t = typed.trimmingCharacters(in: .whitespaces)
        guard t.hasPrefix("-") else { return (t, false) }
        return (String(t.dropFirst()).trimmingCharacters(in: .whitespaces), true)
    }

    /// Adds a value to an attribute; `excluded` (or a value typed `-x`)
    /// adds it to the values left out.
    ///
    /// - From / To: a value is kept once, and is either wanted or excluded:
    ///   adding it to one side removes it from the other.
    /// - Text / Subject: the words replace those already there, each
    ///   excluded word written `-word`, as the core reads it, and a word is
    ///   not repeated. The panel opens on the current words, selected
    ///   (`LineEdit.selectedAll`): typing replaces them, and an arrow lets
    ///   the user edit them, for instance to add `-word` after the fact
    ///   (`ui.md`, "Négation"). Adding to words the user could not see made
    ///   a second Subject… search silently narrow the first one.
    mutating func add(_ field: AttributeField, _ value: String, excluded: Bool = false) {
        let parsed = Self.parse(value)
        let v = parsed.value
        let out = excluded || parsed.excluded
        guard !v.isEmpty else { return }
        // Swift note: an `inout` parameter lets a nested function change the array
        // it is given, here one side or the other of From / To.
        func put(_ v: String, into wanted: inout [String], _ left: inout [String]) {
            left.removeAll { $0 == v }
            if !wanted.contains(v) { wanted.append(v) }
        }
        func words() -> String {
            var all: [String] = []
            for w in v.split(separator: " ").map(String.init) {
                let word = out && !w.hasPrefix("-") ? "-" + w : w
                if !all.contains(word) { all.append(word) }
            }
            return all.joined(separator: " ")
        }
        switch field {
        case .from: if out { put(v, into: &notFrom, &from) } else { put(v, into: &from, &notFrom) }
        case .to: if out { put(v, into: &notTo, &to) } else { put(v, into: &to, &notTo) }
        case .text: text = words()
        case .subject: subject = words()
        case .date, .mailDate: break
        }
    }

    /// Applies a choice of a date panel (`field`: Conversation date… or
    /// Mail date…): it sets the bound it has and keeps the other one, so
    /// "after 1 January" then "before 1 July" is the first half of the
    /// year. Choosing a bound again replaces it.
    mutating func setDate(_ p: DateChoice.Preset, on field: AttributeField = .date) {
        if field == .mailDate {
            if let a = p.after { mailAfter = a; mailAfterLabel = p.label }
            if let b = p.before { mailBefore = b; mailBeforeLabel = p.label }
        } else {
            if let a = p.after { after = a; afterLabel = p.label }
            if let b = p.before { before = b; beforeLabel = p.label }
        }
    }

    /// Whether `p` would leave no day between the two bounds of `field`:
    /// its lower bound at or past the current upper one, or the reverse.
    /// The panel says so on the row rather than refusing it.
    func dateIsEmpty(with p: DateChoice.Preset, on field: AttributeField = .date) -> Bool {
        let (lo0, hi0) = field == .mailDate ? (mailAfter, mailBefore) : (after, before)
        guard let lo = p.after ?? lo0, let hi = p.before ?? hi0 else { return false }
        return lo >= hi
    }

    /// The current words of Text… or Subject…, which their panel opens on
    /// (`nil` for the other attributes, whose values are added one by one).
    func words(of field: AttributeField) -> String? {
        switch field {
        case .text: text
        case .subject: subject
        case .from, .to, .date, .mailDate: nil
        }
    }

    /// Removes the criterion of Text… or Subject… (its panel emptied).
    mutating func clearWords(of field: AttributeField) {
        switch field {
        case .text: text = nil
        case .subject: subject = nil
        case .from, .to, .date, .mailDate: break
        }
    }

    /// Whether `field` has a value (marked "•" in the selector).
    func isActive(_ field: AttributeField) -> Bool {
        switch field {
        case .from: !from.isEmpty || !notFrom.isEmpty
        case .to: !to.isEmpty || !notTo.isEmpty
        case .text: text != nil
        case .subject: subject != nil
        case .date: after != nil || before != nil
        case .mailDate: mailAfter != nil || mailBefore != nil
        }
    }
}

/// The attributes whose value is chosen in the expanded panel ("…").
/// `date` is Conversation date…, the frequent one, listed first so that
/// `da` reaches it; `mailDate` is Mail date… (`ui.md`, "Deux critères de
/// date").
enum AttributeField: String, CaseIterable {
    case from, to, text, subject, date, mailDate

    /// Name in the selector, with "…" since a value follows.
    var title: String {
        switch self {
        case .from: L("From…")
        case .to: L("To…")
        case .text: L("Text…")
        case .subject: L("Subject…")
        case .date: L("Conversation date…")
        case .mailDate: L("Mail date…")
        }
    }

    /// Hidden words that also find the attribute in the selector, so that
    /// the word one thinks of reaches it even when it is not the label
    /// (`cc`, or "destinataire" in French, for "À…"). One localized string
    /// per attribute, its words separated by spaces: each language has its
    /// own synonyms, and the English one (the key) is the fallback.
    var keywords: [String] {
        let words: String = switch self {
        case .from: L("sender")
        case .to: L("recipient cc")
        case .text: L("text words body")
        case .subject: L("title")
        case .date: L("when day period")
        case .mailDate: L("message")
        }
        return words.split(separator: " ").map(String.init)
    }
}

/// Dates of the panels of Conversation date… and Mail date….
enum DateChoice {
    /// A choice: label, and the bound it sets (one of the two).
    struct Preset: Equatable {
        let label: String
        let after: Int?
        let before: Int?
        /// Shown at the right of the row: how six digits were read, when
        /// they make two dates ("dd mm yy" or "yy mm dd").
        var detail: String? = nil
    }

    /// Presets relative to `now`: today, last 7 days, last 30 days, this
    /// year, older than a year.
    static func presets(now: Date = Date(), calendar: Calendar = .current) -> [Preset] {
        let ts = { (d: Date) in Int(d.timeIntervalSince1970) }
        let today = calendar.startOfDay(for: now)
        let year = calendar.date(from: calendar.dateComponents([.year], from: now)) ?? today
        return [
            Preset(label: L("today"), after: ts(today), before: nil),
            Preset(label: L("last 7 days"), after: ts(today) - 6 * 86_400, before: nil),
            Preset(label: L("last 30 days"), after: ts(today) - 29 * 86_400, before: nil),
            Preset(label: L("this year"), after: ts(year), before: nil),
            Preset(label: L("older than a year"), after: nil, before: ts(now) - 365 * 86_400),
        ]
    }

    /// A typed date: `2026-10-01`, `1/10/2026`, `01/10/26` (day first, as
    /// in French). `nil` if it is not one.
    static func parse(_ text: String, calendar: Calendar = .current) -> Date? {
        let t = text.trimmingCharacters(in: .whitespaces)
        for format in ["yyyy-MM-dd", "d/M/yyyy", "d/M/yy", "d.M.yyyy"] {
            let f = DateFormatter()
            f.calendar = calendar
            f.timeZone = calendar.timeZone
            f.locale = Locale(identifier: "en_US_POSIX")
            f.dateFormat = format
            f.isLenient = false
            if let d = f.date(from: t), (1970...2100).contains(calendar.component(.year, from: d)) {
                return calendar.startOfDay(for: d)
            }
        }
        return nil
    }

    /// The dates six digits can be (`ui.md`, "Sélecteur d'attributs"):
    /// `ddmmyy` (as in French) or `yymmdd`, in that order, each kept when
    /// it is a real day (`310299` is no `ddmmyy` date: no 31 February).
    /// The two readings giving the same day (`111111`) count once. The
    /// year `yy` is 20yy below 70, 19yy from 70 (no mail before 1970).
    static func sixDigits(_ text: String, calendar: Calendar = .current) -> [(date: Date, reading: String)] {
        let t = text.trimmingCharacters(in: .whitespaces)
        guard t.count == 6, t.allSatisfy(\.isASCII), t.allSatisfy(\.isNumber) else { return [] }
        let n = t.map { Int(String($0))! }
        let pair = { (i: Int) in n[i] * 10 + n[i + 1] }
        let year = { (yy: Int) in yy < 70 ? 2000 + yy : 1900 + yy }
        // A day exists when the calendar gives back the same day, month
        // and year (`date(from:)` alone turns 31 February into 3 March).
        func day(_ d: Int, _ m: Int, _ y: Int) -> Date? {
            guard (1...12).contains(m), (1...31).contains(d) else { return nil }
            let c = DateComponents(year: y, month: m, day: d)
            guard let date = calendar.date(from: c) else { return nil }
            let back = calendar.dateComponents([.year, .month, .day], from: date)
            return back.year == y && back.month == m && back.day == d ? calendar.startOfDay(for: date) : nil
        }
        var out: [(date: Date, reading: String)] = []
        if let d = day(pair(0), pair(2), year(pair(4))) { out.append((d, L("dd mm yy"))) }
        if let d = day(pair(4), pair(2), year(pair(0))), !out.contains(where: { $0.date == d }) {
            out.append((d, L("yy mm dd")))
        }
        return out
    }

    /// Every date the text can be: one for `2026-10-01` or `1/10/2026`,
    /// one or two for six digits.
    static func candidates(_ text: String, calendar: Calendar = .current) -> [(date: Date, reading: String?)] {
        if let d = parse(text, calendar: calendar) { return [(d, nil)] }
        return sixDigits(text, calendar: calendar).map { ($0.date, $0.reading) }
    }

    /// The unit of a typed duration, by its first letter in English or in
    /// French, or its whole word in either language, accents ignored:
    /// `d`/`j` day, `w`/`s` week, `m` month, `y`/`a` year. Both languages
    /// are always understood, whatever the app's language: the letters do
    /// not collide (`s` is no English unit, `d` no French one).
    private static let units: [String: Calendar.Component] = {
        var u: [String: Calendar.Component] = [:]
        for w in ["d", "j", "day", "days", "jour", "jours"] { u[w] = .day }
        for w in ["w", "s", "week", "weeks", "sem", "semaine", "semaines"] { u[w] = .weekOfYear }
        for w in ["m", "month", "months", "mois"] { u[w] = .month }
        for w in ["y", "a", "year", "years", "an", "ans", "annee", "annees"] { u[w] = .year }
        return u
    }()

    /// A typed duration (`ui.md`, "Sélecteur d'attributs"): a number, then a unit
    /// (`10j`, `3 s`, `8w`, `2 years`); no number means 1 (`m` is a
    /// month). `nil` if it is not one. The count is capped at 9999 so that
    /// the date arithmetic cannot overflow; past 1970 it finds every mail
    /// anyway.
    static func duration(_ text: String) -> (count: Int, unit: Calendar.Component)? {
        let t = text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
            .trimmingCharacters(in: .whitespaces)
        let digits = t.prefix(while: { $0.isASCII && $0.isNumber })
        let word = t.dropFirst(digits.count).trimmingCharacters(in: .whitespaces)
        guard let unit = units[word] else { return nil }
        let count = digits.isEmpty ? 1 : Int(digits) ?? 0
        guard (1...9999).contains(count) else { return nil }
        return (count, unit)
    }

    /// "1 day", "3 weeks": a duration as words, for the row labels. Two
    /// keys (one, several) rather than catalog plural variations: `L()`
    /// formats with `String(format:)`, which does not apply them.
    static func amount(_ count: Int, _ unit: Calendar.Component) -> String {
        switch (unit, count == 1) {
        case (.day, true): L("1 day")
        case (.day, false): L("%lld days", count)
        case (.weekOfYear, true): L("1 week")
        case (.weekOfYear, false): L("%lld weeks", count)
        case (.month, true): L("1 month")
        case (.month, false): L("%lld months", count)
        case (_, true): L("1 year")
        case (_, false): L("%lld years", count)
        }
    }

    /// The two rows of a typed duration: newer than it (lower bound) and
    /// older than it (upper bound). Every unit counts back from `now` by
    /// the calendar: `1d` is this time yesterday, `1m` this day last month
    /// (the same rule for all four, so that `7d` and `1w` agree). The date
    /// it gives is the row's detail.
    static func durationRows(_ text: String, now: Date = Date(), calendar: Calendar = .current) -> [Preset] {
        guard let d = duration(text),
              let start = calendar.date(byAdding: d.unit, value: -d.count, to: now) else { return [] }
        let ts = Int(start.timeIntervalSince1970)
        let shown = ISO8601DateFormatter.string(from: start, timeZone: calendar.timeZone, formatOptions: [.withFullDate])
        let span = amount(d.count, d.unit)
        return [
            Preset(label: L("newer than %@", span), after: ts, before: nil, detail: shown),
            Preset(label: L("older than %@", span), after: nil, before: ts, detail: shown),
        ]
    }

    /// "after" and "before" choices for a typed date: two rows per date it
    /// can be, so four when six digits make two dates; each row then says
    /// how the digits were read.
    /// A typed duration gives its two rows instead.
    static func typed(_ text: String, now: Date = Date(), calendar: Calendar = .current) -> [Preset] {
        let durations = durationRows(text, now: now, calendar: calendar)
        if !durations.isEmpty { return durations }
        let all = candidates(text, calendar: calendar)
        return all.flatMap { c -> [Preset] in
            let day = Int(c.date.timeIntervalSince1970)
            let shown = ISO8601DateFormatter.string(from: c.date, timeZone: calendar.timeZone, formatOptions: [.withFullDate])
            let detail = all.count > 1 ? c.reading : nil
            return [
                Preset(label: L("after %@", shown), after: day, before: nil, detail: detail),
                Preset(label: L("before %@", shown), after: nil, before: day, detail: detail),
            ]
        }
    }
}
