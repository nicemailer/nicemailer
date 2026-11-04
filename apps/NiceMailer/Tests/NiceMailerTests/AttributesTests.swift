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
        f.after = 1; f.dateLabel = "last 7 days"
        #expect(f.chips.map(\.id) == ["seen", "from:0", "from:1", "date"])
        f.remove("from:0")
        #expect(f.from == ["bob"])
        f.remove("seen")
        f.remove("date")
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
