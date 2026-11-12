import Foundation
import Testing
@testable import NiceMailer

/// A mail is listed and opened as soon as its headers are in; its body
/// comes later, announced by a lone `MessageChanged` (no `ThreadChanged`:
/// `Inserter::complete`). The reader must show the body when it arrives,
/// without the user leaving the mail and coming back.
@MainActor
struct BodyArrivalTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()

    init() async {
        // Thread 1's body is not downloaded yet: `body_text` is null.
        core.handler = Self.withoutBody
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread?.id == 1 }
        // The first event is a gap for the model (no position yet), which
        // reloads everything: received and settled here, so the events of
        // the tests are in order and show what they alone reload.
        let before = model.refreshCount
        model.handle(messageChanged(1, thread: 999))
        _ = await eventually { model.refreshCount == before + 1 }
        try? await Task.sleep(for: .milliseconds(100))
    }

    /// The fixture, with the messages of thread 1 still without a body.
    nonisolated static func withoutBody(_ method: String, _ params: [String: Any]) -> Any {
        guard method == "get_thread", params["thread_id"] as? Int == 1 else { return Fixture.answer(method, params) }
        var view = Fixture.threadView(1)
        if var messages = view["messages"] as? [[String: Any]] {
            for i in messages.indices {
                messages[i]["body_text"] = NSNull()
                messages[i]["segments"] = []
                messages[i]["links"] = []
            }
            view["messages"] = messages
        }
        return view
    }

    func messageChanged(_ seq: Int, thread: Int) -> CoreClient.Event {
        CoreClient.Event(method: "MessageChanged", epoch: "e_test", seq: seq,
                         params: ["message_id": .int(thread * 10), "thread_id": .int(thread)])
    }

    @Test func theOpenMailShowsItsBodyWhenItArrives() async {
        #expect(model.openThread?.messages.first?.bodyText == nil)
        // The body is stored by the core, which announces it.
        core.handler = Fixture.answer
        model.handle(messageChanged(2, thread: 1))
        #expect(await eventually { model.openThread?.messages.first?.bodyText != nil })
        #expect(model.openThread?.id == 1)
    }

    /// A listed mail that is not open: its list is reloaded, for the
    /// snippet that came with the body.
    @Test func aListedMailReloadsItsList() async {
        let lists = core.calls("list_threads").count
        let threads = core.calls("get_thread").count
        model.handle(messageChanged(2, thread: 2))
        #expect(await eventually { core.calls("list_threads").count > lists })
        // Thread 2 is not open: the reader is not reloaded.
        #expect(core.calls("get_thread").count == threads)
    }

    /// A mail neither open nor listed (the bodies of a whole account come
    /// in like this): nothing is reloaded.
    @Test func aMailNotOnScreenReloadsNothing() async {
        let before = model.refreshCount
        model.handle(messageChanged(2, thread: 999))
        try? await Task.sleep(for: .milliseconds(150))
        #expect(model.refreshCount == before)
    }
}
