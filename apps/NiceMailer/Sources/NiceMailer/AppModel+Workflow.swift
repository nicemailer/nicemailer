import Foundation

// The folder workflow seen from the app (`dossiers.md`): the decision a
// move takes on the sender, its confirmation when the sender's mail is
// suspected of spoofing, decisions taken on the contacts screen, and `z`,
// which undoes them. The rules themselves live in the core; the app shows
// what the core did and keeps the tokens that undo it.

/// One step of `z`: the undo tokens of a command given on one or several
/// threads, and what to call it in the message. Or, for a reply
/// (`AppModel+Compose.swift`): a mail waiting for its undo delay, whose
/// sending `z` cancels; a draft abandoned, which `z` gives back.
struct UndoStep: Equatable {
    let tokens: [String]
    let label: String
    /// The threads the command acted on: `z` puts the cursor back on the
    /// first of them still listed (`Workflow.cursorAfterUndo`).
    var threads: [Int] = []
    /// The draft whose sending `z` cancels (`cancel_send`).
    var cancelSend: Int? = nil
    /// The draft abandoned with ⌘⌫: `z` starts it again with its text.
    var restoreDraft: DiscardedDraft? = nil
}

/// What it takes to give back an abandoned draft: the mail it answers and
/// its text.
struct DiscardedDraft: Equatable {
    /// The mail answered or forwarded; nil for a new mail.
    let replyTo: Int?
    let body: String
    var kind: DraftKind = .replyAll
    /// The account of a new mail.
    var account: Int? = nil
}

/// A move the core refused until the user confirms it.
struct SuspectMove: Equatable {
    let threadId: Int
    let folderId: Int
    let target: String
    let subject: String
    /// The sender, and its authentication verdict (`fail`, `none`…).
    let addr: String
    let verdict: String
}

/// Texts of the workflow, as pure functions (tested without a core).
enum Workflow {
    /// Where the cursor goes once `z` has put mails back: on the first
    /// thread of the list the command acted on, else where it was.
    ///
    /// Why: the move had taken the cursor to the next thread; left there,
    /// a second Delete after `z` hit that other thread instead of the one
    /// brought back. A pure function, tested alone.
    static func cursorAfterUndo(restored: [Int], listed: [Int], current: Int?) -> Int? {
        let back = Set(restored)
        return listed.first { back.contains($0) } ?? current
    }

    /// Most steps `z` keeps; older ones are forgotten.
    static let undoDepth = 50

    /// Message after a move: the mail (or the number of threads) and the
    /// folder; then, when the core took a decision on the sender, that the
    /// sender's next mails go there too, how many of its other mails were
    /// filed, and that `z` undoes it all.
    static func moveFlash(subjects: [String], folder: String, decision: DecisionView?, filed: Int) -> String {
        var text = subjects.count == 1
            ? L("“%@” moved to %@", Format.flashSubject(subjects[0]), folder)
            : L("%lld threads moved to %@", subjects.count, folder)
        if let d = decision {
            text += " · " + L("next mails of %@ too", d.name)
        }
        if filed > 0 {
            text += " · " + L("%lld other mails filed", filed)
        }
        if decision != nil || filed > 0 {
            text += " · " + L("z: undo")
        }
        return text
    }

    /// Message after a decision taken on the contacts screen.
    static func decisionFlash(_ d: DecisionView?, filed: Int) -> String {
        guard let d else { return L("No change") }
        guard let after = d.after else { return L("No decision on %@ any more", d.name) }
        var text = L("%@ → %@", d.name, Role.name(after))
        if filed > 0 { text += " · " + L("%lld mails filed", filed) }
        return text + " · " + L("z: undo")
    }

    /// Name of a decision for the contacts screen and the menus: the
    /// folder's name, or nothing.
    static func targetName(_ target: String?) -> String {
        target.map { Role.name($0) } ?? ""
    }
}

extension AppModel {
    /// Remembers a step for `z`, keeping the most recent ones.
    func pushUndo(_ step: UndoStep) {
        undoStack.append(step)
        if undoStack.count > Workflow.undoDepth { undoStack.removeFirst(undoStack.count - Workflow.undoDepth) }
    }

