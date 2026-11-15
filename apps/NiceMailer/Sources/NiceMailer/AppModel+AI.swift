import Foundation

// The AI assistance of the reply (`specifications/ia.md`, section 11; the
// rules in `AI.swift`): the menu's prompts, a run from its start to the
// user's answer, and ⌘Z after an accepted proposal.

extension AppModel {
    /// The prompts of an AI menu, or `nil` when the AI is off (the status
    /// bar then says how to turn it on) or the core cannot be asked.
    ///
    /// `screen`: `compose`, `reading`, `list`; `folder`: the role of the
    /// folder shown.
    func aiPrompts(screen: String, folder: String?) async -> [AIPromptView]? {
        guard let client = core else { return nil }
        do {
            let status: AIStatusResult = try await client.call("ai_status")
            guard status.enabled else {
                flash = L("The AI is off: set ai.enabled = true in config.toml")
                return nil
            }
            var params: [String: Any] = ["screen": screen, "lang": AI.language]
            if let folder { params["folder"] = folder }
            return try await client.call("ai_list_prompts", params)
        } catch {
            flash = AI.errorFlash(error)
            return nil
        }
    }

    /// Runs a prompt on the reply being written. The text is saved first:
    /// the core reads the draft as saved, at its revision (`ai_run`).
    ///
    /// The panel opens at once, before the core answers, and takes the
    /// keys from then on: the text cannot change under the run. Its run id
    /// is not known yet (`0`); Escape meanwhile closes it, and the run is
    /// cancelled as soon as its id comes.
    /// `passage`: the text selected when ⌘I was pressed, in UTF-16 units
    /// of the body (the units of the core's `selection`); the prompt then
    /// works on it alone (`ia.md`, section 4, `on`).
    func startAI(_ prompt: AIPromptView, variant: AIVariantView?, passage: NSRange? = nil) async {
        guard let opened = composer?.opened, aiRun == nil else { return }
        // What was kept of earlier runs (the end of one cancelled, a run of
        // the CLI) is not this run's: only what comes from now on may be.
        aiEarly = [:]
        aiRun = AIRun(runId: 0, title: AI.title(prompt, variant: variant), target: .reply(opened))
        // The pending save now, rather than after its pause.
        composeSaveTask?.cancel()
        await saveComposer()
        guard await ensureWritten(), let c = composer, c.opened == opened, aiRun?.runId == 0 else {
            if aiRun?.runId == 0 { aiRun = nil }
            return
        }
        guard !c.isDirty else {
            aiRun = nil
            flash = L("The reply could not be saved: try again")
            return
        }
        guard let client = core else { return }
        var params: [String: Any] = [
            "prompt": prompt.id, "draft_id": c.draft.id, "revision": c.revision, "lang": AI.language,
        ]
        if let v = variant { params["variant"] = v.key }
        // The text saved above is the text the passage was selected in
        // (`isDirty` is false): its units point at the same characters.
        if let passage {
            params["selection"] = ["start": passage.location, "end": passage.location + passage.length]
        }
        do {
            let r: AIRunResult = try await client.call("ai_run", params)
            // Escape was pressed while waiting, or the reply left: the run
            // is not wanted.
            guard let run = aiRun, run.runId == 0, run.composer == opened, composer?.opened == opened else {
                let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": r.runId])
                return
            }
            started(r.runId, run)
        } catch {
            if aiRun?.runId == 0 { aiRun = nil }
            flash = AI.errorFlash(error)
        }
    }

    /// Runs a prompt on a conversation (`a` in the list or the reader):
    /// on the mail under the reader's cursor for a prompt of one mail, the
    /// core choosing the latest received when there is none. For a prompt
    /// of an attachment, on that file, `message` being its mail: the core
    /// reads its text (PDF, DOCX…) before asking the AI.
    func startMailAI(_ prompt: AIPromptView, variant: AIVariantView?, thread: Int, message: Int?,
                     attachment: AttachmentView? = nil) async {
        guard aiRun == nil, let client = core else { return }
        aiEarly = [:]
        let target = AIRun.Target.mail(thread: thread, message: message)
        // A file's prompt names the file: "Summarize · devis.pdf".
        let title = attachment.map { "\(prompt.label) · \(AttachmentDisplay.name($0))" } ?? AI.title(prompt, variant: variant)
        aiRun = AIRun(runId: 0, title: title, target: target)
        var params: [String: Any] = ["prompt": prompt.id, "thread_id": thread, "lang": AI.language]
        if let message, prompt.scope == "message" { params["message_id"] = message }
        if let attachment { params["attachment_id"] = attachment.id }
        if let v = variant { params["variant"] = v.key }
        do {
            let r: AIRunResult = try await client.call("ai_run", params)
            guard let run = aiRun, run.runId == 0, run.target == target else {
                let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": r.runId])
                return
            }
            started(r.runId, run)
        } catch {
            if aiRun?.runId == 0 { aiRun = nil }
            flash = AI.errorFlash(error)
        }
    }

