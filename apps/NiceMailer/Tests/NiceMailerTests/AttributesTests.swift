import Foundation
import Testing
@testable import NiceMailer

/// Attributes narrowing the list (`ThreadFilter`, `DateChoice`).
@MainActor
struct AttributesTests {
    @Test func choosingAnActiveValueRemovesItAndTheOppositeReplacesIt() {
        var f = ThreadFilter()
        f.toggle(.seen, false)
        #expect(f.seen == false)
        f.toggle(.seen, true)
        #expect(f.seen == true)
        f.toggle(.seen, true)
        #expect(f.seen == nil && f.isEmpty)
    }

    @Test func parametersFollowTheCoreNames() {
        var f = ThreadFilter()
        f.toggle(.attachments, true)
        f.toggle(.conversation, false)
        f.add(.from, " alice ")
        f.add(.from, "alice")
        f.add(.to, "bob")
        f.contains = "devis"
        f.add(.text, "acme")
        f.add(.subject, "renouvellement")
        f.add(.from, "-@acme.fr")
        f.add(.to, "@acme.fr", excluded: true)
        f.after = 100
        let p = f.parameters
        #expect(p["attachments"] as? Bool == true)
        #expect(p["conversation"] as? Bool == false)
        #expect(p["from"] as? [String] == ["alice"])
        #expect(p["to"] as? [String] == ["bob"])
        #expect(p["contains"] as? String == "devis")
        #expect(p["text"] as? String == "acme" && p["subject"] as? String == "renouvellement")
        #expect(p["not_from"] as? [String] == ["@acme.fr"] && p["not_to"] as? [String] == ["@acme.fr"])
        #expect(p["after"] as? Int == 100)
        #expect(p["seen"] == nil && p["before"] == nil)
    }

    @Test func chipsListEveryCriterionAndRemoveIt() {
        var f = ThreadFilter()
        f.toggle(.seen, false)
        f.add(.from, "alice")
        f.add(.from, "bob")
        f.after = 1; f.afterLabel = "last 7 days"
        f.before = 9; f.beforeLabel = "before 2026-10-01"
        #expect(f.chips.map(\.id) == ["seen", "from:0", "from:1", "after", "before"])
        f.remove("from:0")
        #expect(f.from == ["bob"])
        f.remove("seen")
        // Each bound goes alone.
        f.remove("after")
        #expect(f.after == nil && f.afterLabel == nil && f.before == 9)
        f.remove("before")
        f.remove("from:0")
        #expect(f.isEmpty)
    }

    @Test func aValueTypedWithADashIsExcluded() {
        #expect(ThreadFilter.parse(" -@acme.fr ") == ("@acme.fr", true))
        #expect(ThreadFilter.parse("e-mail") == ("e-mail", false))
        #expect(ThreadFilter.parse("-").value.isEmpty)
        var f = ThreadFilter()
        f.add(.from, "-")
        #expect(f.isEmpty)
        f.add(.from, "-bob")
        #expect(f.notFrom == ["bob"] && f.from.isEmpty)
        // A value is wanted or excluded, not both: the last choice wins.
        f.add(.from, "bob")
        #expect(f.from == ["bob"] && f.notFrom.isEmpty)
        f.add(.from, "bob", excluded: true)
        #expect(f.notFrom == ["bob"] && f.from.isEmpty)
        #expect(f.isActive(.from) && !f.isActive(.to))
    }

    @Test func textAndSubjectWordsAddUpWithTheirExclusions() {
        var f = ThreadFilter()
        f.add(.text, "acme.fr")
        f.add(.text, "facture", excluded: true)
        #expect(f.text == "acme.fr -facture")
        // Already there: not repeated; an excluded word typed with its dash.
        f.add(.text, "acme.fr -panne")
        #expect(f.text == "acme.fr -facture -panne")
        // Excluding several words excludes each of them.
        f.add(.subject, "a b", excluded: true)
        #expect(f.subject == "-a -b")
        #expect(f.isActive(.text) && f.isActive(.subject))
    }

