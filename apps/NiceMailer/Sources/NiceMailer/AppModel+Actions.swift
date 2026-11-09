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

    /// Why a move is refused, and what to do instead (`dossiers.md`,
    /// "Sent et Drafts"): a draft only goes to Deleted (Delete), a sent
    /// mail stays in Sent. A pure function, tested alone.
    nonisolated static func refusedMoveFlash(from: String?, to role: String) -> String {
        switch from {
        case "drafts": return L("A draft is not moved: Delete puts it in %@", Role.name("deleted"))
        case "sent": return L("A sent mail stays in Sent")
        default: return L("Not a target from this folder: %@", Role.name(role))
        }
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
            let from = folders.first { $0.id == targets[0].folderId }?.role
            flash = AppModel.refusedMoveFlash(from: from, to: role)
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
        // which one left. The core's answer may complete the message with
        // the decision on the sender (`Workflow.moveFlash`).
        flash = Workflow.moveFlash(subjects: allowed.map(\.subject), folder: Role.name(role), decision: nil, filed: 0)
        Task {
            var tokens: [String] = []
            var decision: DecisionView?
            var filed = 0
            var refused = false
            for t in allowed {
                do {
                    let r: WorkflowResult = try await client.call(
                        "move_thread", ["thread_id": t.id, "target": role, "from_folder_id": t.folderId])
                    if let u = r.undo { tokens.append(u) }
                    decision = decision ?? r.decision
                    filed += r.filed
                } catch let e as CoreClient.RPCError where e.code == CoreClient.RPCError.confirmationRequired {
                    // The thread comes back to the list, and the user is
                    // asked; the other threads go on.
                    refused = true
                    if suspectMove == nil {
                        suspectMove = SuspectMove(threadId: t.id, folderId: t.folderId, target: role, subject: t.subject,
                                                  addr: e.data["addr"] ?? "", verdict: e.data["verdict"] ?? "")
                    }
                } catch {
                    report(error)
                }
            }
            let moved = allowed.filter { t in suspectMove?.threadId != t.id }
            if !tokens.isEmpty {
                pushUndo(UndoStep(tokens: tokens, label: L("move to %@", Role.name(role)), threads: moved.map(\.id)))
            }
            if let s = suspectMove {
                flash = L("Probable spoofing of %@: confirm or cancel", s.addr)
            } else if decision != nil || filed > 0 {
                flash = Workflow.moveFlash(subjects: moved.map(\.subject), folder: Role.name(role), decision: decision, filed: filed)
            }
            if listFilter != nil || refused { await reloadThreads(keepDepth: true) }
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

    /// Sends `set_thread_seen` for each thread. Not optimistic, as for the
    /// flag: the list updates when the core's event arrives.
    private func setSeen(_ seen: Bool, of targets: [ThreadSummary]) {
        guard let client = core, !targets.isEmpty else { return }
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

    /// `se`, `st`, `sm`: saves the latest message of each target thread
    /// (the marked ones, or the one under the cursor) to `exportDirectory`,
    /// by default ~/Downloads (chosen by the core). One file: its path in
    /// the status bar; several: how many. `sT`, `sM` (`open`): then opens
    /// the files in front, 5 at most (`maxOpened`).
    func export(_ format: String, open: Bool = false) {
        guard let client = core, !targetThreads.isEmpty else { return }
        let dir = exportDirectory
        Task {
            var paths: [String] = []
            var firstError: Error?
            for thread in await targetThreadViews(client: client) {
                guard let m = thread.messages.last else { continue }
                var params: [String: Any] = ["message_id": m.id, "format": format]
                if let dir { params["dir"] = dir }
                do {
                    let r: ExportResult = try await client.call("export_message", params)
                    paths.append(r.path)
                } catch { firstError = firstError ?? error }
            }
            // Opened in front, each in the app macOS gives its type; not
            // beyond `maxOpened`, which would bury the screen in windows.
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
    }

    /// Files `export(_:open:)` opens at most: beyond, they are only saved.
    static let maxOpened = 5

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
    /// of each thread; `sg` leaves the graphics out (logos, signature
    /// images, photos: `AttachmentDisplay.isGraphic`). The threads that are
    /// not open are read from the core. Files the core cannot give (no raw
    /// message) are counted, not fatal.
    func saveAttachmentsOfTargets(includingGraphics: Bool) {
        guard let client = core, !targetThreads.isEmpty else { return }
        Task {
            var files: [AttachmentView] = []
            for thread in await targetThreadViews(client: client) {
                files += AttachmentDisplay.toSave(thread.messages.flatMap(\.attachments),
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
            case 1: return label + " " + L("(and 1 other)")
            default: return label + " " + L("(and %lld others)", otherTexts.count)
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