    /// The run's id is known: the panel follows it, and what came for it
    /// before this answer is played (a quick provider ends first).
    private func started(_ runId: Int, _ run: AIRun) {
        aiRun = AIRun(runId: runId, title: run.title, target: run.target)
        for e in aiEarly.removeValue(forKey: runId) ?? [] { aiEvent(e) }
    }

    /// `AiDelta`, `AiDone`, `AiFailed`, and those of a sorting.
    func aiEvent(_ e: CoreClient.Event) {
        guard let id = e.params["run_id"]?.int else { return }
        // A sorting: its own state, not the panel's.
        if e.method == "AiSuggestions" {
            // Any client's sorting: the rows shown take its suggestions.
            let ids = Set(e.params["thread_ids"]?.ints ?? [])
            if !ids.isDisjoint(with: threads.map(\.id)) { scheduleSuggestions() }
            return
        }
        if let sort = aiSort, sort.runId == id {
            sortEvent(e, sort)
            return
        }
        if aiSort?.runId == 0, e.method != "AiDelta" {
            if aiEarly.count < 32 { aiEarly[id, default: []].append(e) }
            return
        }
        guard var run = aiRun, run.runId == id else {
            // Not (yet) ours: kept a while, in case `ai_run` has not
            // answered yet. A run of another client (the CLI) ends up here
            // too, hence the bound.
            if aiEarly.count < 32 { aiEarly[id, default: []].append(e) }
            return
        }
        switch e.method {
        case "AiDelta":
            guard case .running = run.phase else { return }
            run.text += e.params["text"]?.string ?? ""
            aiRun = run
        case "AiDone":
            Task { await fetchProposal(id) }
        case "AiFailed":
            if e.params["code"]?.string == "ai_cancelled" {
                aiRun = nil
                return
            }
            run.phase = .failed(e.params["message"]?.string ?? L("The AI failed"))
            aiRun = run
        default:
            break
        }
    }

    private func sortEvent(_ e: CoreClient.Event, _ sort: AISort) {
        switch e.method {
        case "AiProgress":
            var s = sort
            s.done = e.params["done"]?.int ?? s.done
            s.total = e.params["total"]?.int ?? s.total
            aiSort = s
        case "AiDone":
            aiSort = nil
            Task {
                guard let client = core else { return }
                let v: AIRunView? = try? await client.call("ai_get_run", ["run_id": sort.runId])
                if case .targets(let items, let failed)? = v?.proposal {
                    flash = AI.sortedFlash(found: items.count, failedBatches: failed)
                }
                scheduleSuggestions()
            }
        case "AiFailed":
            aiSort = nil
            if e.params["code"]?.string != "ai_cancelled" {
                flash = L("AI: %@", e.params["message"]?.string ?? L("The AI failed"))
            }
        default:
            break
        }
    }

    /// Sorts conversations (`a` `s`): those marked, else what the prompt's
    /// `unmarked` says, the conversation under the cursor or the whole
    /// folder (the core takes it). Nothing takes the keys meanwhile: the
    /// suggestions show on the rows as each batch ends.
    func startSort(_ prompt: AIPromptView) async {
        guard aiSort == nil, let client = core else { return }
        var params: [String: Any] = ["prompt": prompt.id, "folder_ids": listedFolderIds, "lang": AI.language]
        if !marked.isEmpty {
            params["thread_ids"] = Array(marked)
        } else if prompt.unmarked == "cursor" {
            guard let t = selectedThreadId else { flash = L("No mail to work on"); return }
            params["thread_ids"] = [t]
        }
        aiEarly = [:]
        aiSort = AISort(runId: 0, title: prompt.label)
        do {
            let r: AIRunResult = try await client.call("ai_run", params)
            guard aiSort?.runId == 0 else {
                let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": r.runId])
                return
            }
            aiSort = AISort(runId: r.runId, title: prompt.label)
            for e in aiEarly.removeValue(forKey: r.runId) ?? [] { aiEvent(e) }
        } catch {
            aiSort = nil
            flash = AI.errorFlash(error)
        }
    }