    /// `z`: undoes the last move or decision (`ui.md`). Each `z` goes one
    /// step further back. A mail moved again since stays where it is: the
    /// core only puts back what is still where the command left it.
    func undoLast() async {
        guard let client = core else { return }
        guard let step = undoStack.popLast() else {
            flash = L("Nothing to undo")
            return
        }
        if let draft = step.cancelSend {
            await cancelSend(draft)
            return
        }
        if let d = step.restoreDraft {
            await restoreDraft(d)
            return
        }
        var moved = 0
        // Last command first, as the core undoes each one backwards.
        for token in step.tokens.reversed() {
            do {
                let r: WorkflowResult = try await client.call("undo", ["token": token])
                moved += r.moved
            } catch {
                report(error)
            }
        }
        flash = L("Undone: %@ (%lld mails put back)", step.label, moved)
        await reloadThreads(keepDepth: true)
        selectedThreadId = Workflow.cursorAfterUndo(restored: step.threads, listed: threads.map(\.id), current: selectedThreadId)
    }

    /// The user confirmed the suspect move: it is sent again with the
    /// confirmation. `trustTool`: the sender uses a tool that fails
    /// authentication, so its mail is accepted whatever the verdict
    /// (`auth_policy = not_required`).
    func confirmSuspect(trustTool: Bool) {
        guard let client = core, let s = suspectMove else { return }
        suspectMove = nil
        threads.removeAll { $0.id == s.threadId }
        if selectedThreadId == s.threadId { selectedThreadId = threads.first?.id }
        var params: [String: Any] = [
            "thread_id": s.threadId, "target": s.target, "from_folder_id": s.folderId, "confirm_suspect": true,
        ]
        if trustTool { params["auth_policy"] = "not_required" }
        Task {
            do {
                let r: WorkflowResult = try await client.call("move_thread", params)
                if let u = r.undo { pushUndo(UndoStep(tokens: [u], label: L("move to %@", Role.name(s.target)), threads: [s.threadId])) }
                flash = Workflow.moveFlash(subjects: [s.subject], folder: Role.name(s.target), decision: r.decision, filed: r.filed)
            } catch {
                report(error)
            }
            await reloadThreads(keepDepth: true)
        }
    }

    /// Escape on the confirmation: nothing moves.
    func cancelSuspect() {
        guard let s = suspectMove else { return }
        suspectMove = nil
        flash = L("Not moved: %@", Format.flashSubject(s.subject))
    }

    /// Takes, changes or removes (`target == nil`) the decision on an
    /// address, or on a domain, from the contacts screen. The core files the
    /// mails already received; `z` undoes it.
    /// Forces an address to be a person or a robot, or gives it back to the
    /// core's verdict (`kind` nil), from the contacts screen
    /// (`expediteurs.md`, section 9). Undoable like a decision. The list is
    /// loaded again when filtered on human / robot: its content changed.
    func setSenderKind(_ addressId: Int, kind: String?) async {
        guard let client = core else { return }
        var params: [String: Any] = ["address_id": addressId]
        if let kind { params["kind"] = kind }
        do {
            let r: WorkflowResult = try await client.call("set_sender_kind", params)
            if let u = r.undo { pushUndo(UndoStep(tokens: [u], label: L("origin of the address"))) }
            flash = switch kind {
            case "person": L("Marked as written by a person")
            case "robot": L("Marked as written by a robot")
            default: L("Origin computed from the mails again")
            }
        } catch {
            report(error)
        }
    }

    func setScreening(_ name: String, domain: Bool, target: String?) async {
        guard let client = core else { return }
        let method = domain ? "set_domain_screening" : "set_address_screening"
        var params: [String: Any] = [domain ? "domain" : "addr": name]
        if let target { params["target"] = target }
        do {
            let r: WorkflowResult = try await client.call(method, params)
            if let u = r.undo { pushUndo(UndoStep(tokens: [u], label: L("decision on %@", r.decision?.name ?? name))) }
            flash = Workflow.decisionFlash(r.decision, filed: r.filed)
        } catch {
            report(error)
        }
    }
}
