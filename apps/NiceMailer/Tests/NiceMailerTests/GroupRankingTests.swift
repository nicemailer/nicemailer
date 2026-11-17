import Foundation
import Testing
@testable import NiceMailer

/// Which groups the folder column shows when it has no room for all of them
/// (`GroupRanking`, `ui.md`, "Groupes dans la colonne des dossiers").
@MainActor
struct GroupRankingTests {
    /// A group with its two signals; the id is also its display position.
    func g(_ id: Int, modified: Int, mails: Int) -> GroupView {
        GroupView(id: id, name: "G\(id)", color: nil, memberCount: 1, dateModified: modified, mailCount: mails)
    }

    @Test func theRankingInterleavesRecencyAndSize() {
        let groups = [
            g(1, modified: 10, mails: 1),     // old, small
            g(2, modified: 500, mails: 3),    // most recent
            g(3, modified: 20, mails: 900),   // biggest
            g(4, modified: 400, mails: 2),    // second most recent
            g(5, modified: 30, mails: 800),   // second biggest
        ]
        #expect(GroupRanking.ranked(groups).map(\.id) == [2, 3, 4, 5, 1])
    }

    @Test func aGroupFirstOnBothCountsTakesOnePlace() {
        let groups = [g(1, modified: 900, mails: 900), g(2, modified: 800, mails: 1), g(3, modified: 1, mails: 800)]
        // 1 by recency, then by size 1 is taken: 3; then 2 by recency.
        #expect(GroupRanking.ranked(groups).map(\.id) == [1, 3, 2])
    }

    @Test func tiesKeepTheDisplayOrder() {
        let groups = (1...4).map { g($0, modified: 100, mails: 0) }
        #expect(GroupRanking.ranked(groups).map(\.id) == [1, 2, 3, 4])
        #expect(GroupRanking.ranked([]).isEmpty)
    }

    @Test func everyGroupIsShownWhenTheyFit() {
        let groups = (1...8).map { g($0, modified: $0, mails: 0) }
        let shown = GroupRanking.shown(groups, active: [])
        #expect(shown.rows.map(\.id) == Array(1...8) && shown.hidden == 0)
    }

    @Test func tooManyGroupsKeepTheMostRelevantInDisplayOrder() {
        // Twelve groups: recency grows with the id, size decreases with it.
        let groups = (1...12).map { g($0, modified: $0, mails: 100 - $0) }
        let shown = GroupRanking.shown(groups, active: [], limit: 4)
        // Recency: 12, 11; size: 1, 2. Shown in display order.
        #expect(shown.rows.map(\.id) == [1, 2, 11, 12])
        #expect(shown.hidden == 8)
    }

    @Test func aGroupFilteringTheListIsAlwaysShown() {
        let groups = (1...12).map { g($0, modified: $0, mails: 100 - $0) }
        // Group 6 is neither recent nor big, but filters the list.
        let shown = GroupRanking.shown(groups, active: [6], limit: 4)
        // It takes one of the four places; the others go to 12 (recency),
        // 1 (size) and 11 (recency).
        #expect(shown.rows.map(\.id) == [1, 6, 11, 12])
        #expect(shown.hidden == 8)
        // More active groups than room: all of them, nothing else.
        let many = GroupRanking.shown(groups, active: [3, 5, 7, 9, 10], limit: 4)
        #expect(many.rows.map(\.id) == [3, 5, 7, 9, 10] && many.hidden == 7)
    }
}
