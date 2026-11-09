import Foundation

// The reply, seen from the model (`specifications/reponse.md`): `r` asks
// the core for a draft (`create_draft`), the text typed is saved after
// each pause (`update_draft`), ⌘↩ queues it (`send_draft`), `z` cancels it
// during the undo delay (`cancel_send`), and the core's events say how it
// ended (`DraftSent`, `SendFailed`, `SendPartial`).

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
    /// (`Compose.replyTarget`), or the reply already started to it.
    func startReply(current: Int?) async {
        guard composer == nil else { return }
        guard let client = core, let thread = openThread, let m = Compose.replyTarget(thread, current: current) else {
            flash = L("No mail to answer")
            return
        }
        do {
            let r: CreateDraftResult = try await client.call("create_draft", ["reply_to": m.id, "quote_header": Compose.quoteHeader(m)])
            if r.draft.state == "sending" {
                // Already on its way: `z` cancels it while it can.
                flash = L("This reply is being sent · z to cancel")
                return
            }
            composer = Composer(draft: r.draft)
            flash = r.reused ? L("Reply resumed") : nil
        } catch {
            flash = Self.composeErrorFlash(error)
        }
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
        guard let client = core, let c = composer, c.isDirty else { return }
        let text = c.buffer.text
        do {
            let r: UpdateDraftResult = try await client.call("update_draft", ["id": c.draft.id, "revision": c.revision, "body": text])
            // The user may have left, or started another reply, meanwhile.
            if composer?.draft.id == c.draft.id {
                composer?.revision = r.revision
                composer?.saved = text
            }
        } catch {
            flash = Self.composeErrorFlash(error)
        }
    }

    /// Escape: back to the mail; the draft is kept (saved now if needed)
    /// and closed: the core files it in Drafts, here and on the server
    /// (`brouillons.md`, section 4). The next `r` on the mail, or on the
    /// draft in Drafts, reopens it.
    func leaveComposer() {
        guard let id = composer?.draft.id else { return }
        composeSaveTask?.cancel()
        Task {
            await saveComposer()
            composer = nil
            guard let client = core else { return }
            do {
                let _: CommandResult = try await client.call("close_draft", ["id": id])
            } catch {
                flash = Self.composeErrorFlash(error)
            }
        }
        flash = L("Reply kept in Drafts · r to resume")
    }

    /// ⌘⌫: the draft is abandoned; `z` gives it back with its text.
    func discardComposer() async {
        guard let client = core, let c = composer else { return }
        composeSaveTask?.cancel()
        await composeSaving?.value
        do {
            let _: DraftView = try await client.call("discard_draft", ["id": c.draft.id])
            composer = nil
            if let replyTo = c.draft.replyTo {
                pushUndo(UndoStep(tokens: [], label: L("reply abandoned"), restoreDraft: DiscardedDraft(replyTo: replyTo, body: c.buffer.text)))
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
        composeSaveTask?.cancel()
        await composeSaving?.value
        guard let c = composer else { return }
        do {
            var params: [String: Any] = ["id": c.draft.id, "revision": c.revision, "body": c.buffer.text]
            if let afterSend { params["after_send"] = afterSend }
            let r: SendDraftResult = try await client.call("send_draft", params)
            composer = nil
            sending[c.draft.id] = c.draft.subject
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
        do {
            let d: DraftView = try await client.call("cancel_send", ["id": draftId])
            sending[draftId] = nil
            composer = Composer(draft: d)
            flash = L("Sending cancelled: the reply is open again")
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
            let r: CreateDraftResult = try await client.call("create_draft", ["reply_to": d.replyTo, "body": d.body])
            composer = Composer(draft: r.draft)
            flash = L("Reply restored")
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
            if let undo { pushUndo(UndoStep(tokens: [undo], label: L("filing of the reply"))) }
            flash = Compose.sentFlash(movedTo: params["moved_to"]?.string, undo: undo != nil)
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
