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
        f.add(.contains, "devis")
        f.after = 100
        let p = f.parameters
        #expect(p["attachments"] as? Bool == true)
        #expect(p["conversation"] as? Bool == false)
        #expect(p["from"] as? [String] == ["alice"])
        #expect(p["to"] as? [String] == ["bob"])
        #expect(p["contains"] as? String == "devis")
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
