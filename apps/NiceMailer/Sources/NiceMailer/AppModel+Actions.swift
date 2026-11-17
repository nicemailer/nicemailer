import AppKit
import Contacts
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

    /// Why a move is refused, and what to do instead (`dossiers.md`,
    /// "Sent et Drafts"): a draft only goes to Deleted (Delete). A pure
    /// function, tested alone.
    nonisolated static func refusedMoveFlash(from: String?, to role: String) -> String {
        switch from {
        case "drafts": return L("A draft is not moved: Delete puts it in %@", Role.name("deleted"))
        default: return L("Not a target from this folder: %@", Role.name(role))
        }
    }

    /// Whether a thread moved out of the folder `role` may still belong to
    /// its list afterwards, so that its row must not be removed before the
    /// core answers. Only Sent: its copies in Sent stay when the rest of
    /// the conversation is filed, and only a conversation with nothing but
    /// sent mails leaves (`dossiers.md`, "Cibles proposées"). The app
    /// cannot tell the two apart from a row, so it reads the list again.
    nonisolated static func moveMayKeepRow(from role: String?) -> Bool {
        role == "sent"
    }

    /// Moves the target threads to the folder with `role` (`d` = archive,
    /// `m` + letter, Delete). Threads for which the move is not allowed from
    /// their folder are skipped and reported.
    func moveTargets(to role: String, only: [ThreadSummary]? = nil) {
        guard let client = core else { return }
        let targets = only ?? targetThreads
        guard !targets.isEmpty else { return }
        var allowed: [ThreadSummary] = []
        for t in targets {
            let from = folders.first { $0.id == t.folderId }
            if from?.moveTargets.contains(role) == true { allowed.append(t) }
        }
        guard !allowed.isEmpty else {
            let from = folders.first { $0.id == targets[0].folderId }?.role
            flash = AppModel.refusedMoveFlash(from: from, to: role)
            return
        }
        // Optimistic update: the threads leave a folder list at once, and
        // the cursor goes to the next remaining thread (Helix and Vim keep
        // the cursor's position rather than jumping to the top).
        let ids = Set(allowed.map(\.id))
        let mayStay = allowed.contains { t in
            AppModel.moveMayKeepRow(from: folders.first { $0.id == t.folderId }?.role)
        }
        if listFilter == nil && !mayStay {
            let index = threads.firstIndex { $0.id == selectedThreadId } ?? 0
            threads.removeAll { ids.contains($0.id) }
            marked.subtract(ids)
            selectedThreadId = threads.isEmpty ? nil : threads[min(index, threads.count - 1)].id
        } else {
            marked.subtract(ids)
        }
        // Name the mail, not only the folder: after the move the cursor is
        // already on another mail, so "Moved to Archive" alone does not say
        // which one left. The core's answer may complete the message with
        // the decision on the sender (`Workflow.moveFlash`).
        flash = Workflow.moveFlash(subjects: allowed.map(\.subject), folder: Role.name(role), decision: nil, filed: 0)
        // One request for every thread (`ui.md`, "Actions groupées"): one
        // transaction, one batch of events, one undo token. A thread whose
        // sender is suspected of spoofing is left by the core and asked
        // about; it comes back to the list.
        Task {
            let items: [[String: Any]] = allowed.map { ["thread_id": $0.id, "from_folder_id": $0.folderId] }
            let r: MoveThreadsResult
            do {
                r = try await client.call("move_threads", ["threads": items, "target": role])
            } catch {
                report(error)
                await reloadThreads(keepDepth: true)
                return
            }
            if let s = r.suspects.first, let t = allowed.first(where: { $0.id == s.threadId }), suspectMove == nil {
                suspectMove = SuspectMove(threadId: t.id, folderId: t.folderId, target: role, subject: t.subject,
                                          addr: s.addr, verdict: s.verdict)
            }
            let refused = !r.suspects.isEmpty || !r.refused.isEmpty
            let moved = allowed.filter { r.movedThreads.contains($0.id) }
            let decision = r.decisions.last
            let filed = r.filed
            if let u = r.undo {
                pushUndo(UndoStep(tokens: [u], label: L("move to %@", Role.name(role)), threads: moved.map(\.id)))
            }
            if let s = suspectMove {
                flash = L("Probable spoofing of %@: confirm or cancel", s.addr)
            } else if decision != nil || filed > 0 {
                flash = Workflow.moveFlash(subjects: moved.map(\.subject), folder: Role.name(role), decision: decision, filed: filed)
            }
            if listFilter != nil || refused || mayStay { await reloadThreads(keepDepth: true) }
            // The question is about that mail: the cursor goes back to it.
            if let s = suspectMove, threads.contains(where: { $0.id == s.threadId }) { selectedThreadId = s.threadId }
        }
    }

    /// Read / unread for the target threads (`u`): all become read if one of
    /// them is unread, else all become unread.
    func toggleSeenTargets() {
        let targets = targetThreads
        setSeen(targets.contains { $0.unreadCount > 0 }, of: targets)
    }

    /// Marks the target threads read (`U`), whatever their state: unlike `u`,
    /// pressing it on a read mail does not make it unread, so the key can be
    /// pressed without first looking at the unread dot (`ui.md`).
    func markTargetsSeen() {
        // Threads already read are left out: no request for nothing.
        setSeen(true, of: targetThreads.filter { $0.unreadCount > 0 })
    }

    /// Read or unread for the target threads: shown at once on every row
    /// (optimistic), then **one** request for all of them (`thread_ids`,
    /// `ui.md`, "Actions groupées"). One request per thread made the rows
    /// change one after the other, as each answer and event came back. On
    /// an error the list is read again, which puts back the core's state.
    private func setSeen(_ seen: Bool, of targets: [ThreadSummary]) {
        guard let client = core, !targets.isEmpty else { return }
        let ids = Set(targets.map(\.id))
        // Unread: every mail of the thread, as the core does.
        threads = threads.map { ids.contains($0.id) ? $0.with(unreadCount: seen ? 0 : $0.messageCount, isFlagged: $0.isFlagged) : $0 }
        Task {
            do {
                let _: CommandResult = try await client.call("set_thread_seen", ["thread_ids": targets.map(\.id), "seen": seen])
            } catch {
                report(error)
                await reloadThreads(keepDepth: true)
            }
        }
    }

    /// Flag for the target threads, by the same rule as `u` (all flagged
    /// if one is not), shown at once and sent in one request.
    func toggleFlagTargets() {
        guard let client = core else { return }
        let targets = targetThreads
        guard !targets.isEmpty else { return }
        let flagged = targets.contains { !$0.isFlagged }
        let ids = Set(targets.map(\.id))
        threads = threads.map { ids.contains($0.id) ? $0.with(unreadCount: $0.unreadCount, isFlagged: flagged) : $0 }
        Task {
            do {
                let _: CommandResult = try await client.call(
                    "set_thread_flagged", ["thread_ids": targets.map(\.id), "flagged": flagged])
            } catch {
                report(error)
                await reloadThreads(keepDepth: true)
            }
        }
    }

    /// Adds the tag `name` to the target threads, or removes it from them:
    /// the `t` picker has a row for each action (`LabelChoices`).
    func tagTargets(_ name: String, remove: Bool) {
        guard let client = core else { return }
        // Removing: only the threads that have the tag, so that the core
        // does no work for the others and the message counts them right.
        let targets = remove ? targetThreads.filter { $0.tags.contains(name) } : targetThreads
        guard !targets.isEmpty, !name.isEmpty else { return }
        flash = remove ? L("Tag removed: %@", name) : L("Tagged: %@", name)
        Task {
            do {
                let _: CommandResult = try await client.call(
                    "tag_thread", ["thread_ids": targets.map(\.id), "name": name, "remove": remove])
            } catch { report(error) }
        }
    }

    /// Adds the person each target thread is with to the group `name`, or
    /// removes them from it. The core picks that person (the one the
    /// sender column names) and never adds one of the user's addresses
    /// (`groupes.md`, "Adresses de l'utilisateur"): a thread with nobody
    /// else is refused.
    func setSendersGroup(_ name: String, remove: Bool) {
        guard let client = core else { return }
        let targets = targetThreads
        guard !targets.isEmpty, !name.isEmpty else { return }
        Task {
            var changed: [String] = []
            var refused = false
            for t in targets {
                do {
                    let r: GroupMemberResult = try await client.call(
                        "set_group_member", ["group": name, "thread_id": t.id, "remove": remove])
                    // Two threads with the same person: named once.
                    if r.changed, !changed.contains(r.addr) { changed.append(r.addr) }
                } catch {
                    refused = true
                    report(error)
                }
            }
            let who = changed.joined(separator: ", ")
            if remove {
                flash = changed.isEmpty ? L("Not in %@", name) : L("Removed from %@: %@", name, who)
            } else {
                // "Already in" only when every thread answered so: a
                // refused thread added nobody, which it would misreport.
                flash = !changed.isEmpty ? L("Added to %@: %@", name, who)
                    : refused ? L("Nobody to add to %@", name) : L("Already in %@", name)
            }
            // A new group, or a member more or less (its date and mail
            // count change): the folder column and the Group… panel
            // follow. No event says a membership changed: the header of
            // the open thread, which names its person's groups, is read
            // again here.
            await loadGroups()
            await reloadOpenThread()
        }
    }

    /// `r` on the Tags or Groups screen (`ui.md`, "Écrans Tags et
    /// Groupes"): renames the tag (in every account that has it) or the
    /// group `old` to `new`. The core refuses a name another tag or group
    /// already has; the error is shown and nothing changes. Returns
    /// whether the name changed. `id`: the group's, which the attributes
    /// hold.
    @discardableResult
    func renameLabel(_ kind: LabelKind, _ old: String, id: Int? = nil, to new: String) async -> Bool {
        guard let client = core else { return false }
        let new = new.trimmingCharacters(in: .whitespaces)
        guard !new.isEmpty, new != old else { return false }
        do {
            let r: RenameResult = try await client.call(
                kind == .tag ? "rename_tag" : "rename_group", ["name": old, "new_name": new])
            guard r.changed else { return false }
        } catch {
            report(error)
            return false
        }
        flash = kind == .tag ? L("Tag renamed: %@ → %@", old, new) : L("Group renamed: %@ → %@", old, new)
        await afterLabelChange(kind, id: id, old: old, new: new)
        return true
    }

    /// `d` on the Tags or Groups screen, once confirmed: deletes the tag
    /// (taken off every mail, on the server too) or the group (its mails
    /// do not change).
    func deleteLabel(_ kind: LabelKind, _ name: String, id: Int? = nil) async {
        guard let client = core else { return }
        do {
            switch kind {
            case .tag:
                let _: CommandResult = try await client.call("delete_tag", ["name": name])
                flash = L("Tag deleted: %@", name)
            case .group:
                let _: DeleteGroupResult = try await client.call("delete_group", ["name": name])
                flash = L("Group deleted: %@", name)
            }
        } catch {
            report(error)
            return
        }
        await afterLabelChange(kind, id: id, old: name, new: nil)
    }

    /// After a rename or a deletion: the groups (folder column, Group…),
    /// the attributes and list that name the label, the rows (they show
    /// tags) and the header of the open mail. No event says a name
    /// changed, so they are read again here.
    private func afterLabelChange(_ kind: LabelKind, id: Int?, old: String, new: String?) async {
        if kind == .group { await loadGroups() }
        labelChanged(kind, id: id, old: old, new: new)
        await reloadThreads(keepDepth: true)
        await reloadOpenThread()
    }

    /// The tags and groups of the threads `ids` (`thread_labels`), for the
    /// rows of `t` and `g`; `[]` when the core cannot answer (the picker
    /// then only adds).
    func threadLabels(_ ids: [Int]) async -> [ThreadLabels] {
        guard let client = core, !ids.isEmpty else { return [] }
        do {
            // The core reads 1,000 threads at most (`MAX_LABEL_THREADS`):
            // beyond, the groups of the first ones are offered for removal.
            let r: Items<ThreadLabels> = try await client.call("thread_labels", ["thread_ids": Array(ids.prefix(1000))])
            return r.items
        } catch {
            report(error)
            return []
        }
    }

    /// The message an export or a display choice applies to: the latest
    /// message of the open thread.
    var latestMessage: MessageView? { openThread?.messages.last }

    /// The latest received message (not in Sent or Drafts): its sender
    /// decides the display of the thread (text or HTML).
    /// The message a deep link to the open mail points to (`DeepLink`):
    /// its latest received message, so that the link opens it in its
    /// folder (Inbox, Archive…) rather than in Sent.
    var deepLinkMessageId: Int? { latestReceived?.id }

    var latestReceived: MessageView? {
        openThread?.messages.last { $0.folderRole != "sent" && $0.folderRole != "drafts" } ?? latestMessage
    }

    /// Display of the open thread: "text", "html" or "images" (`ui.md`,
    /// "Images distantes"). The choice remembered for the sender of its
    /// latest received message, text by default in the geek theme. A
    /// remembered "images" only applies when that mail's images load by
    /// themselves (`RemoteImages.automatic`: authenticated, in a folder
    /// that allows them); otherwise the thread is shown as HTML, without
    /// its images, unless the user asked for them (`remoteImagesOnce`).
    var bodyFormat: String {
        guard let m = latestReceived, m.bodyHtml != nil else { return "text" }
        if let once = remoteImagesOnce, once == openThread?.id, imagesOffered { return "images" }
        let format = m.fromBodyFormat ?? "text"
        return format == "images" && !RemoteImages.automatic(m) ? "html" : format
    }

    /// Whether the open thread has remote images that may be loaded: the
    /// "images" switch is shown only then.
    var imagesOffered: Bool {
        openThread?.messages.contains(where: RemoteImages.offered) ?? false
    }

    /// Tracking pixels removed from the mails of the open thread.
    var trackingPixels: Int {
        openThread?.messages.reduce(0) { $0 + $1.trackingPixels } ?? 0
    }

    /// Switches the open thread between text and HTML (`vh`): from HTML or
    /// images to text, from text to HTML.
    func toggleBodyFormat() {
        setBodyFormat(bodyFormat == "text" ? "html" : "text")
    }

    /// Switches the open thread between HTML with and without its remote
    /// images (`vi`).
    func toggleRemoteImages() {
        setBodyFormat(bodyFormat == "images" ? "html" : "images")
    }

    /// Shows the open thread as `format` ("text", "html", "images") and
    /// remembers the choice for the sender of its latest received mail
    /// (`vh`, `vi`, or a click on the switch of the reading pane).
    ///
    /// "images" is remembered even when that mail is not authenticated, but
    /// then it applies to this conversation only (`remoteImagesOnce`): the
    /// next mails of the sender load their images by themselves only if
    /// they are authenticated (`RemoteImages.automatic`).
    func setBodyFormat(_ format: String) {
        guard let client = core, let thread = openThread, let m = latestReceived, let addr = m.fromAddr else { return }
        guard m.bodyHtml != nil else {
            flash = L("This mail has no HTML version")
            return
        }
        if format == "images" {
            guard !RemoteImages.blockedRoles.contains(m.folderRole ?? "") else {
                flash = L("Images are never loaded in Junk, Quarantine or Banned")
                return
            }
            guard imagesOffered else {
                flash = L("This mail has no remote images")
                return
            }
        }
        let once = format == "images" && m.authResult != "pass"
        Task {
            do {
                let _: CommandResult = try await client.call("set_body_format", ["addr": addr, "format": format])
                remoteImagesOnce = once ? thread.id : nil
                await reloadOpenThread()
                flash = once
                    ? L("Images loaded for this conversation only: %@ is not authenticated", addr)
                    : L("%@ shown as %@", addr, format)
            } catch { report(error) }
        }
    }

    /// The target threads (marked, or the one under the cursor) as read by
    /// the core: the open one as it is, the others asked for.
    private func targetThreadViews(client: any CoreCalling) async -> [ThreadView] {
        let open = openThread
        var out: [ThreadView] = []
        for t in targetThreads {
            if let open, open.id == t.id {
                out.append(open)
                continue
            }
            do {
                out.append(try await client.call("get_thread", ["thread_id": t.id]))
            } catch { report(error) }
        }
        return out
    }

    /// `se`, `st`, `sm`: saves one message of each target thread (the
    /// marked ones, or the one under the cursor) to `exportDirectory`, by
    /// default ~/Downloads (chosen by the core): in the conversation shown,
    /// the one the reader is on (`current`, `exportMessage`), elsewhere the
    /// latest. One file: its path in the status bar; several: how many.
    /// `sT`, `sM` (`open`): then opens the files in front, 5 at most
    /// (`maxOpened`).
    func export(_ format: String, open: Bool = false, current: Int? = nil) {
        guard let client = core, !targetThreads.isEmpty else { return }
        let dir = exportDirectory
        let shown = openThread?.id
        Task {
            var paths: [String] = []
            var firstError: Error?
            for thread in await targetThreadViews(client: client) {
                guard let m = Self.exportMessage(thread, current: thread.id == shown ? current : nil) else { continue }
                var params: [String: Any] = ["message_id": m.id, "format": format]
                if let dir { params["dir"] = dir }
                do {
                    let r: ExportResult = try await client.call("export_message", params)
                    paths.append(r.path)
                } catch { firstError = firstError ?? error }
            }
            saved(paths, open: open, firstError: firstError)
        }
    }

    /// `sp`, `sP`, `si`, `sI` (`ui.md`, menu `s`): saves one message of
    /// each target thread as a PDF (`MailPDF`), plain or as evidence with
    /// its technical information (`kind`), to `saveDirectory`, never
    /// overwriting a file. The message is the one the reader is on in the
    /// conversation shown (`current`, `exportMessage`), so that a whole
    /// conversation can be printed mail by mail (`ctrl-k` / `ctrl-j`, then
    /// `sp`); the latest of every other target thread. The core gives the
    /// record of the mail, read from its raw message (`message_record`);
    /// the app lays it out, since only it has the text layout and fonts of
    /// macOS. `open`: then opens the files in front, as `sT` does.
    func exportPDF(_ kind: MailPDF.Kind, open: Bool = false, current: Int? = nil) {
        guard let client = core, !targetThreads.isEmpty else { return }
        let dir = saveDirectory
        let shown = openThread?.id
        Task {
            var paths: [String] = []
            var firstError: Error?
            for thread in await targetThreadViews(client: client) {
                guard let m = Self.exportMessage(thread, current: thread.id == shown ? current : nil) else { continue }
                do {
                    let record: MessageRecord = try await client.call("message_record", ["message_id": m.id])
                    // Laid out on the main actor: TextKit 1 objects belong
                    // to one thread, and a mail takes milliseconds.
                    let data = MailPDF.render(record, kind: kind)
                    let name = MailPDF.fileName(subject: record.subject, received: record.dateReceived)
                    // Written off the main thread: a long mail with many
                    // pages is a few MB.
                    let url = try await Task.detached { try MailPDF.write(data, in: dir, name: name) }.value
                    paths.append(url.path)
                } catch { firstError = firstError ?? error }
            }
            saved(paths, open: open, firstError: firstError)
        }
    }

    /// The message of `thread` an export of the menu `s` takes (`sc`, `se`,
    /// `st`, `sm`, `sp`, `si`): `current`, the one the reader is on, when it
    /// belongs to the thread; else the latest, the one the reader shows
    /// unfolded at the bottom. A whole conversation is thus exported mail by
    /// mail with `ctrl-k` / `ctrl-j`. A pure function, tested alone.
    nonisolated static func exportMessage(_ thread: ThreadView, current: Int?) -> MessageView? {
        if let current, let m = thread.messages.first(where: { $0.id == current }) { return m }
        return thread.messages.last
    }

    /// The messages of `thread` whose attachments `sa` / `sg` save: the one
    /// the reader is on when one was chosen (`current`); otherwise every
    /// message, the files of the whole conversation, as before a message
    /// is chosen there is no single mail the user looks at.
    nonisolated static func attachmentMessages(_ thread: ThreadView, current: Int?) -> [MessageView] {
        if let current, let m = thread.messages.first(where: { $0.id == current }) { return [m] }
        return thread.messages
    }

    /// What the status bar says once exports are written, and the files
    /// opened when asked: one file, its path; several, how many. Opened in
    /// front, each in the app macOS gives its type; not beyond
    /// `maxOpened`, which would bury the screen in windows.
    private func saved(_ paths: [String], open: Bool, firstError: Error?) {
        let opening = open && paths.count <= Self.maxOpened
        if opening {
            for path in paths { openFile(URL(fileURLWithPath: path)) }
        }
        if paths.count == 1 {
            flash = opening ? L("Saved and opened: %@", paths[0]) : L("Saved: %@", paths[0])
        } else if let first = paths.first {
            let place = FileManager.default.displayName(atPath: (first as NSString).deletingLastPathComponent)
            if opening {
                flash = L("%lld mails saved to %@ and opened", paths.count, place)
            } else if open {
                flash = L("%lld mails saved to %@, not opened: more than %lld", paths.count, place, Self.maxOpened)
            } else {
                flash = L("%lld mails saved to %@", paths.count, place)
            }
        } else if let firstError {
            report(firstError)
        }
    }

    /// Files `export(_:open:)` and `exportPDF(_:open:)` open at most: beyond,
    /// they are only saved.
    static let maxOpened = 5

    /// `sc` (`ui.md`, menu `s`): puts the text of one mail of every target
    /// thread (the marked ones, or the one under the cursor) on
    /// `pasteboard`, in the order of the list: in the conversation shown,
    /// the one the reader is on (`current`), elsewhere the latest. The core gives each text
    /// with its paragraphs joined back, one line each (`message_text`):
    /// pasted into a document, a paragraph stays a paragraph, without the
    /// line breaks the sender's mail app put every 72 characters. The
    /// pasteboard is the controller's: the general one, a private one in
    /// tests and UI scripts.
    func copyText(to pasteboard: NSPasteboard, current: Int? = nil) {
        guard let client = core, !targetThreads.isEmpty else { return }
        let shown = openThread?.id
        Task {
            var texts: [String] = []
            var subjects: [String] = []
            var firstError: Error?
            for thread in await targetThreadViews(client: client) {
                guard let m = Self.exportMessage(thread, current: thread.id == shown ? current : nil) else { continue }
                do {
                    let r: MessageTextResult = try await client.call("message_text", ["message_id": m.id])
                    texts.append(r.text)
                    subjects.append(m.subject)
                } catch { firstError = firstError ?? error }
            }
            guard !texts.isEmpty else {
                if let firstError { report(firstError) }
                return
            }
            pasteboard.clearContents()
            pasteboard.setString(Self.clipboardText(texts), forType: .string)
            flash = texts.count == 1
                ? L("“%@” copied to the clipboard", Format.flashSubject(subjects[0]))
                : L("%lld mails copied to the clipboard", texts.count)
        }
    }

    /// The texts of several mails as one: an empty line between two mails,
    /// as between two paragraphs; each starts with its `From:` line, which
    /// tells where the next one begins. A pure function, tested alone.
    nonisolated static func clipboardText(_ texts: [String]) -> String {
        texts.joined(separator: "\n\n")
    }

    /// Attachments of the open thread, message by message (`l`, tiles).
    var attachments: [AttachmentView] { openThread?.messages.flatMap(\.attachments) ?? [] }

    /// The unsubscribe offered for the open thread: that of its latest
    /// received message, outside the spam folders (`UnsubscribeDisplay`).
    var unsubscribeOffer: UnsubscribeView? { UnsubscribeDisplay.offer(latestReceived) }

    /// Unsubscribes from the list of the latest received message (`ui.md`,
    /// "Désabonnement"): the core does it with one click, or gives the page
    /// or address to open, which `openFile` opens (browser, Mail). A failed
    /// one-click attempt falls back to the page.
    func unsubscribe() {
        guard let client = core, let m = latestReceived, unsubscribeOffer != nil else {
            flash = L("No unsubscribe link in this mail")
            return
        }
        let sender = m.fromName ?? m.fromAddr ?? ""
        Task {
            do {
                let r: UnsubscribeResult = try await client.call("unsubscribe", ["message_id": m.id])
                if r.done {
                    flash = L("Unsubscribed from %@", sender)
                    // Shows "unsubscribed on …" instead of the button.
                    await reloadOpenThread()
                    return
                }
                guard let s = r.open, let url = URL(string: s) else { return }
                openFile(url)
                if let error = r.error {
                    flash = L("One-click unsubscribe failed (%@): page opened", error)
                } else {
                    flash = r.method == "mail" ? L("Unsubscribe mail opened") : L("Unsubscribe page opened")
                }
            } catch { report(error) }
        }
    }

    /// Opens a link of a mail with the system: a web page in the browser,
    /// a `mailto:` in the mail app, a `tel:` number with the phone (FaceTime
    /// through the iPhone). With `ui.auto_confirm_calls`, the confirmation
    /// macOS shows before calling is pressed by the app (`CallConfirmation`;
    /// `docs/geek-ui.md`, "Calling a number"), and the status bar says how
    /// it went.
    func openLink(_ url: URL) {
        guard url.scheme?.lowercased() == "tel", autoConfirmCalls else {
            openFile(url)
            return
        }
        Task {
            // Swift note: `[weak self]` is not needed here: the model lives
            // as long as the app, and the task ends within seconds.
            let outcome = await confirmCall(url) { openFile(url) }
            flash = Self.callMessage(outcome)
        }
    }

    /// What the status bar says once a call was confirmed, or could not be.
    nonisolated static func callMessage(_ outcome: CallConfirmation.Outcome) -> String {
        switch outcome {
        case .confirmed: L("Calling…")
        case .notTrusted: L("To confirm calls, allow NiceMailer in System Settings › Privacy & Security › Accessibility")
        case .notFound: L("Call not confirmed: click the green button")
        }
    }

    /// What to do with an attachment.
    enum AttachmentAction {
        /// Copy it to Downloads (a click on a tile, Return in `l`).
        case save
        /// Copy it to Downloads, then open that copy, which comes to the
        /// front (⌘-click on a tile, ⌘-Return in `l`).
        case saveAndOpen
    }

    /// Saves the attachment `id` of the open thread to `exportDirectory`
    /// (~/Downloads by default), never overwriting a file ("devis (2).pdf"),
    /// then opens the saved copy with its default app if `action` says so. The core extracts
    /// the file from the raw message and gives its path
    /// (`architecture.md`, section 5); the app copies or opens it.
    func saveAttachment(_ id: Int, _ action: AttachmentAction) {
        guard let client = core, attachments.contains(where: { $0.id == id }) else { return }
        Task {
            do {
                let dest = try await copyAttachment(id, client: client)
                let place = FileManager.default.displayName(atPath: saveDirectory.path)
                if action == .saveAndOpen {
                    // The saved copy, so that changes made in the app that
                    // opens it are kept.
                    openFile(dest)
                    flash = L("%@ saved to %@ and opened", dest.lastPathComponent, place)
                } else {
                    flash = L("%@ saved to %@", dest.lastPathComponent, place)
                }
            } catch { report(error) }
        }
    }

    /// The bytes of an image embedded in a mail (`cid:`), for the HTML
    /// reader (`InlineImageLoader`): the part the core extracts, read from
    /// its file. Nil when the core cannot give it, or when it is not an
    /// image that can be shown (`InlineImages.showable`).
    func inlineImage(_ id: Int) async -> (data: Data, mimeType: String)? {
        guard let client = core,
              let file: AttachmentFileView = try? await client.call("get_attachment", ["attachment_id": id]),
              InlineImages.showable(mimeType: file.mimeType, size: file.size) else { return nil }
        // Read off the main thread, like the copies of `copyAttachment`.
        let path = file.path
        guard let data = await Task.detached(operation: { try? Data(contentsOf: URL(fileURLWithPath: path)) }).value,
              data.count <= InlineImages.maxBytes else { return nil }
        return (data, file.mimeType)
    }

    /// Where attachments are saved: `exportDirectory`, by default
    /// ~/Downloads.
    var saveDirectory: URL {
        exportDirectory.map { URL(fileURLWithPath: $0) }
            ?? FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0]
    }

    /// Asks the core for the file of attachment `id` and copies it to
    /// `saveDirectory`, under a name no file has yet. Returns the copy.
    private func copyAttachment(_ id: Int, client: any CoreCalling) async throws -> URL {
        let file: AttachmentFileView = try await client.call("get_attachment", ["attachment_id": id])
        let dir = saveDirectory
        // Copied off the main thread: a file can weigh tens of MB.
        // Swift note: `Task.detached` runs the closure on a background
        // thread; `.value` waits for its result.
        return try await Task.detached {
            let fm = FileManager.default
            let dest = AttachmentDisplay.freeURL(in: dir, name: file.filename) { fm.fileExists(atPath: $0.path) }
            try fm.copyItem(at: URL(fileURLWithPath: file.path), to: dest)
            // The core's copy is read-only; the user's is not. The quarantine
            // attribute is kept by the copy.
            try fm.setAttributes([.posixPermissions: 0o644], ofItemAtPath: dest.path)
            return dest
        }.value
    }

    /// `sa` / `sg` (`ui.md`, menu `s`): saves the attachments of the target
    /// threads, the marked ones or the one under the cursor, every message
    /// of each thread, or in the conversation shown only the message the
    /// reader is on when one was chosen (`current`, `attachmentMessages`);
    /// `sg` leaves the graphics out (logos, signature
    /// images, photos: `AttachmentDisplay.isGraphic`). The threads that are
    /// not open are read from the core. Files the core cannot give (no raw
    /// message) are counted, not fatal.
    func saveAttachmentsOfTargets(includingGraphics: Bool, current: Int? = nil) {
        guard let client = core, !targetThreads.isEmpty else { return }
        let shown = openThread?.id
        Task {
            var files: [AttachmentView] = []
            for thread in await targetThreadViews(client: client) {
                let messages = Self.attachmentMessages(thread, current: thread.id == shown ? current : nil)
                files += AttachmentDisplay.toSave(messages.flatMap(\.attachments),
                                                  includingGraphics: includingGraphics)
            }
            guard !files.isEmpty else {
                flash = includingGraphics ? L("No attachment to save") : L("No attachment to save but graphics")
                return
            }
            var saved: [URL] = []
            var failed = 0
            var firstError: Error?
            // One after the other: names are chosen against the files
            // already there ("devis (2).pdf"), so two copies cannot race.
            for f in files {
                do { saved.append(try await copyAttachment(f.id, client: client)) } catch {
                    failed += 1
                    firstError = firstError ?? error
                }
            }
            let place = FileManager.default.displayName(atPath: saveDirectory.path)
            if saved.isEmpty, let firstError {
                report(firstError)
            } else if failed > 0 {
                flash = L("%lld attachments saved to %@, %lld not available", saved.count, place, failed)
            } else if saved.count == 1 {
                flash = L("%@ saved to %@", saved[0].lastPathComponent, place)
            } else {
                flash = L("%lld attachments saved to %@", saved.count, place)
            }
        }
    }

    /// Links of the open thread for `l`, each once (`Link.merge`). The
    /// core finds them (`links.rs`, `phones.rs`). A number that is also in
    /// a sender's card is left to `contactLinks`, which names it.
    var links: [Link] {
        guard let thread = openThread else { return [] }
        let known = Set(contactLinks.filter(\.isPhone).map(\.key))
        return Link.merge(thread.messages.map(\.links)).filter { !known.contains($0.key) }
    }

    /// The verification code of the open thread for `l`: that of its
    /// latest message giving one, since a newer code replaces the older
    /// ones (a site asked twice sends two codes, and only the last works).
    var verificationCode: String? {
        openThread?.messages.last(where: { $0.verificationCode != nil })?.verificationCode
    }

    /// The senders of the open thread, latest first, each once: the
    /// addresses of its received messages (not those in Sent or Drafts,
    /// which are the user's). People in copy are left out: in a
    /// conversation, the one to call back is one who wrote.
    var threadSenders: [String] {
        guard let thread = openThread else { return [] }
        var out: [String] = []
        for m in thread.messages.reversed() where m.folderRole != "sent" && m.folderRole != "drafts" {
            guard let addr = m.fromAddr?.lowercased(), !addr.isEmpty, !out.contains(addr) else { continue }
            out.append(addr)
        }
        return out
    }

    /// The rows the senders' cards add to `l` (`ContactLinks`): their
    /// numbers, then each card. Empty until the cards of this thread are
    /// read.
    var contactLinks: [Link] {
        guard let id = openThread?.id, senderCardsThreadId == id else { return [] }
        return ContactLinks.links(senderCards)
    }

    /// Whether each sender of the open thread is in the address book, by
    /// message id (`SenderBook.statuses`): empty until the cards of this
    /// thread are read, so that no icon claims "not in Contacts" for a book
    /// that was not read.
    var senderBook: [Int: SenderBook] {
        guard let thread = openThread else { return [:] }
        return SenderBook.statuses(thread, cards: senderCardsThreadId == thread.id ? senderCards : nil)
    }

    /// The card of `address` if it is a sender of the open thread found in
    /// the address book.
    func senderCard(_ address: String) -> ContactCard? {
        guard let id = openThread?.id, senderCardsThreadId == id else { return nil }
        return SenderBook.card(of: address, in: senderCards)
    }

    /// Reads the cards again when the address book changes (a card created
    /// in Contacts after `ctrl-c` copied the address): the icons of the
    /// open thread follow without opening it again. macOS posts
    /// `CNContactStoreDidChange` for changes made by any app. Installed
    /// once, when the core answers (`connect`, `attach`).
    ///
    /// Swift note: the closure holds `self` weakly (`[weak self]`), so the
    /// notification centre does not keep the model alive; it runs on the
    /// main queue, which `MainActor.assumeIsolated` tells the compiler.
    func watchAddressBook() {
        guard addressBookObserver == nil else { return }
        addressBookObserver = NotificationCenter.default.addObserver(
            forName: .CNContactStoreDidChange, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                Task { await self.loadSenderCards(asking: false) }
            }
        }
    }

    /// Reads the cards of the open thread's senders. With `asking`, macOS's
    /// question is asked first if it never was (the first `l`); a book that
    /// cannot be read is said once in the status bar.
    func loadSenderCards(asking: Bool) async {
        guard let id = openThread?.id else { return }
        let book = addressBook
        switch book.access {
        case .authorized:
            break
        case .notDetermined:
            guard asking, await book.requestAccess() else { return }
        case .denied:
            if asking { sayAddressBookRefused(L("NiceMailer may not read Contacts: allow it in System Settings, Privacy & Security, Contacts")) }
            return
        case .unavailable:
            if asking { sayAddressBookRefused(L("Contacts are only read by the installed app (make open)")) }
            return
        }
        let senders = threadSenders
        let cards = await book.cards(matching: senders)
        // Another thread opened meanwhile: these cards are not its.
        guard openThread?.id == id else { return }
        senderCards = cards
        senderCardsThreadId = id
    }

    private func sayAddressBookRefused(_ message: String) {
        guard !addressBookRefusalShown else { return }
        addressBookRefusalShown = true
        flash = message
    }

    /// A link of the `l` panel: where it goes, and what the mail calls it.
    /// A phone number is a `tel:` link.
    struct Link: Equatable {
        let url: URL
        /// Text of the link in the HTML, or a phone number as written; nil
        /// when the mail only wrote the address, or put an image in the link.
        var text: String?
        /// Where a tracking link really goes, when the core could read it
        /// from its address (`links.rs`, `target`).
        var target: String? = nil
        /// The other texts of the same link in the thread, each once:
        /// "Voir ma facture (and 2 others)".
        var otherTexts: [String] = []
        /// A number from the address book, as its card writes it
        /// (`ContactLinks`): shown on the right, the text being the name.
        var shownNumber: String? = nil

        var isPhone: Bool { url.scheme == "tel" }
        /// A card of the address book, opened in Contacts.
        var isCard: Bool { url.scheme == "addressbook" }

        /// The links of several messages as one list, each once, in three
        /// groups, each in the order of the mail:
        ///
        /// 1. links with a text ("Voir ma facture"): what the mail offers;
        /// 2. phone numbers;
        /// 3. links without a text: in a newsletter, mostly an image
        ///    leading to a social network or a shop, the least wanted.
        ///
        /// A link first seen without text takes the text a later message
        /// gives it (and its group); a URL Foundation cannot read is left
        /// out.
        static func merge(_ lists: [[LinkView]]) -> [Link] {
            var all: [Link] = []
            var index: [String: Int] = [:]
            for view in lists.joined() {
                guard let url = URL(string: view.url) else { continue }
                let text = view.text?.isEmpty == false ? view.text : nil
                let link = Link(url: url, text: text, target: view.target, otherTexts: view.otherTexts ?? [])
                guard let i = index[link.key] else {
                    index[link.key] = all.count
                    all.append(link)
                    continue
                }
                // The same link in another message (a quoted history, a
                // later issue): its texts join those already known.
                var known = all[i]
                for t in [link.text].compactMap({ $0 }) + link.otherTexts {
                    if known.text == nil {
                        known.text = t
                    } else if t != known.text, !known.otherTexts.contains(t) {
                        known.otherTexts.append(t)
                    }
                }
                all[i] = known
            }
            let phones = all.filter(\.isPhone)
            let others = all.filter { !$0.isPhone }
            return others.filter { $0.text != nil } + phones + others.filter { $0.text == nil }
        }

        /// What makes two links the same: the URL, or for a phone its last
        /// nine digits, as in the core (`links.rs`, `dedup_key`): a quoted
        /// signature gives the same number in national and international
        /// form.
        var key: String {
            // A tracking link is known by where it goes: each place of a
            // newsletter has its own tracking address to the same page.
            guard isPhone else { return target ?? url.absoluteString }
            return "tel:" + String(number.filter(\.isNumber).suffix(9))
        }

        /// The number of a `tel:` link: `+33612345678`, `0612345678`.
        private var number: String {
            String(url.absoluteString.dropFirst("tel:".count)).removingPercentEncoding ?? ""
        }

        /// The address of a `mailto:` link, without the scheme.
        private var mailAddress: String { String(url.absoluteString.dropFirst("mailto:".count)) }

        /// Left half of the row: what the link is. Its text; without one,
        /// the number of a phone, the address of a mail link, the site of a
        /// web link ("facebook.com"), the full address being on the right.
        var label: String {
            if let text { return text }
            if isPhone { return number }
            if url.scheme == "mailto" { return mailAddress }
            // The site the link leads to, not that of its tracker.
            return target.flatMap { URL(string: $0)?.host() } ?? url.host() ?? url.absoluteString
        }

        /// The row's text: the label and, when the mail links the same
        /// place from other texts, how many: "ressent "les angoisses" du
        /// pays (and 2 others)".
        var title: String {
            switch otherTexts.count {
            case 0: return label
            default: return L("%@ (and %lld others)", label, otherTexts.count)
            }
        }

        /// Right half of the row: where the link goes, in full (cut in the
        /// middle when too long), so it can be read before opening it.
        /// Nothing when the left half already says it: a mail address or a
        /// number shown as such.
        var destination: String? {
            if isCard { return L("Contacts") }
            if let shownNumber { return shownNumber }
            if isPhone {
                // "06 12 34 56 78" for tel:0612345678 says it already.
                guard let text else { return nil }
                return text.filter(\.isNumber).hasSuffix(String(number.filter(\.isNumber).suffix(9))) ? nil : number
            }
            if url.scheme == "mailto" { return text == nil ? nil : mailAddress }
            // A tracking link shows the page it leads to, not the tracker.
            return target ?? url.absoluteString
        }

        /// SF Symbol of the row.
        var icon: String { isCard ? "person.crop.circle" : isPhone ? "phone" : "link" }
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
