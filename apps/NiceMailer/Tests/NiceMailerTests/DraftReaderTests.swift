import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// A click on the draft shown in Drafts reopens it to be edited, as Return
/// does (`InterfaceController.readerEditsDraft`, `clickDraftReader`;
/// `dossiers.md`, "Sent et Drafts").
///
/// The fake core adds a Drafts folder (id 6) with two drafts (61, 62).
@MainActor
struct DraftReaderTests {
    let core: FakeCore
    let model = AppModel()
    let controller: InterfaceController

    init() async {
        core = FakeCore { method, params in
            switch method {
            case "list_folders":
                var folders = Fixture.folders
                folders.append(Fixture.folder(6, "drafts", "Drafts", "R", targets: ["deleted"]))
                return ["items": folders, "as_of": Fixture.position]
            case "list_threads":
                let folder = (params["folder_ids"] as? [Int])?.first ?? params["folder_id"] as? Int ?? 1
                guard folder == 6 else { return Fixture.answer(method, params) }
                let items = (61...62).map { Fixture.thread($0, folder: 6) }
                return ["items": items, "next_cursor": NSNull(), "as_of": Fixture.position]
            default:
                return Fixture.answer(method, params)
            }
        }
        controller = InterfaceController(model: model, defaults: TestDefaults.make(), textSize: TextSize())
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    func press(_ chars: String) {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: 0)!
        _ = controller.handle(event)
    }

    @Test func outsideDraftsAClickOnTheMailEditsNothing() {
        #expect(model.currentRole == "inbox")
        #expect(!controller.readerEditsDraft)
        controller.clickDraftReader()
        #expect(model.composer == nil)
    }

    @Test func inDraftsAClickOnTheDraftReopensIt() async {
        press(";"); press("r")
        #expect(await eventually { model.currentRole == "drafts" && model.openThread?.id == 61 })
        #expect(controller.readerEditsDraft)
        controller.clickDraftReader()
        #expect(await eventually { model.composer != nil })
        // While it is written, the reader is the reply: no layer to click.
        #expect(!controller.readerEditsDraft)
    }
}