    @Test func everyNewCriterionHasItsChip() {
        var f = ThreadFilter()
        f.add(.from, "-@acme.fr")
        f.add(.to, "-@acme.fr")
        f.add(.text, "acme")
        f.add(.subject, "x")
        f.contains = "y"
        #expect(f.chips.map(\.id) == ["not_from:0", "not_to:0", "text", "subject", "contains"])
        #expect(f.chips[0].label == L("not from: %@", "@acme.fr"))
        #expect(f.chips[4].label == L("anywhere: %@", "y"))
        for id in ["not_from:0", "not_to:0", "text", "subject", "contains"] { f.remove(id) }
        #expect(f.isEmpty)
    }

    @Test func typedDatesAreDayFirstOrISO() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        let iso = DateChoice.parse("2026-10-01", calendar: cal)
        let fr = DateChoice.parse("1/10/2026", calendar: cal)
        #expect(iso != nil && iso == fr)
        #expect(DateChoice.parse("01/10/26", calendar: cal) == iso)
        #expect(DateChoice.parse("hier", calendar: cal) == nil)
        #expect(DateChoice.parse("31/02/2026", calendar: cal) == nil)
        let choices = DateChoice.typed("2026-10-01", calendar: cal)
        #expect(choices.count == 2)
        #expect(choices[0].after == Int(iso!.timeIntervalSince1970) && choices[0].before == nil)
        #expect(choices[1].before == Int(iso!.timeIntervalSince1970))
    }

    @Test func sixDigitsAreReadDayFirstThenYearFirst() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        let iso = { (d: Date) in ISO8601DateFormatter.string(from: d, timeZone: cal.timeZone, formatOptions: [.withFullDate]) }
        // Two real days: 1 January 2020 (dd mm yy), 20 January 2001 (yy mm dd).
        #expect(DateChoice.sixDigits("010120", calendar: cal).map { iso($0.date) } == ["2020-01-01", "2001-01-20"])
        #expect(DateChoice.sixDigits("200601", calendar: cal).map { iso($0.date) } == ["2001-06-20", "2020-06-01"])
        // One reading only: no 99th day, no 13th month, no 31 February.
        #expect(DateChoice.sixDigits("311299", calendar: cal).map { iso($0.date) } == ["1999-12-31"])
        #expect(DateChoice.sixDigits("251301", calendar: cal).isEmpty)
        #expect(DateChoice.sixDigits("310299", calendar: cal).isEmpty)
        #expect(DateChoice.sixDigits("290223", calendar: cal).map { iso($0.date) } == ["2029-02-23"])
        // Both readings the same day: once.
        #expect(DateChoice.sixDigits("111111", calendar: cal).map { iso($0.date) } == ["2011-11-11"])
        // Years: 20yy below 70, 19yy from 70.
        #expect(DateChoice.sixDigits("010170", calendar: cal).first.map { iso($0.date) } == "1970-01-01")
        // Not six ASCII digits.
        #expect(DateChoice.sixDigits("01012", calendar: cal).isEmpty)
        #expect(DateChoice.sixDigits("0101200", calendar: cal).isEmpty)
        #expect(DateChoice.sixDigits("01-120", calendar: cal).isEmpty)
        #expect(DateChoice.sixDigits("０１０１２０", calendar: cal).isEmpty)
    }

    @Test func sixDigitsGiveTwoRowsPerDateAndSayHowTheyWereRead() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        let two = DateChoice.typed("010120", calendar: cal)
        #expect(two.map(\.label) == [L("after %@", "2020-01-01"), L("before %@", "2020-01-01"),
                                      L("after %@", "2001-01-20"), L("before %@", "2001-01-20")])
        #expect(two.map(\.detail) == [L("dd mm yy"), L("dd mm yy"), L("yy mm dd"), L("yy mm dd")])
        #expect(two[0].after != nil && two[0].before == nil && two[1].before == two[0].after)
        // A single date: two rows, no reading to explain.
        let one = DateChoice.typed("311299", calendar: cal)
        #expect(one.map(\.label) == [L("after %@", "1999-12-31"), L("before %@", "1999-12-31")])
        #expect(one.allSatisfy { $0.detail == nil })
        #expect(DateChoice.typed("2026-10-01", calendar: cal).allSatisfy { $0.detail == nil })
    }

    @Test func dateBoundsMakeAnInterval() {
        var f = ThreadFilter()
        let after = DateChoice.Preset(label: "after 1", after: 100, before: nil)
        let before = DateChoice.Preset(label: "before 2", after: nil, before: 200)
        f.setDate(after)
        f.setDate(before)
        // Both kept: the interval [100, 200).
        #expect(f.after == 100 && f.before == 200)
        #expect(f.chips.map(\.label) == ["after 1", "before 2"])
        #expect(f.parameters["after"] as? Int == 100 && f.parameters["before"] as? Int == 200)
        // A bound chosen again replaces the old one, the other stays.
        f.setDate(DateChoice.Preset(label: "after 3", after: 150, before: nil))
        #expect(f.after == 150 && f.afterLabel == "after 3" && f.before == 200)
        // A bound that leaves no day between them is flagged, not refused.
        #expect(f.dateIsEmpty(with: DateChoice.Preset(label: "", after: 200, before: nil)))
        #expect(f.dateIsEmpty(with: DateChoice.Preset(label: "", after: nil, before: 150)))
        #expect(!f.dateIsEmpty(with: DateChoice.Preset(label: "", after: 199, before: nil)))
        // Without the other bound, nothing to compare.
        #expect(!ThreadFilter().dateIsEmpty(with: after))
    }

    @Test func durationsAreANumberAndAUnitInEitherLanguage() {
        let day = Calendar.Component.day, week = Calendar.Component.weekOfYear
        // First letters, English and French.
        #expect(DateChoice.duration("10j")! == (10, day))
        #expect(DateChoice.duration("10d")! == (10, day))
        #expect(DateChoice.duration("3s")! == (3, week))
        #expect(DateChoice.duration("8w")! == (8, week))
        #expect(DateChoice.duration("6m")! == (6, .month))
        #expect(DateChoice.duration("2a")! == (2, .year))
        #expect(DateChoice.duration("2y")! == (2, .year))
        // Spaces, case, whole words, accents; no number means 1.
        #expect(DateChoice.duration(" 3 S ")! == (3, week))
        #expect(DateChoice.duration("2 années")! == (2, .year))
        #expect(DateChoice.duration("4 weeks")! == (4, week))
        #expect(DateChoice.duration("m")! == (1, .month))
        // Not durations: no unit, an unknown unit, zero, too large, dates.
        for t in ["10", "", "10x", "10 jo", "0j", "10000d", "-3d", "2026-10-01", "010120", "j10"] {
            #expect(DateChoice.duration(t) == nil, "\(t)")
        }
    }

    @Test func aDurationGivesNewerAndOlderRowsCountedBackByTheCalendar() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        // 2026-11-04 12:00 UTC.
        let now = Date(timeIntervalSince1970: 1_793_793_600)
        let rows = DateChoice.typed("10j", now: now, calendar: cal)
        #expect(rows.map(\.label) == [L("newer than %@", L("%lld days", 10)), L("older than %@", L("%lld days", 10))])
        #expect(rows[0].after == 1_793_793_600 - 10 * 86_400 && rows[0].before == nil)
        #expect(rows[1].before == rows[0].after && rows[1].after == nil)
        #expect(rows.allSatisfy { $0.detail == "2026-10-25" })
        // 7 days and 1 week agree; a month is the same day last month.
        #expect(DateChoice.typed("7d", now: now, calendar: cal)[0].after == DateChoice.typed("1s", now: now, calendar: cal)[0].after)
        #expect(DateChoice.typed("1m", now: now, calendar: cal)[0].detail == "2026-10-04")
        #expect(DateChoice.typed("1a", now: now, calendar: cal)[0].label == L("newer than %@", L("1 year")))
        #expect(DateChoice.amount(1, .weekOfYear) == L("1 week") && DateChoice.amount(3, .month) == L("%lld months", 3))
    }

    @Test func presetsAreRelativeToNow() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_793_000_000)
        let p = DateChoice.presets(now: now, calendar: cal)
        #expect(p.count == 5)
        let today = Int(cal.startOfDay(for: now).timeIntervalSince1970)
        #expect(p[0].after == today)
        #expect(p[1].after == today - 6 * 86_400)
        #expect(p[4].before == 1_793_000_000 - 365 * 86_400 && p[4].after == nil)
    }
}
