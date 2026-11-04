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
    var from: [String] = []
    var to: [String] = []
    var contains: String?
    var after: Int?
    var before: Int?
    /// How the date criterion is shown ("last 7 days", "after 2026-10-01").
    var dateLabel: String?

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
        if !to.isEmpty { p["to"] = to }
        if let contains, !contains.isEmpty { p["contains"] = contains }
        if let after { p["after"] = after }
        if let before { p["before"] = before }
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
        for (i, v) in to.enumerated() { out.append(Chip(id: "to:\(i)", label: L("to: %@", v))) }
        if let contains { out.append(Chip(id: "contains", label: L("contains: %@", contains))) }
        if after != nil || before != nil { out.append(Chip(id: "date", label: dateLabel ?? L("date"))) }
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
        case "to": if let i = parts.last.flatMap(Int.init), to.indices.contains(i) { to.remove(at: i) }
        case "contains": contains = nil
        case "date": after = nil; before = nil; dateLabel = nil
        default: break
        }
    }

    /// Adds a value to From or To (ignored if already there).
    mutating func add(_ field: AttributeField, _ value: String) {
        let v = value.trimmingCharacters(in: .whitespaces)
        guard !v.isEmpty else { return }
        switch field {
        case .from: if !from.contains(v) { from.append(v) }
        case .to: if !to.contains(v) { to.append(v) }
        case .contains: contains = v
        case .date: break
        }
    }
}

/// The attributes whose value is chosen in the expanded panel ("…").
enum AttributeField: String, CaseIterable {
    case from, to, contains, date

    /// Name in the selector, with "…" since a value follows.
    var title: String {
        switch self {
        case .from: L("From…")
        case .to: L("To…")
        case .contains: L("Contains…")
        case .date: L("Date…")
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
        case .contains: L("text words")
        case .date: L("when day period")
        }
        return words.split(separator: " ").map(String.init)
    }
}

/// Dates of the "Date…" panel.
enum DateChoice {
    /// A choice: label, and the range it sets.
    struct Preset: Equatable {
        let label: String
        let after: Int?
        let before: Int?
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

    /// "after" and "before" choices for a typed date.
    static func typed(_ text: String, calendar: Calendar = .current) -> [Preset] {
        guard let d = parse(text, calendar: calendar) else { return [] }
        let day = Int(d.timeIntervalSince1970)
        let shown = ISO8601DateFormatter.string(from: d, timeZone: calendar.timeZone, formatOptions: [.withFullDate])
        return [
            Preset(label: L("after %@", shown), after: day, before: nil),
            Preset(label: L("before %@", shown), after: nil, before: day),
        ]
    }
}
