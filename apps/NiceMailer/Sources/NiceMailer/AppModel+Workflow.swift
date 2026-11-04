import Foundation

// The folder workflow seen from the app (`dossiers.md`): the decision a
// move takes on the sender, its confirmation when the sender's mail is
// suspected of spoofing, decisions taken on the contacts screen, and `z`,
// which undoes them. The rules themselves live in the core; the app shows
// what the core did and keeps the tokens that undo it.

/// One step of `z`: the undo tokens of a command given on one or several
/// threads, and what to call it in the message.
struct UndoStep: Equatable {
    let tokens: [String]
    let label: String
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
                if let u = r.undo { pushUndo(UndoStep(tokens: [u], label: L("move to %@", Role.name(s.target)))) }
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
