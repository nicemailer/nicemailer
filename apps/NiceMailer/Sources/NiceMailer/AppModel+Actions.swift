import AppKit
import Foundation

// Actions of the geek theme on the threads (`ui.md`, "Les opérations
// classiques sur un mail"). They act on the marked threads, or on the
// thread under the cursor when none is marked.
//
// Swift note: an `extension` adds methods to a type defined elsewhere.
// Stored properties cannot be added this way: they stay in AppModel.swift.

extension AppModel {
    /// Threads an action applies to: the marked ones, else the cursor's.
    var targetThreads: [ThreadSummary] {
        if !marked.isEmpty { return threads.filter { marked.contains($0.id) } }
        return selectedThread.map { [$0] } ?? []
    }

    /// Moves the target threads to the folder with `role` (`d` = archive,
    /// `m` + letter, Delete). Threads for which the move is not allowed from
    /// their folder are skipped and reported.
    func moveTargets(to role: String) {
        guard let client = core else { return }
        let targets = targetThreads
        guard !targets.isEmpty else { return }
        var allowed: [ThreadSummary] = []
        for t in targets {
            let from = folders.first { $0.id == t.folderId }
            if from?.moveTargets.contains(role) == true { allowed.append(t) }
        }
        guard !allowed.isEmpty else {
            flash = L("Not a target from this folder: %@", Role.name(role))
            return
        }
        // Optimistic update: the threads leave a folder list at once, and
        // the cursor goes to the next remaining thread (Helix and Vim keep
        // the cursor's position rather than jumping to the top).
        let ids = Set(allowed.map(\.id))
        if listFilter == nil {
            let index = threads.firstIndex { $0.id == selectedThreadId } ?? 0
            threads.removeAll { ids.contains($0.id) }
            marked.subtract(ids)
            selectedThreadId = threads.isEmpty ? nil : threads[min(index, threads.count - 1)].id
        } else {
            marked.subtract(ids)
        }
        // Name the mail, not only the folder: after the move the cursor is
        // already on another mail, so "Moved to Archive" alone does not say
        // which one left.
        flash = allowed.count == 1
            ? L("“%@” moved to %@", Format.flashSubject(allowed[0].subject), Role.name(role))
            : L("%lld threads moved to %@", allowed.count, Role.name(role))
        Task {
            for t in allowed {
                do {
                    let _: CommandResult = try await client.call(
                        "move_thread", ["thread_id": t.id, "target": role, "from_folder_id": t.folderId])
                } catch {
                    report(error)
                }
            }
            if listFilter != nil { await reloadThreads(keepDepth: true) }
        }
    }

    /// Read / unread for the target threads: all become read if one of them
    /// is unread, else all become unread.
    func toggleSeenTargets() {
        guard let client = core else { return }
        let targets = targetThreads
        let seen = targets.contains { $0.unreadCount > 0 }
        Task {
            for t in targets {
                do {
                    let _: CommandResult = try await client.call("set_thread_seen", ["thread_id": t.id, "seen": seen])
                } catch { report(error) }
            }
        }
    }

    /// Flag for the target threads, by the same rule.
    func toggleFlagTargets() {
        guard let client = core else { return }
        let targets = targetThreads
        let flagged = targets.contains { !$0.isFlagged }
        Task {
            for t in targets {
                do {
                    let _: CommandResult = try await client.call(
                        "set_thread_flagged", ["thread_id": t.id, "flagged": flagged])
                } catch { report(error) }
            }
        }
    }

    /// Adds the tag `name` to the target threads, or removes it if every
    /// target already carries it (the same key toggles).
    func tagTargets(_ name: String) {
        guard let client = core else { return }
        let targets = targetThreads
        guard !targets.isEmpty, !name.isEmpty else { return }
        let remove = targets.allSatisfy { $0.tags.contains(name) }
        flash = remove ? L("Tag removed: %@", name) : L("Tagged: %@", name)
        Task {
            for t in targets {
                do {
                    let _: CommandResult = try await client.call(
                        "tag_thread", ["thread_id": t.id, "name": name, "remove": remove])
                } catch { report(error) }
            }
        }
    }

    /// Adds the sender of each target thread to the group `name`.
    func addSendersToGroup(_ name: String) {
        guard let client = core else { return }
        let targets = targetThreads
        guard !targets.isEmpty, !name.isEmpty else { return }
        Task {
            var added: [String] = []
            for t in targets {
                do {
                    let r: GroupMemberResult = try await client.call(
                        "set_group_member", ["group": name, "thread_id": t.id])
                    if r.changed { added.append(r.addr) }
                } catch { report(error) }
            }
            flash = added.isEmpty
                ? L("Already in %@", name)
                : L("Added to %@: %@", name, added.joined(separator: ", "))
        }
    }

