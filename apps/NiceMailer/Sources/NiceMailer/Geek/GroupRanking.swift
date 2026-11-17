import Foundation

/// Which contact groups the folder column shows, and in which order the
/// Group… panel of the attribute selector lists them (`ui.md`, "Groupes
/// dans la colonne des dossiers").
///
/// The column has room for a few groups under the folders. With more groups
/// than that, it keeps the most relevant ones, by two signals the core gives
/// with each group (`list_groups`):
///
/// - **recency**: the latest change of the group or of its members (a group
///   one has just filled is the one being worked on);
/// - **size**: the mails received from its members (a group one hears from
///   a lot is one that filters a lot).
///
/// Neither signal alone is right: by size only, a new group never shows
/// until it has collected mail; by recency only, a big group touched long
/// ago disappears. So the two rankings are **interleaved**: the most recent
/// group, then the biggest one not taken yet, then the next most recent…
/// A group first on both counts takes one place, and the next one goes to
/// the other ranking. A score mixing a date and a count would need weights
/// with no natural value; the interleaving needs none and is easy to
/// predict.
///
/// Pure functions of their input, tested without a view
/// (`Tests/NiceMailerTests/GroupRankingTests.swift`).
enum GroupRanking {
    /// Groups shown in the folder column before "N more…". Eight rows of
    /// 20 points keep the folders, the part one uses most, in view on a
    /// laptop screen.
    static let columnLimit = 8

    /// Every group, most relevant first (interleaved recency and size). Ties
    /// keep the core's display order (position, then name), so that the
    /// order never depends on how the sort breaks them.
    static func ranked(_ groups: [GroupView]) -> [GroupView] {
        // Swift note: `enumerated()` pairs each element with its index; the
        // index is the tie-breaker of both sorts (display order).
        let indexed = Array(groups.enumerated())
        let byRecency = indexed.sorted { ($0.element.dateModified, -$0.offset) > ($1.element.dateModified, -$1.offset) }
        let bySize = indexed.sorted { ($0.element.mailCount, -$0.offset) > ($1.element.mailCount, -$1.offset) }
        var out: [GroupView] = []
        var taken = Set<Int>()
        var r = 0, s = 0
        // Swift note: a nested function may change the variables of the
        // function around it (`taken`, `out`) and its `inout` index.
        func takeNext(from list: [(offset: Int, element: GroupView)], at i: inout Int) {
            while i < list.count {
                let g = list[i].element
                i += 1
                if taken.insert(g.id).inserted {
                    out.append(g)
                    return
                }
            }
        }
        while out.count < groups.count {
            takeNext(from: byRecency, at: &r)
            if out.count < groups.count { takeNext(from: bySize, at: &s) }
        }
        return out
    }

    /// What the folder column shows: the groups in display order, all of
    /// them when they fit in `limit`; otherwise the groups filtering the
    /// list (`active`, always shown: a filter one cannot see is one that
    /// cannot be removed by a click), then the most relevant ones up to
    /// `limit`. `hidden` is the number left out, for the "N more…" row.
    ///
    /// The rows keep the display order rather than the ranking: a group
    /// does not jump to another row when a mail arrives, only in or out.
    static func shown(_ groups: [GroupView], active: Set<Int>, limit: Int = columnLimit) -> (rows: [GroupView], hidden: Int) {
        guard groups.count > limit else { return (groups, 0) }
        var keep = Set(groups.filter { active.contains($0.id) }.map(\.id))
        for g in ranked(groups) where keep.count < limit {
            keep.insert(g.id)
        }
        let rows = groups.filter { keep.contains($0.id) }
        return (rows, groups.count - rows.count)
    }
}
