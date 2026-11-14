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
    func startAI(_ prompt: AIPromptView, variant: AIVariantView?) async {
        guard let opened = composer?.opened, aiRun == nil else { return }
        // What was kept of earlier runs (the end of one cancelled, a run of
        // the CLI) is not this run's: only what comes from now on may be.
        aiEarly = [:]
        aiRun = AIRun(runId: 0, title: AI.title(prompt, variant: variant), composer: opened)
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
        do {
            let r: AIRunResult = try await client.call("ai_run", params)
            // Escape was pressed while waiting, or the reply left: the run
            // is not wanted.
            guard var run = aiRun, run.runId == 0, run.composer == opened, composer?.opened == opened else {
                let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": r.runId])
                return
            }
            run = AIRun(runId: r.runId, title: run.title, composer: opened)
            aiRun = run
            // What came before this answer (a quick provider ends first).
            for e in aiEarly.removeValue(forKey: r.runId) ?? [] { aiEvent(e) }
        } catch {
            if aiRun?.runId == 0 { aiRun = nil }
            flash = AI.errorFlash(error)
        }
    }

    /// `AiDelta`, `AiDone`, `AiFailed`.
    func aiEvent(_ e: CoreClient.Event) {
        guard let id = e.params["run_id"]?.int else { return }
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
        case .cancel:
            aiRun = nil
            flash = L("AI stopped")
            // Not known yet: `startAI` cancels it when its id comes.
            if run.runId > 0 {
                let _: CommandResult? = try? await client.call("ai_cancel", ["run_id": run.runId])
            }
        case .refuse, .close:
            aiRun = nil
            if case .proposal = run.phase {
                let _: CommandResult? = try? await client.call("ai_dismiss", ["run_id": run.runId])
            }
        case .accept, .choose:
            guard let c = composer, c.opened == run.composer else { return }
            var params: [String: Any] = ["run_id": run.runId, "revision": c.revision]
            if case .choose(let i) = action { params["choice"] = i }
            do {
                let r: AIApplyResult = try await client.call("ai_apply", params)
                aiRun = nil
                applied(r.draft, opened: run.composer)
            } catch {
                flash = AI.errorFlash(error)
            }
        }
    }

    /// The draft as `ai_apply` saved it: the text (kept for ⌘Z) or the
    /// subject, and the revision.
    private func applied(_ d: DraftView, opened: UUID) {
        guard var c = composer, c.opened == opened else { return }
        let before = c.buffer.text
        if d.body != before {
            c.buffer = TextBuffer(d.body, cursor: AI.cursorAfter(old: before, new: d.body, cursor: c.buffer.cursor))
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
}