    /// The message an export or a display choice applies to: the latest
    /// message of the open thread.
    var latestMessage: MessageView? { openThread?.messages.last }

    /// The latest received message (not in Sent or Drafts): its sender
    /// decides the display of the thread (text or HTML).
    var latestReceived: MessageView? {
        openThread?.messages.last { $0.folderRole != "sent" && $0.folderRole != "drafts" } ?? latestMessage
    }

    /// Display of the open thread: the choice remembered for the sender of
    /// its latest received message, text by default in the geek theme.
    var bodyFormat: String {
        guard let m = latestReceived, m.bodyHtml != nil else { return "text" }
        return m.fromBodyFormat ?? "text"
    }

    /// Switches the open thread between text and HTML, and remembers the
    /// choice for the sender (`vh`, or the button of the reading pane).
    func toggleBodyFormat() {
        guard let client = core, let m = latestReceived, let addr = m.fromAddr else { return }
        guard m.bodyHtml != nil else {
            flash = L("This mail has no HTML version")
            return
        }
        let next = bodyFormat == "html" ? "text" : "html"
        Task {
            do {
                let _: CommandResult = try await client.call("set_body_format", ["addr": addr, "format": next])
                await reloadOpenThread()
                flash = L("%@ shown as %@", addr, next)
            } catch { report(error) }
        }
    }

    /// Saves the latest message of the open thread (`se`, `st`, `sm`) to
    /// `exportDirectory`, by default ~/Downloads (chosen by the core), and
    /// shows the path in the status bar.
    func export(_ format: String) {
        guard let client = core, let m = latestMessage else { return }
        var params: [String: Any] = ["message_id": m.id, "format": format]
        if let dir = exportDirectory { params["dir"] = dir }
        Task {
            do {
                let r: ExportResult = try await client.call("export_message", params)
                flash = L("Saved: %@", r.path)
            } catch { report(error) }
        }
    }

    /// Links of the open thread's text, in order, without duplicates (`l`).
    var links: [URL] {
        guard let thread = openThread else { return [] }
        let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue)
        var seen = Set<URL>()
        var out: [URL] = []
        for m in thread.messages {
            let text = m.bodyText ?? ""
            let range = NSRange(text.startIndex..., in: text)
            for match in detector?.matches(in: text, range: range) ?? [] {
                if let url = match.url, url.scheme?.hasPrefix("http") == true || url.scheme == "mailto",
                   seen.insert(url).inserted {
                    out.append(url)
                }
            }
        }
        return out
    }

    /// Marks the cursor's thread, then moves the cursor and marks the next
    /// one too (J / K): marked threads build a selection for grouped actions.
    func markAndMove(_ delta: Int) {
        guard let cursor = selectedThreadId else { return }
        let result = Selection.markAndMove(delta, in: threads.map(\.id), cursor: cursor, marked: marked)
        marked = result.marked
        selectedThreadId = result.cursor
    }
}

/// Selection rules of the mail list, as pure functions of the list, the
/// cursor and the marks, so they can be tested without any view
/// (`Tests/NiceMailerTests/SelectionTests.swift`).
enum Selection {
    struct Result: Equatable {
        let cursor: Int
        let marked: Set<Int>
    }

    /// A click on `id`, Finder-style: plain click selects and unmarks
    /// everything; ⌘-click marks or unmarks `id` (the current mail joins the
    /// selection first); ⇧-click marks every mail from the cursor to `id`.
    static func click(on id: Int, in ids: [Int], cursor: Int?, marked: Set<Int>, command: Bool, shift: Bool) -> Result {
        if shift, let anchor = cursor, let a = ids.firstIndex(of: anchor), let b = ids.firstIndex(of: id) {
            return Result(cursor: id, marked: marked.union(ids[min(a, b)...max(a, b)]))
        }
        if command {
            var m = marked
            if m.isEmpty, let c = cursor, c != id { m.insert(c) }
            if m.contains(id) { m.remove(id) } else { m.insert(id) }
            return Result(cursor: id, marked: m)
        }
        return Result(cursor: id, marked: [])
    }

    /// J / K: marks the cursor's mail, moves by `delta` (staying in the
    /// list) and marks the mail reached.
    static func markAndMove(_ delta: Int, in ids: [Int], cursor: Int, marked: Set<Int>) -> Result {
        guard let i = ids.firstIndex(of: cursor) else { return Result(cursor: cursor, marked: marked) }
        let next = ids[max(0, min(ids.count - 1, i + delta))]
        return Result(cursor: next, marked: marked.union([cursor, next]))
    }
}