    /// Escape during a sorting: it stops; what is sorted stays.
    func stopSort() async {
        guard let sort = aiSort else { return }
        aiSort = nil
        flash = L("AI stopped")
        if sort.runId > 0, let client = core {
            let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": sort.runId])
        }
    }

    /// Whether the AI is on, from the core: suggestions are only asked
    /// for then.
    func refreshAIEnabled() async {
        guard let client = core else { return }
        let s: AIStatusResult? = try? await client.call("ai_status")
        aiEnabled = s?.enabled ?? false
        scheduleSuggestions()
    }

    /// The suggestions of the rows shown, a moment after they change (a
    /// burst of reloads asks once).
    func scheduleSuggestions() {
        guard aiEnabled else {
            if !aiSuggestions.isEmpty { aiSuggestions = [:] }
            return
        }
        aiSuggestionsTask?.cancel()
        aiSuggestionsTask = Task {
            try? await Task.sleep(for: .milliseconds(50))
            guard !Task.isCancelled, let client = core else { return }
            let ids = threads.map(\.id)
            guard !ids.isEmpty else { aiSuggestions = [:]; return }
            guard let list: [AISuggestion] = try? await client.call("ai_suggestions", ["thread_ids": ids]) else { return }
            guard !Task.isCancelled else { return }
            aiSuggestions = Dictionary(list.map { ($0.threadId, $0) }, uniquingKeysWith: { a, _ in a })
        }
    }

    /// The suggestion of the conversation under the cursor.
    var selectedSuggestion: AISuggestion? {
        selectedThreadId.flatMap { aiSuggestions[$0] }
    }

    /// Return on a conversation the AI suggested a target for: moved
    /// there, as the target's key would (`ia.md`, section 11).
    func acceptSuggestion() -> Bool {
        guard let s = selectedSuggestion, let t = selectedThread else { return false }
        moveTargets(to: s.target, only: [t])
        return true
    }

    /// After missed events: the state of the run still waited for, from
    /// the core, in case its end was among them.
    func aiResync() async {
        guard let run = aiRun, run.runId > 0, case .running = run.phase, let client = core else { return }
        guard let v: AIRunView = try? await client.call("ai_get_run", ["run_id": run.runId]) else { return }
        switch v.state {
        case "done": await fetchProposal(run.runId)
        case "failed", "cancelled":
            guard var r = aiRun, r.runId == run.runId else { return }
            r.phase = .failed(v.error ?? L("The AI failed"))
            aiRun = r
        default: break
        }
    }

    private func fetchProposal(_ id: Int) async {
        guard let client = core else { return }
        do {
            let v: AIRunView = try await client.call("ai_get_run", ["run_id": id])
            guard var run = aiRun, run.runId == id else { return }
            run.phase = v.proposal.map { .proposal($0) } ?? .failed(v.error ?? L("The AI gave nothing to apply"))
            aiRun = run
        } catch {
            guard var run = aiRun, run.runId == id else { return }
            run.phase = .failed(AI.errorFlash(error))
            aiRun = run
        }
    }

