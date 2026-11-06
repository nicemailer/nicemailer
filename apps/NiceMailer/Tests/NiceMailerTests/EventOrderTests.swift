import Testing
@testable import NiceMailer

/// The place of an event in the stream (`evenements.md`): in order, after a
/// gap (reload everything), or transient. A transient event (`SyncProgress`,
/// sent several times a second during a synchronization) has no `seq`: it
/// must neither cause a reload nor move the position, else the next real
/// event would look like a gap too.
struct EventOrderTests {
    func event(_ method: String, _ epoch: String, _ seq: Int?) -> CoreClient.Event {
        CoreClient.Event(method: method, epoch: epoch, seq: seq, params: [:])
    }

    @Test func theNextEventIsInOrder() {
        let last = Position(epoch: "e_1", seq: 7)
        #expect(AppModel.order(of: event("FolderChanged", "e_1", 8), after: last) == .inOrder(Position(epoch: "e_1", seq: 8)))
    }

    @Test func aMissedEventOrAnotherEpochIsAGap() {
        let last = Position(epoch: "e_1", seq: 7)
        #expect(AppModel.order(of: event("FolderChanged", "e_1", 9), after: last).isGap)
        #expect(AppModel.order(of: event("FolderChanged", "e_2", 8), after: last).isGap)
        #expect(AppModel.order(of: event("FolderChanged", "e_1", 1), after: nil).isGap)
    }

    @Test func aTransientEventStaysOutOfTheSequence() {
        let last = Position(epoch: "e_1", seq: 7)
        #expect(AppModel.order(of: event("SyncProgress", "e_1", nil), after: last) == .transient)
        // The position is unchanged, so the next numbered event is in order.
        #expect(AppModel.order(of: event("MessageArrived", "e_1", 8), after: last) == .inOrder(Position(epoch: "e_1", seq: 8)))
    }

    /// `ActionReverted`: the server refused a change made in NiceMailer,
    /// which the core undid. The status bar says what, and the server's
    /// answer.
    @Test func aRefusedChangeIsSaidInTheStatusBar() {
        // Compared through `L`: the tests run in the language of the Mac.
        let move = AppModel.revertedFlash(kind: "move", reason: "Mailbox doesn't exist: Archive")
        #expect(move == L("The server refused the move") + " · " + L("undone: %@", "Mailbox doesn't exist: Archive"))
        #expect(AppModel.revertedFlash(kind: "set_flags", reason: "quota").hasPrefix(L("The server refused the change")))
    }
}
