import Foundation

/// The reload of the conversations that changed, rather than of the whole
/// list (`AppModel.patchThreads`, `evenements.md`, "Rechargement par
/// conversation").
///
/// Why: each mail read under the cursor changes one conversation, and the
/// core says so. Reloading the whole list for it cost, in a list 2,000
/// rows deep, a dozen requests and 1.4 MB to decode, several times a
/// minute while the user goes down the list: the keys lagged now and
/// then. Asking for the conversations that changed costs the same
/// whatever the depth.
enum ListPatch {
    /// Above this many conversations changed at once, the whole list is
    /// reloaded instead: a request with a few hundred ids is still one
    /// request, but a burst that large (a whole folder marked read) changes
    /// most of what is shown anyway.
    static let maxThreads = 200

    /// The sort key of a row, as the core sorts the list (`queries.rs`,
    /// `list_query`): the date of the sort, then the id, both descending.
    static func key(_ t: ThreadSummary, sort: String) -> (Int, Int) {
        (sort == "modified" ? t.dateLastModified : t.dateLastReceived, t.id)
    }

    /// `rows` with the conversations `changed` taken out, and `fresh` (what
    /// the core gives for them now, in the list's scope and criteria) put
    /// at their place in the order of the list. A conversation of
    /// `changed` that `fresh` does not hold has left the list.
    ///
    /// `complete`: the list is loaded to its end. When it is not, a fresh
    /// row whose place is after the last row loaded is left out: it belongs
    /// to the part not loaded yet, which the next page brings, and showing
    /// it at the end would put it before rows that should come first.
    ///
    /// A pure function, tested alone.
    static func patched(_ rows: [ThreadSummary], changed: Set<Int>, fresh: [ThreadSummary],
                        complete: Bool, sort: String) -> [ThreadSummary] {
        var out = rows.filter { !changed.contains($0.id) }
        // The last loaded row's key, taken before anything is added: the
        // limit of what this list holds.
        let limit = out.last.map { key($0, sort: sort) }
        for row in fresh where changed.contains(row.id) {
            let k = key(row, sort: sort)
            if !complete, let limit, k < limit { continue }
            // Swift note: tuples of comparable values compare element by
            // element, like the core's `(date, id) < (?, ?)`.
            let at = out.firstIndex { key($0, sort: sort) < k } ?? out.count
            out.insert(row, at: at)
        }
        return out
    }
}
