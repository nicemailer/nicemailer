import Foundation
import UniformTypeIdentifiers

// The reply, seen from the model (`specifications/reponse.md`): `r` asks
// the core what to open (`prepare_draft`, which writes nothing, so that the
// screen never waits for the database while an import writes to it), the
// draft is written at the first pause (`create_draft`, `ensureWritten`),
// the text typed is saved after each pause (`update_draft`), ⌘↩ queues it
// (`send_draft`), `z` cancels it during the undo delay (`cancel_send`),
// and the core's events say how it ended (`DraftSent`, `SendFailed`,
// `SendPartial`).

/// Error codes of the core the reply screen tells apart (`nm_api::error_code`).
enum ComposeError {
    static let revisionConflict = 1012
    static let draftSending = 1013
    static let sendStarted = 1014
    static let cannotSend = 1015
    static let bodyPending = 1016
}

extension AppModel {
    /// `r`: a reply to all to the mail under the reader's cursor
    /// (`Compose.replyTarget`), or the reply already started to it; `R` to
    /// its sender alone, `f` a forward of it.
    func startReply(current: Int?, kind: DraftKind = .replyAll) async {
        guard composer == nil else { return }
        guard let client = core, let thread = openThread, let m = Compose.replyTarget(thread, current: current) else {
            flash = kind == .forward ? L("No mail to forward") : L("No mail to answer")
            return
        }
        let header = kind == .forward ? Compose.forwardHeader(m) : Compose.quoteHeader(m)
        let mark = flashCount
        do {
            // A proposal, or the draft already started: nothing is written
            // until the user writes something (`ensureWritten`).
            let r: CreateDraftResult = try await client.call(
                "prepare_draft", ["kind": kind.rawValue, "reply_to": m.id, "quote_header": header])
            if r.draft.state == "sending" {
                // Already on its way: `z` cancels it while it can.
                flash = L("This reply is being sent · z to cancel")
                return
            }
            let opened = await openComposer(r.draft, kind: kind, quoteHeader: header)
            if let w = r.warning {
                sayOpening(w, for: opened, since: mark)
            } else if r.reused {
                sayOpening(r.draft.kind == "forward" ? L("Forward resumed") : L("Reply resumed"), for: opened, since: mark)
            } else {
                // The mail's account cannot send (an archive, no SMTP): the
                // core chose another; the user is told, and how to change it.
                sayOpening(Compose.otherSenderFlash(r.draft, senders: senders), for: opened, since: mark)
            }
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// `c`: a new mail, from the account of the view (in the view of all
    /// accounts, the one of the mail selected; else the core's default).
    func startNewMail() async {
        guard composer == nil, let client = core else { return }
        var params: [String: Any] = ["kind": DraftKind.new.rawValue]
        let folder = threads.first { $0.id == selectedThreadId }?.folderId
        let selected = folders.first { $0.id == folder }?.accountId
        if let account = scopeAccountId ?? selected { params["account"] = account }
        let mark = flashCount
        do {
            let r: CreateDraftResult = try await client.call("prepare_draft", params)
            let opened = await openComposer(r.draft, kind: .new)
            sayOpening(nil, for: opened, since: mark)
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// Opens the reply on a draft, with the addresses it can be sent from
    /// (`list_senders`: the accounts that can send may change with
    /// `accounts.toml`, so they are asked again each time). The reply opens
    /// even if the list does not come: only the account's key and ⌥⌘F
    /// miss it.
    ///
    /// Returns which opening it is (`Composer.opened`): the caller's message
    /// in the status bar is for it only, not for a screen the user already
    /// left while the senders came.
    @discardableResult
    func openComposer(_ draft: DraftView, kind: DraftKind? = nil, quoteHeader: String = "") async -> UUID {
        let c = Composer(draft: draft, kind: kind, quoteHeader: quoteHeader)
        composer = c
        guard let client = core else { return c.opened }
        if let r: ListSendersResult = try? await client.call("list_senders", [:]) {
            senders = r.items
        }
        return c.opened
    }

    /// The message of the reply screen `opened`, once it is open: said only
    /// if the user is still on that screen and nothing newer was said
    /// since `mark` (`flashCount` when the opening began). Escape pressed
    /// while the senders came says "kept" or "nothing written": that must
    /// stay, not be replaced by the opening's message arriving late.
    func sayOpening(_ text: String?, for opened: UUID, since mark: Int) {
        guard composer?.opened == opened, flashCount == mark else { return }
        flash = text
    }

    /// ⌥⌘F, or a click on the line "From": the choice of the address to
    /// send from, when there is one to make.
    var canChooseSender: Bool { senders.count > 1 }

    /// An address of "Send from" chosen: the account that sends, and the
    /// From, saved with the next pause like the other headers. The core
    /// checks it again (`update_draft`).
    func chooseSender(_ s: SenderView) {
        guard composer != nil else { return }
        composer?.headers.from = AddressView(addr: s.addr, name: s.name)
        composer?.headers.sendAccountId = s.accountId
        flash = L("Sending from %@", Compose.senderLabel(s))
        composerChanged()
    }

    /// The text typed changed: saved after `Compose.saveDelay` without
    /// typing.
    func composerChanged() {
        composeSaveTask?.cancel()
        composeSaveTask = Task {
            try? await Task.sleep(for: Compose.saveDelay)
            guard !Task.isCancelled else { return }
            await saveComposer()
        }
    }

    /// Saves the text if it changed. One save at a time, in order: each
    /// sends the revision the previous one got back.
    func saveComposer() async {
        let previous = composeSaving
        let task = Task { @MainActor in
            await previous?.value
            await self.performSave()
        }
        composeSaving = task
        await task.value
    }

    private func performSave() async {
        guard composer?.isDirty == true else { return }
        // A proposal is written first, with the text typed: often all
        // there is to save.
        guard await ensureWritten() else { return }
        guard let client = core, let c = composer, c.isDirty else { return }
        // What is sent: the text and the headers as they are now. Typing
        // may go on during the call; what is typed meanwhile stays dirty
        // and goes with the next save.
        let text = c.buffer.text, headers = c.headers
        do {
            let r: UpdateDraftResult = try await client.call("update_draft", Compose.saveParams(c))
            // The user may have left, or started another reply, meanwhile.
            if composer?.opened == c.opened {
                composer?.revision = r.revision
                composer?.saved = text
                composer?.savedHeaders = headers
            }
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// Writes the draft if it is only proposed (`prepare_draft`): with
    /// `create_draft`, the text typed as its body. True when the draft is
    /// written (now or before), false when it could not be (the flash says
    /// why) or the user left meanwhile.
    ///
    /// Called before anything that needs the draft's id: a save, a file
    /// attached or removed, the sending. One writing at a time: a second
    /// caller waits for the one in progress, so that a new mail, which the
    /// core cannot recognize as "the same", is never written twice.
    func ensureWritten() async -> Bool {
        guard let c = composer else { return false }
        if c.isWritten { return true }
        if let writing = composeWriting { return await writing.value }
        let task = Task { @MainActor in await self.write(c) }
        composeWriting = task
        let ok = await task.value
        composeWriting = nil
        return ok
    }

    private func write(_ c: Composer) async -> Bool {
        guard let client = core else { return false }
        do {
            let r: CreateDraftResult = try await client.call("create_draft", Compose.createParams(c))
            guard composer?.opened == c.opened else { return false }
            composer?.written(r.draft)
            if let w = r.warning { flash = w }
            return true
        } catch {
            flash = Self.composeErrorFlash(error)
            return false
        }
    }

    /// Takes the text typed in the focused recipient line, not yet made
    /// addresses, and empties the line: synchronously, before the focus
    /// moves, so that the text is read for the line it was typed in.
    func takePendingLine() -> (field: ComposeField, text: String)? {
        guard var c = composer, c.focus.isRecipients else { return nil }
        let text = c.line.text.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return nil }
        c.line = LineEdit()
        c.suggestions = []
        c.suggestionIndex = nil
        composer = c
        return (c.focus, text)
    }

    /// Makes addresses of a text typed in a recipient line (Return, `,`,
    /// Tab, leaving the line, sending): the core reads it (`parse_addresses`,
    /// the same rules for every client). What is not an address goes back
    /// to its line, which takes the focus again, for the user to correct.
    /// Returns false when some text could not be read.
    @discardableResult
    func commitRecipients(_ text: String, to field: ComposeField) async -> Bool {
        guard let client = core, let opened = composer?.opened else { return true }
        do {
            let r: ParseAddressesResult = try await client.call("parse_addresses", ["text": text])
            guard composer?.opened == opened else { return true }
            let already = composer?.add(r.addresses, to: field) ?? []
            if !r.invalid.isEmpty {
                giveBack(r.invalid.joined(separator: ", "), to: field)
                flash = L("Not an address: %@", r.invalid.joined(separator: ", "))
            } else if let first = already.first {
                flash = L("Already a recipient: %@", first.addr)
            }
            composerChanged()
            return r.invalid.isEmpty
        } catch {
            giveBack(text, to: field)
            flash = Self.composeErrorFlash(error)
            return false
        }
    }

    /// A text that did not become addresses goes back to its line, before
    /// what was typed there since.
    private func giveBack(_ text: String, to field: ComposeField) {
        guard var c = composer else { return }
        let typed = c.focus == field ? c.line.text : ""
        c.focus = field
        c.line = LineEdit(typed.isEmpty ? text : text + ", " + typed)
        composer = c
    }

    /// The pending text of the focused recipient line made addresses now
    /// (before sending or leaving). False when some of it is not one.
    @discardableResult
    func commitRecipientLine() async -> Bool {
        guard let (field, text) = takePendingLine() else { return true }
        return await commitRecipients(text, to: field)
    }

    /// ⇧⌘A: the open panel, then the files chosen are attached.
    func chooseAttachments() async {
        guard composer != nil else { return }
        let urls = await chooseFiles()
        await attach(urls)
    }

    /// Attaches files (the open panel, a drop, a paste): each is copied
    /// into `incoming.noindex` (a clone on APFS: instant, no space taken),
    /// then taken by the core (`add_draft_attachment`), which reads nothing
    /// outside its data directory. One file at a time, in the order of the
    /// saves, so that each call has the revision the previous one gave.
    func attach(_ urls: [URL]) async {
        let files = urls.filter { $0.isFileURL }
        guard !files.isEmpty, composer != nil else { return }
        let previous = composeSaving
        let task = Task { @MainActor in
            await previous?.value
            for url in files { await self.attachOne(url) }
        }
        composeSaving = task
        await task.value
    }

    private func attachOne(_ url: URL) async {
        guard let client = core, composer != nil else { return }
        let isDirectory = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false
        if isDirectory {
            flash = L("A folder cannot be attached: %@", url.lastPathComponent)
            return
        }
        let staged = Compose.stagedName()
        let destination = incomingDirectory.appendingPathComponent(staged)
        let dir = incomingDirectory
        // Swift note: `Task.detached` runs the copy away from the main
        // actor, so that a big file does not freeze the window; `try await
        // ….value` waits for it and passes its error on.
        do {
            try await Task.detached {
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
                try FileManager.default.copyItem(at: url, to: destination)
            }.value
        } catch {
            flash = L("Cannot attach %@: %@", url.lastPathComponent, error.localizedDescription)
            return
        }
        let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
        // The file goes into a draft written: a proposal is written first.
        guard await ensureWritten(), let c = composer else {
            try? FileManager.default.removeItem(at: destination)
            return
        }
        do {
            let r: AddDraftAttachmentResult = try await client.call(
                "add_draft_attachment",
                ["id": c.draft.id, "revision": c.revision, "staged": staged,
                 "filename": url.lastPathComponent, "mime_type": mime])
            guard composer?.opened == c.opened else { return }
            composer?.revision = r.revision
            composer?.attachments.append(r.attachment)
            flash = r.warning.map { L("Attached %@ · %@", r.attachment.filename, $0) } ?? L("Attached %@", r.attachment.filename)
        } catch {
            // Refused: the copy is no use; the core took nothing.
            try? FileManager.default.removeItem(at: destination)
            flash = Self.composeErrorFlash(error)
        }
    }

    /// ⌫ on a file of the line of attachments: it leaves the draft.
    func removeAttachment(_ id: Int) async {
        let previous = composeSaving
        let task = Task { @MainActor in
            await previous?.value
            guard let client = self.core, let before = self.composer else { return }
            // A file of a forward proposed (negative id) is in the draft
            // once written: the same place in the list, its real id.
            let index = before.attachments.firstIndex { $0.id == id }
            guard await self.ensureWritten(), let c = self.composer else { return }
            let target = before.isWritten ? id : index.flatMap { c.attachments.indices.contains($0) ? c.attachments[$0].id : nil }
            guard let target else { return }
            do {
                let r: UpdateDraftResult = try await client.call(
                    "remove_draft_attachment", ["id": c.draft.id, "revision": c.revision, "attachment_id": target])
                guard self.composer?.opened == c.opened else { return }
                self.composer?.revision = r.revision
                self.composer?.attachments.removeAll { $0.id == target }
                let left = self.composer?.attachments.count ?? 0
                if let i = self.composer?.attachmentIndex, i >= left { self.composer?.attachmentIndex = max(0, left - 1) }
                if left == 0 { self.composer?.focus = .body }
            } catch {
                self.flash = Self.composeErrorFlash(error)
            }
        }
        composeSaving = task
        await task.value
    }

    /// The addresses to suggest for the text typed in a recipient line,
    /// those the user writes to most first, without those already there.
    func suggestRecipients() {
        guard let client = core, let c = composer, c.focus.isRecipients else { return }
        let query = c.line.text.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else {
            composer?.suggestions = []
            composer?.suggestionIndex = nil
            return
        }
        Task {
            guard let r: SuggestRecipientsResult = try? await client.call("suggest_recipients", ["query": query, "limit": 8]) else { return }
            // Only for the text still typed: an older answer arriving late
            // is dropped.
            guard let now = composer, now.focus.isRecipients, now.line.text.trimmingCharacters(in: .whitespaces) == query else { return }
            let present = Set(now.headers.all.map { $0.addr.lowercased() })
            // Swift note: in `composer?.x = …`, the write to `composer`
            // starts before the right side is evaluated; reading `composer`
            // there is a conflict of access, a crash at run time. The
            // value is made first.
            let shown = r.items.filter { !present.contains($0.addr.lowercased()) }
            composer?.suggestions = shown
            composer?.suggestionIndex = shown.isEmpty ? nil : 0
        }
    }

    /// A suggestion chosen (Return, Tab or a click): it becomes an address
    /// of the focused line.
    func chooseSuggestion(_ index: Int) {
        guard var c = composer, c.focus.isRecipients, c.suggestions.indices.contains(index) else { return }
        // Swift note: `c.add(…, to: c.focus)` would read `c` while the
        // mutating call writes it, which Swift's exclusivity rule forbids
        // (a crash at run time): the values are read first.
        let field = c.focus, chosen = c.suggestions[index]
        c.add([chosen], to: field)
        c.line = LineEdit()
        c.suggestions = []
        c.suggestionIndex = nil
        composer = c
        composerChanged()
    }

    /// Escape: back to the mail; the draft is kept (saved now if needed)
    /// and closed: the core files it in Drafts, here and on the server
    /// (`brouillons.md`, section 4). The next `r` on the mail, or on the
    /// draft in Drafts, reopens it.
    func leaveComposer() {
        guard let c = composer else { return }
        composeSaveTask?.cancel()
        Task {
            // An address typed but not yet made one is kept; a text that is
            // not one keeps the draft open, to be corrected rather than lost.
            guard await commitRecipientLine() else { return }
            await saveComposer()
            // A proposal left as it was is written nowhere: nothing to close.
            let id = composer?.isWritten == true ? composer?.draft.id : nil
            composer = nil
            guard let client = core, let id else { return }
            do {
                let _: CommandResult = try await client.call("close_draft", ["id": id])
            } catch {
                flash = Self.composeErrorFlash(error)
            }
        }
        flash = Compose.keptFlash(c)
    }

    /// ⌘⌫: the draft is abandoned; `z` gives it back with its text.
    func discardComposer() async {
        guard let client = core, let c = composer else { return }
        composeSaveTask?.cancel()
        await composeSaving?.value
        do {
            // A proposal never written: nothing to discard in the core.
            if let written = composer, written.isWritten {
                let _: DraftView = try await client.call("discard_draft", ["id": written.draft.id])
            }
            composer = nil
            let kind: DraftKind = c.draft.kind == "forward" ? .forward : c.draft.kind == "new" ? .new : .replyAll
            if c.draft.replyTo != nil || c.draft.forwardOf != nil || kind == .new {
                let gone = DiscardedDraft(replyTo: c.draft.replyTo ?? c.draft.forwardOf, body: c.buffer.text, kind: kind, account: c.draft.accountId)
                pushUndo(UndoStep(tokens: [], label: L("reply abandoned"), restoreDraft: gone))
            }
            flash = L("Reply abandoned · z to restore it")
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// ⌘↩: the last text is sent with the draft; the mail leaves after the
    /// undo delay, which `z` can cancel. `afterSend`: the filing chosen with
    /// ⇧⌘↩ (`Compose.sendChoices`); nil keeps the one chosen earlier, else
    /// the core's preselection.
    func sendComposer(afterSend: String? = nil) async {
        guard let client = core, composer != nil else { return }
        // An address still typed in a line goes with the mail; a text that
        // is not one stops the sending, to be corrected.
        guard await commitRecipientLine() else { return }
        composeSaveTask?.cancel()
        await composeSaving?.value
        // The headers are saved first: `send_draft` takes the body only.
        if composer.map({ $0.headers != $0.savedHeaders }) == true { await saveComposer() }
        guard let unsent = composer else { return }
        if unsent.headers.all.isEmpty {
            flash = L("Cannot send: %@", L("no recipient"))
            return
        }
        // A reply sent as proposed, without a word typed, is written now.
        guard await ensureWritten(), let c = composer else { return }
        do {
            var params: [String: Any] = ["id": c.draft.id, "revision": c.revision, "body": c.buffer.text]
            if let afterSend { params["after_send"] = afterSend }
            let r: SendDraftResult = try await client.call("send_draft", params)
            composer = nil
            sending[c.draft.id] = c.headers.subject
            pushUndo(UndoStep(tokens: [], label: L("sending of the reply"), cancelSend: c.draft.id))
            let seconds = max(0, r.sendAt - Int(Date().timeIntervalSince1970))
            flash = Compose.sendingFlash(seconds: seconds)
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// `z` during the undo delay: the mail does not leave, and the reply
    /// is open again with its text.
    func cancelSend(_ draftId: Int) async {
        guard let client = core else { return }
        let mark = flashCount
        do {
            let d: DraftView = try await client.call("cancel_send", ["id": draftId])
            sending[draftId] = nil
            let opened = await openComposer(d)
            sayOpening(L("Sending cancelled: the reply is open again"), for: opened, since: mark)
        } catch let e as CoreClient.RPCError where e.code == ComposeError.sendStarted {
            flash = L("Too late: the mail is leaving")
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// `z` after ⌘⌫: the reply starts again with its text.
    func restoreDraft(_ d: DiscardedDraft) async {
        guard let client = core else { return }
        do {
            var params: [String: Any] = ["kind": d.kind.rawValue, "body": d.body]
            if let replyTo = d.replyTo { params["reply_to"] = replyTo }
            if d.kind == .new, let account = d.account { params["account"] = account }
            // Proposed with its text, open at once, then written at the
            // pause like a text typed: the text given back is kept even if
            // the user leaves without typing.
            let mark = flashCount
            let r: CreateDraftResult = try await client.call("prepare_draft", params)
            let opened = await openComposer(r.draft, kind: d.kind)
            guard composer?.opened == opened else { return }
            if composer?.isWritten == false {
                composer?.saved = ""
                composerChanged()
            }
            sayOpening(L("Reply restored"), for: opened, since: mark)
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// The events of a sending (`reponse.md`, section 11).
    func draftEvent(_ method: String, _ params: [String: JSONValue]) {
        guard let id = params["draft_id"]?.int else { return }
        switch method {
        case "DraftSent":
            sending[id] = nil
            // Its sending can no longer be cancelled; its filing can be.
            undoStack.removeAll { $0.cancelSend == id }
            let undo = params["undo"]?.string
            // `filing`: a new mail's own, which the core applies once the
            // copy in Sent is made (`reponse.md`, section 8, "Nouveau
            // mail"). It comes after this token, so `z` then undoes the
            // decisions taken on the recipients alone, and says so.
            let filing = params["filing"]?.string
            if let undo {
                pushUndo(UndoStep(tokens: [undo],
                                  label: filing == nil ? L("filing of the reply") : L("decisions of the sending"),
                                  threads: params["thread_id"]?.int.map { [$0] } ?? []))
            }
            flash = Compose.sentFlash(movedTo: params["moved_to"]?.string, filing: filing, undo: undo != nil)
        case "SendFailed":
            sending[id] = nil
            undoStack.removeAll { $0.cancelSend == id }
            flash = L("Not sent: %@ · r on the mail to reopen the reply", params["reason"]?.string ?? "")
        case "SendPartial":
            let refused = params["refused"]?.strings ?? []
            flash = L("Sent, but the server refused: %@", refused.joined(separator: ", "))
        default:
            break
        }
    }

    /// The status bar for an error of the reply: what the user can do,
    /// in the app's language, for the errors the core gives a code to.
    nonisolated static func composeErrorFlash(_ error: Error) -> String {
        guard let e = error as? CoreClient.RPCError else { return error.localizedDescription }
        switch e.code {
        case ComposeError.bodyPending: return L("This mail is still downloading: try again in a moment")
        case ComposeError.cannotSend: return L("Cannot send: %@", e.message)
        case ComposeError.revisionConflict: return L("This reply was changed elsewhere: close it and reopen it")
        case ComposeError.draftSending: return L("This reply is being sent · z to cancel")
        default: return e.message
        }
    }
}
