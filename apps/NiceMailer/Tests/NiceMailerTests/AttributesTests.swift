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

    /// Group… (`ui.md`, "Sélecteur d'attributs"): several groups, sent by
    /// id; a group is wanted or excluded, never both; its chip names it and
    /// removes it by its id.
    @Test func groupsAreChosenByTheirIdAndExcludedLikeFrom() {
        let vip = ThreadFilter.GroupRef(id: 1, name: "VIP")
        let team = ThreadFilter.GroupRef(id: 7, name: "Équipe")
        var f = ThreadFilter()
        f.toggleGroup(vip)
        f.toggleGroup(team)
        #expect(f.parameters["group_ids"] as? [Int] == [1, 7])
        #expect(f.parameters["not_group_ids"] == nil)
        #expect(f.isActive(.group) && f.groupChoice(1) == true && f.groupChoice(3) == nil)
        #expect(f.chips.map(\.label) == [L("group: %@", "VIP"), L("group: %@", "Équipe")])
        // Excluded: leaves the wanted ones.
        f.toggleGroup(vip, excluded: true)
        #expect(f.parameters["group_ids"] as? [Int] == [7])
        #expect(f.parameters["not_group_ids"] as? [Int] == [1])
        #expect(f.groupChoice(1) == false)
        #expect(f.chips.map(\.id) == ["group:7", "not_group:1"])
        // Chosen again on the same side: removed.
        f.toggleGroup(vip, excluded: true)
        #expect(f.notGroups.isEmpty)
        // A chip removes its own group.
        f.remove("group:7")
        #expect(f.isEmpty && !f.isActive(.group))
        // Typed values do not reach groups (`add` is for From… and the words).
        f.add(.group, "VIP")
        #expect(f.isEmpty && f.words(of: .group) == nil)
    }

    /// Tag… (`ui.md`, "Sélecteur d'attributs"): tags by name, without
    /// case, OR-ed, excluded like a group; the core gets `tags` and
    /// `not_tags`.
    @Test func tagsAreChosenByNameAndExcludedLikeGroups() {
        var f = ThreadFilter()
        f.toggleTag("Urgent")
        f.toggleTag("Équipe")
        #expect(f.parameters["tags"] as? [String] == ["Urgent", "Équipe"])
        #expect(f.parameters["not_tags"] == nil)
        // Without case: "urgent" is the tag already chosen.
        #expect(f.isActive(.tag) && f.tagChoice("urgent") == true && f.tagChoice("Calme") == nil)
        #expect(f.chips.map(\.label) == [L("tag: %@", "Urgent"), L("tag: %@", "Équipe")])
        f.toggleTag("urgent", excluded: true)
        #expect(f.parameters["tags"] as? [String] == ["Équipe"])
        #expect(f.parameters["not_tags"] as? [String] == ["urgent"])
        #expect(f.tagChoice("Urgent") == false)
        #expect(f.chips.map(\.id) == ["tag:Équipe", "not_tag:urgent"])
        // Chosen again on the same side: removed.
        f.toggleTag("URGENT", excluded: true)
        #expect(f.notTags.isEmpty)
        // A chip removes its own tag, even with a ":" in its name.
        f.toggleTag("projet:toit")
        f.remove("tag:projet:toit")
        f.remove("tag:Équipe")
        #expect(f.isEmpty && !f.isActive(.tag))
        // Typed values do not reach tags.
        f.add(.tag, "Urgent")
        #expect(f.isEmpty && f.words(of: .tag) == nil)
    }

    @Test func withoutGroupAndAWantedGroupReplaceEachOther() {
        var f = ThreadFilter()
        let vip = ThreadFilter.GroupRef(id: 1, name: "VIP")
        f.toggleGroup(vip)
        f.toggleGroup(ThreadFilter.GroupRef(id: 2, name: "Club"), excluded: true)
        f.toggleNoGroup()
        // The wanted group goes; the excluded one stays.
        #expect(f.noGroup && f.groups.isEmpty && f.notGroups.map(\.id) == [2])
        #expect(f.parameters["no_group"] as? Bool == true)
        #expect(f.chips.map(\.id) == ["not_group:2", "no_group"])
        f.toggleGroup(vip)
        #expect(!f.noGroup && f.parameters["no_group"] == nil)
        // The same for tags; a chip removes it.
        f.toggleTag("Urgent")
        f.toggleNoTag()
        #expect(f.noTag && f.tags.isEmpty && f.isActive(.tag))
        #expect(f.chips.last?.label == L("without tag"))
        f.remove("no_tag")
        #expect(!f.noTag && !f.isActive(.tag))
    }

    /// Human / robot (`expediteurs.md`, section 8): one yes / no attribute,
    /// sent to the core as `human`, the opposite replacing the other.
    @Test func humanAndRobotAreOneAttribute() {
        var f = ThreadFilter()
        f.toggle(.human, true)
        #expect(f.parameters["human"] as? Bool == true)
        #expect(f.chips.map(\.label) == [L("human")])
        f.toggle(.human, false)
        #expect(f.parameters["human"] as? Bool == false)
        #expect(f.chips.map(\.label) == [L("robot")])
        f.remove("human")
        #expect(f.isEmpty)
        // Listed last, so that the letters reaching the others stay.
        #expect(ThreadFilter.Flag.allCases.last == .human)
        #expect(ThreadFilter.keywords(.human, true) == L("person people").split(separator: " ").map(String.init))
        #expect(ThreadFilter.keywords(.seen, true).isEmpty)
    }

    @Test func theOriginOfAContactSaysWhenTheUserChoseIt() {
        let c = { (kind: String?, forced: Bool?) in
            ContactView(id: 1, addr: "a@b.fr", name: nil, screeningTarget: nil, groups: [], messageCount: 1,
                        senderKind: kind, senderKindForced: forced)
        }
        #expect(c("person", false).originLabel == L("human"))
        #expect(c("robot", true).originLabel == L("robot") + " *")
        #expect(c(nil, nil).originLabel == "")
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

    @Test func mailDateHasItsOwnBoundsChipsAndParameters() {
        var f = ThreadFilter()
        let after = DateChoice.Preset(label: "last 7 days", after: 10, before: nil)
        let before = DateChoice.Preset(label: "before 2020-01-01", after: nil, before: 20)
        f.setDate(after, on: .mailDate)
        f.setDate(before, on: .mailDate)
        #expect(f.mailAfter == 10 && f.mailBefore == 20 && f.after == nil && f.before == nil)
        #expect(f.isActive(.mailDate) && !f.isActive(.date))
        #expect(f.parameters["mail_after"] as? Int == 10 && f.parameters["mail_before"] as? Int == 20)
        #expect(f.parameters["after"] == nil)
        #expect(f.chips.map(\.id) == ["mail_after", "mail_before"])
        #expect(f.chips[0].label == L("mail: %@", "last 7 days"))
        // Each criterion checks its own interval.
        f.setDate(DateChoice.Preset(label: "x", after: 5, before: nil))
        #expect(f.dateIsEmpty(with: DateChoice.Preset(label: "y", after: nil, before: 4), on: .mailDate))
        #expect(f.dateIsEmpty(with: DateChoice.Preset(label: "y", after: 30, before: nil), on: .mailDate))
        #expect(!f.dateIsEmpty(with: DateChoice.Preset(label: "y", after: 30, before: nil), on: .date))
        f.remove("mail_after")
        #expect(f.mailAfter == nil && f.mailAfterLabel == nil && f.mailBefore == 20)
        f.remove("mail_before"); f.remove("after")
        #expect(f.isEmpty)
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

    @Test func textAndSubjectWordsReplaceThePreviousOnes() {
        var f = ThreadFilter()
        f.add(.subject, "devis")
        // A second Subject… search replaces the first one: it used to be
        // added to it, unseen, so that the list was always wrong.
        f.add(.subject, "facture")
        #expect(f.subject == "facture")
        // An excluded word typed with its dash; a word is not repeated.
        f.add(.text, "acme.fr -panne acme.fr")
        #expect(f.text == "acme.fr -panne")
        // Excluding several words excludes each of them.
        f.add(.subject, "a b", excluded: true)
        #expect(f.subject == "-a -b")
        #expect(f.isActive(.text) && f.isActive(.subject))
        // Their panel opens on the current words; they can be removed.
        #expect(f.words(of: .subject) == "-a -b" && f.words(of: .from) == nil)
        f.clearWords(of: .subject)
        #expect(f.subject == nil && !f.isActive(.subject) && f.text != nil)
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
        #expect(DateChoice.typed("1a", now: now, calendar: cal)[0].label == L("newer than %@", L("%lld years", 1)))
        #expect(DateChoice.amount(1, .weekOfYear) == L("%lld weeks", 1) && DateChoice.amount(3, .month) == L("%lld months", 3))
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

    @Test func attributesFollowARenamedOrDeletedLabel() {
        // `r` and `d` on the Tags and Groups screens (`ui.md`, "Écrans
        // Tags et Groupes"): a tag goes by its name, a group by its id.
        var f = ThreadFilter()
        f.toggleTag("urgent")
        f.toggleTag("Calme", excluded: true)
        f.toggleGroup(.init(id: 1, name: "VIP"))
        f.toggleGroup(.init(id: 2, name: "Famille"), excluded: true)
        f.renameTag("URGENT", to: "Très urgent")
        f.renameGroup(2, to: "Proches")
        #expect(f.tags == ["Très urgent"] && f.notTags == ["Calme"])
        #expect(f.notGroups == [.init(id: 2, name: "Proches")])
        #expect(f.chips.map(\.label).contains(L("not group: %@", "Proches")))
        f.renameTag("calme", to: nil)
        f.renameGroup(1, to: nil)
        #expect(f.notTags.isEmpty && f.groups.isEmpty)
        // A name or id the attributes do not hold: nothing changes.
        let before = f
        f.renameTag("Autre", to: "X")
        f.renameGroup(9, to: nil)
        #expect(f == before)
    }
}