    /// The user's answer to the panel (`AI.action`).
    func answerAI(_ action: AIKeyAction) async {
        guard let run = aiRun, let client = core else { return }
        switch action {
        case .hint:
            flash = AI.hint(run.phase)
        case .move(let i):
            // `GeekController.handleAIKey` applies it itself, at once.
            aiRun?.choice = i
        case .cancel:
            aiRun = nil
            flash = L("AI stopped")
            // Not known yet: `startAI` cancels it when its id comes.
            if run.runId > 0 {
                let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": run.runId])
            }
        case .copy:
            // `GeekController.handleAIKey` copies it itself: the pasteboard
            // is the controller's (a private one in tests).
            break
        case .refuse, .close:
            aiRun = nil
            if case .proposal = run.phase {
                let _: CommandResult? = try? await client.call("ai_dismiss", ["run_id": run.runId])
            }
        case .accept, .choose, .open:
            var params: [String: Any] = ["run_id": run.runId]
            if case .choose(let i) = action { params["choice"] = i }
            if case .open(let i) = action { params["choice"] = i }
            switch run.target {
            case .reply(let opened):
                guard let c = composer, c.opened == opened else { return }
                params["revision"] = c.revision
            case .mail:
                break
            }
            do {
                let r: AIApplyResult = try await client.call("ai_apply", params)
                // An appointment: its file saved, the panel stays open, to
                // save another date of the mail (the core keeps the
                // proposal).
                if let file = r.calendar {
                    let dest = try await saveAIFile(file)
                    let place = FileManager.default.displayName(atPath: saveDirectory.path)
                    if case .open = action {
                        // Opened with its default app, Calendar, which asks
                        // which calendar to add it to.
                        openFile(dest)
                        flash = L("%@ saved to %@ and opened", dest.lastPathComponent, place)
                    } else {
                        flash = L("%@ saved to %@", dest.lastPathComponent, place)
                    }
                    return
                }
                // The sender's card: saved, and opened with ⌘↩ (Contacts,
                // the default app of a .vcf, asks before adding it). One
                // card: the core ended the run, the panel closes.
                if let file = r.contact {
                    aiRun = nil
                    let dest = try await saveAIFile(file)
                    let place = FileManager.default.displayName(atPath: saveDirectory.path)
                    if case .open = action {
                        openFile(dest)
                        flash = L("%@ saved to %@ and opened", dest.lastPathComponent, place)
                    } else {
                        flash = L("%@ saved to %@", dest.lastPathComponent, place)
                    }
                    return
                }
                aiRun = nil
                if let d = r.draft, let opened = run.composer { applied(d, opened: opened) }
                if let text = r.text, case .mail(_, let message) = run.target { await replyWith(text, current: message) }
            } catch {
                flash = AI.errorFlash(error)
            }
        }
    }

    /// A quick reply chosen: the reply to the mail (`r`), the text above the
    /// quotation, the cursor after it, saved at the next pause as if typed.
    private func replyWith(_ text: String, current: Int?) async {
        await startReply(current: current)
        guard var c = composer else { return }
        c.buffer = AI.insertReply(text, into: c.buffer)
        composer = c
        composerChanged()
    }

    /// The draft as `ai_apply` saved it: the text (kept for ⌘Z) or the
    /// subject, and the revision.
    private func applied(_ d: DraftView, opened: UUID) {
        guard var c = composer, c.opened == opened else { return }
        let before = c.buffer.text
        if d.body != before {
            // A passage selected (other wordings…): from its end,
            // so the cursor lands after the new words, where writing goes
            // on, whichever way the selection was made.
            let from = c.buffer.selection?.upperBound ?? c.buffer.cursor
            c.buffer = TextBuffer(d.body, cursor: AI.cursorAfter(old: before, new: d.body, cursor: from))
            c.aiUndo = AIUndo(before: before, after: c.buffer.text)
            flash = L("Applied · ⌘Z undoes it")
        }
        if d.subject != c.headers.subject {
            c.headers.subject = d.subject
            c.savedHeaders.subject = d.subject
            c.subjectLine = LineEdit(d.subject)
            flash = L("Subject: %@", d.subject)
        }
        c.revision = d.revision
        c.saved = c.buffer.text
        composer = c
    }

    /// ⌘Z in the reply: the text before the last AI proposal accepted, if
    /// it is still the text the AI gave. True when something was undone.
    func undoAI() -> Bool {
        guard var c = composer, let u = c.aiUndo else { return false }
        c.aiUndo = nil
        guard c.buffer.text == u.after else {
            composer = c
            flash = L("The text changed since the AI's change: nothing undone")
            return true
        }
        c.buffer = TextBuffer(u.before, cursor: AI.cursorAfter(old: u.after, new: u.before, cursor: c.buffer.cursor))
        composer = c
        flash = L("AI change undone")
        composerChanged()
        return true
    }

    /// Writes a file of the AI (the calendar file of an appointment, the
    /// vCard of a contact) to `saveDirectory` (~/Downloads by default),
    /// under its name or a free variant of it ("… (2).ics"), never
    /// overwriting a file; returns where. Opening it adds the event to
    /// Calendar or the card to Contacts; the app does not, the user decides.
    func saveAIFile(_ file: AIFile) async throws -> URL {
        let dir = saveDirectory
        // Written off the main thread, like the copies of attachments.
        return try await Task.detached {
            let fm = FileManager.default
            let dest = AttachmentDisplay.freeURL(in: dir, name: file.filename) { fm.fileExists(atPath: $0.path) }
            // `withoutOverwriting`: a file created between the check and the
            // write is not replaced, the write fails instead.
            try Data(file.content.utf8).write(to: dest, options: .withoutOverwriting)
            return dest
        }.value
    }
}
