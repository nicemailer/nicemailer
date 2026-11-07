import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// The pure rules of the reply screen (`Compose`).
struct ComposeRulesTests {
    func message(_ id: Int, role: String = "inbox", name: String? = "Alice Martin", addr: String? = "alice@x.fr", date: Int = 1_794_042_720) throws -> MessageView {
        let json: [String: Any] = [
            "id": id, "folder_id": 1, "folder_role": role, "subject": "S", "from_name": name ?? NSNull(),
            "from_addr": addr ?? NSNull(), "to": [], "cc": [], "date_sent": date, "date_received": date,
            "body_text": "x", "is_seen": true, "is_flagged": false, "is_answered": false, "has_attachments": false,
            "auth_result": "pass", "body_html": NSNull(), "has_remote_content": false, "from_body_format": NSNull(),
            "segments": [], "only_quotes": false, "expanded": true, "show_recipients": false,
            "attachments": [], "unsubscribe": NSNull(), "tracking_pixels": 0, "links": [],
        ]
        return try CoreClient.decoder.decode(MessageView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    func thread(_ messages: [MessageView]) -> ThreadView {
        ThreadView(id: 1, subject: "S", messages: messages, asOf: Position(epoch: "e", seq: 0))
    }

    @Test func theReplyGoesToTheMailUnderTheCursorElseTheLatestReceived() throws {
        let t = thread([try message(1), try message(2), try message(3, role: "sent")])
        // No cursor: the latest received, not my own reply in Sent.
        #expect(Compose.replyTarget(t, current: nil)?.id == 2)
        // The cursor moved with ctrl-k / ctrl-j.
        #expect(Compose.replyTarget(t, current: 1)?.id == 1)
        // A cursor on a message no longer there: the rule again.
        #expect(Compose.replyTarget(t, current: 99)?.id == 2)
        // Only my own mails: the latest.
        #expect(Compose.replyTarget(thread([try message(5, role: "sent")]), current: nil)?.id == 5)
        #expect(Compose.replyTarget(thread([]), current: nil) == nil)
    }

    @Test func theQuoteHeaderIsInTheAppsLanguage() throws {
        let utc = TimeZone(identifier: "UTC")!
        let m = try message(1)
        let en = Compose.quoteHeader(m, locale: Locale(identifier: "en_US"), timeZone: utc)
        #expect(en.contains("Nov 7, 2026"), "\(en)")
        #expect(en.contains("Alice Martin <alice@x.fr>"))
        #expect(en.hasSuffix(":"))
        // Without a name, the address alone.
        let bare = Compose.quoteHeader(try message(1, name: nil), locale: Locale(identifier: "en_US"), timeZone: utc)
        #expect(bare.contains(" alice@x.fr "), "\(bare)")
        #expect(!bare.contains("<"))
    }

    @Test func recipientsFitOnOneLine() {
        let a = AddressView(addr: "alice@x.fr", name: "Alice")
        let b = AddressView(addr: "bob@x.fr", name: nil)
        #expect(Compose.recipients([a, b]) == "Alice <alice@x.fr>, bob@x.fr")
        let many = (1...10).map { AddressView(addr: "person\($0)@example.org", name: nil) }
        let line = Compose.recipients(many, limit: 60)
        #expect(line.hasPrefix("person1@example.org, person2@example.org"))
        #expect(line.hasSuffix("+8"), "\(line)")
        #expect(Compose.recipients([]) == "")
    }

    @Test func theLinesOfTheStatusBar() {
        #expect(Compose.afterSendLine(nil) == nil)
        #expect(Compose.afterSendLine("archive") == L("After sending: %@", Role.name("archive")))
        #expect(Compose.sendingFlash(seconds: 5) == L("Sending in %lld s · z to cancel", 5))
        #expect(Compose.sendingFlash(seconds: 0) == L("Sending…"))
        #expect(Compose.sentFlash(movedTo: "archive", undo: true) == L("Sent") + " · " + L("filed in %@", Role.name("archive")) + " · " + L("z: undo the filing"))
        #expect(Compose.sentFlash(movedTo: nil, undo: false) == L("Sent"))
        #expect(Compose.sentFlash(movedTo: nil, undo: true) == L("Sent") + " · " + L("z: undo"))
    }

    @Test func theComposerStartsAboveTheQuotation() throws {
        let json = Fixture.draft(replyTo: 10)
        let d = try CoreClient.decoder.decode(DraftView.self, from: JSONSerialization.data(withJSONObject: json))
        let c = Composer(draft: d)
        #expect(c.buffer.cursor == 0)
        #expect(!c.isDirty)
        #expect(c.revision == 1)
    }

    @Test func errorsOfTheCoreAreSaidInTheAppsLanguage() {
        let pending = CoreClient.RPCError(code: ComposeError.bodyPending, message: "x")
        #expect(AppModel.composeErrorFlash(pending) == L("This mail is still downloading: try again in a moment"))
        let cannot = CoreClient.RPCError(code: ComposeError.cannotSend, message: "no [smtp]")
        #expect(AppModel.composeErrorFlash(cannot) == L("Cannot send: %@", "no [smtp]"))
    }
}

/// The reply through the keys of the geek theme (`GeekController`), against
/// a fake core: `r`, the text typed, the saves, ⌘↩, Escape, ⌘⌫, `z`.
@MainActor
struct ComposeControllerTests {
    let core = FakeCore(handler: Fixture.answer)
    let model = AppModel()
    let controller: GeekController

    init() async {
        let defaults = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        model.openFile = { _ in }
        model.addressBook = FixtureAddressBook(cards: [:])
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    func press(_ name: String) {
        var chars = name
        var code: UInt16 = 0
        var mods: NSEvent.ModifierFlags = []
        switch name {
        case "esc": chars = "\u{1b}"; code = KeyCode.escape
        case "ret": chars = "\r"; code = KeyCode.returnKey
        case "cmd-ret": chars = "\r"; code = KeyCode.returnKey; mods = .command
        case "del": chars = "\u{7f}"; code = KeyCode.delete
        case "cmd-del": chars = "\u{7f}"; code = KeyCode.delete; mods = .command
        case "alt-del": chars = "\u{7f}"; code = KeyCode.delete; mods = .option
        case "left": chars = "\u{F702}"; code = KeyCode.left; mods = .function
        case "right": chars = "\u{F703}"; code = KeyCode.right; mods = .function
        case "up": chars = "\u{F700}"; code = KeyCode.up; mods = .function
        case "down": chars = "\u{F701}"; code = KeyCode.down; mods = .function
        case "cmd-down": chars = "\u{F701}"; code = KeyCode.down; mods = [.function, .command]
        case "shift-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .shift]
        case "shift-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .shift]
        case "shift-alt-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .shift, .option]
        case "shift-cmd-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .shift, .command]
        case "shift-cmd-up": chars = "\u{F700}"; code = KeyCode.up; mods = [.function, .shift, .command]
        case "shift-down": chars = "\u{F701}"; code = KeyCode.down; mods = [.function, .shift]
        case "cmd-q": chars = "q"; mods = .command
        default:
            if name.hasPrefix("cmd-") { mods = .command; chars = String(name.dropFirst(4)) }
            if name.hasPrefix("ctrl-") { mods = .control; chars = String(name.dropFirst(5)) }
        }
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        _ = controller.handle(event)
    }

    func type(_ text: String) {
        for c in text { press(String(c)) }
    }

    /// `r` on the first mail, and the composer open.
    func reply() async {
        press("r")
        _ = await eventually { model.composer != nil }
    }

    @Test func rOpensAReplyToTheMailUnderTheCursor() async throws {
        await reply()
        let calls = core.calls("create_draft")
        #expect(calls.count == 1)
        #expect(calls.first?["reply_to"] as? Int == 10)
        let header = try #require(calls.first?["quote_header"] as? String)
        #expect(header.contains("Sender 1 <sender1@example.org>"), "\(header)")
        let c = try #require(model.composer)
        #expect(c.draft.subject == "Re: Subject 1")
        #expect(c.buffer.cursor == 0)
        // `r` again while writing: a letter of the text, not a second draft.
        press("r")
        #expect(core.calls("create_draft").count == 1)
        #expect(model.composer?.buffer.text.hasPrefix("r") == true)
    }

    @Test func lettersAreTextNotCommands() async {
        await reply()
        type("djk;m")
        #expect(model.composer?.buffer.text.hasPrefix("djk;m\n") == true)
        #expect(core.calls("move_thread").isEmpty)
        #expect(controller.overlay == .none)
        #expect(model.selectedThreadId == model.threads.first?.id)
    }

    @Test func editingKeys() async {
        await reply()
        type("Merci")
        press("ret")
        type("Bye")
        press("del")
        press("left")
        press("alt-del")
        #expect(model.composer?.buffer.text.hasPrefix("Merci\ny\n") == true)
        press("up")
        #expect(model.composer?.buffer.position.line == 0)
        press("cmd-down")
        #expect(model.composer?.buffer.cursor == model.composer?.buffer.text.count)
        press("ctrl-a")
        #expect(model.composer?.buffer.position.column == 0)
        // ⌘V pastes several lines as they are.
        controller.pasteboard.clearContents()
        controller.pasteboard.setString("one\ntwo", forType: .string)
        press("cmd-v")
        #expect(model.composer?.buffer.text.contains("one\ntwo") == true)
        // The menus' shortcuts go on to AppKit.
        let q = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
            characters: "q", charactersIgnoringModifiers: "q", isARepeat: false, keyCode: 0)!
        #expect(controller.handle(q) == false)
    }

    @Test func shiftWithTheArrowsSelects() async {
        await reply()
        type("Merci Alice")
        press("shift-alt-left")
        #expect(model.composer?.buffer.selectedText == "Alice")
        press("shift-left")
        #expect(model.composer?.buffer.selectedText == " Alice")
        // ← ends the selection at its start, without moving further.
        press("left")
        #expect(model.composer?.buffer.selection == nil)
        #expect(model.composer?.buffer.cursor == 5)
        press("shift-right")
        press("shift-right")
        #expect(model.composer?.buffer.selectedText == " A")
        // → at its end.
        press("right")
        #expect(model.composer?.buffer.cursor == 7)
        // ⇧⌘←: to the start of the line; typing replaces it.
        press("shift-cmd-left")
        #expect(model.composer?.buffer.selectedText == "Merci A")
        type("Bonjour a")
        #expect(model.composer?.buffer.text.hasPrefix("Bonjour alice\n") == true)
        // ⇧↓ goes down a line, ⇧⌘↑ to the start of the text.
        press("shift-down")
        #expect(model.composer?.buffer.selectedText.hasSuffix("\n") == true)
        press("shift-cmd-up")
        #expect(model.composer?.buffer.selectedText == "Bonjour a")
    }

    @Test func copyCutAndSelectAll() async {
        await reply()
        type("Merci")
        press("shift-alt-left")
        press("cmd-c")
        #expect(controller.pasteboard.string(forType: .string) == "Merci")
        #expect(model.composer?.buffer.text.hasPrefix("Merci") == true)
        press("cmd-x")
        #expect(model.composer?.buffer.text.hasPrefix("\n\n") == true)
        #expect(controller.pasteboard.string(forType: .string) == "Merci")
        press("cmd-v")
        #expect(model.composer?.buffer.text.hasPrefix("Merci\n") == true)
        press("cmd-a")
        #expect(model.composer?.buffer.selectedText == model.composer?.buffer.text)
        // Delete empties the whole reply; ⌘⌫ still abandons it.
        press("del")
        #expect(model.composer?.buffer.text == "")
        // Nothing selected: ⌘C copies nothing.
        controller.pasteboard.clearContents()
        press("cmd-c")
        #expect(controller.pasteboard.string(forType: .string) == nil)
    }

    @Test func aShiftClickExtendsTheSelection() async {
        await reply()
        type("Merci")
        controller.clickCompose(utf16: 2)
        #expect(model.composer?.buffer.cursor == 2)
        controller.clickCompose(utf16: 5, extend: true)
        #expect(model.composer?.buffer.selectedText == "rci")
        controller.clickCompose(utf16: 0)
        #expect(model.composer?.buffer.selection == nil)
    }

    /// Kerlig and the like: they select through the accessibility API, and
    /// write the corrected text back in place of the selection.
    @Test func anotherAppSelectsAndReplaces() async {
        await reply()
        type("Merci beaucop")
        controller.selectCompose(utf16: NSRange(location: 6, length: 7))
        #expect(model.composer?.buffer.selectedText == "beaucop")
        controller.replaceComposeSelection("beaucoup")
        #expect(model.composer?.buffer.text.hasPrefix("Merci beaucoup\n") == true)
        #expect(model.composer?.buffer.cursor == 14)
        // Saved like typing.
        #expect(await eventually { core.calls("update_draft").last?["body"] as? String == model.composer?.buffer.text })
        // An empty text deletes the selection, and nothing without one.
        controller.selectCompose(utf16: NSRange(location: 0, length: 6))
        controller.replaceComposeSelection("")
        #expect(model.composer?.buffer.text.hasPrefix("beaucoup\n") == true)
        controller.replaceComposeSelection("")
        #expect(model.composer?.buffer.text.hasPrefix("beaucoup\n") == true)
    }

    @Test func theTextIsSavedAfterAPause() async {
        await reply()
        type("Merci")
        #expect(core.calls("update_draft").isEmpty)
        #expect(await eventually { core.calls("update_draft").count == 1 })
        let save = core.calls("update_draft")[0]
        #expect(save["revision"] as? Int == 1)
        #expect((save["body"] as? String)?.hasPrefix("Merci\n\n") == true)
        #expect(await eventually { model.composer?.revision == 2 && model.composer?.isDirty == false })
        // The next save sends the new revision.
        type("!")
        #expect(await eventually { core.calls("update_draft").count == 2 })
        #expect(core.calls("update_draft")[1]["revision"] as? Int == 2)
    }

    @Test func commandReturnSendsAndZCancelsDuringTheDelay() async {
        await reply()
        type("Merci")
        press("cmd-ret")
        #expect(await eventually { model.composer == nil })
        let send = core.calls("send_draft")
        #expect(send.count == 1)
        #expect(send.first?["id"] as? Int == 901)
        #expect((send.first?["body"] as? String)?.hasPrefix("Merci") == true)
        #expect(model.flash == L("Sending in %lld s · z to cancel", 5) || model.flash == L("Sending in %lld s · z to cancel", 4))
        #expect(model.undoStack.last?.cancelSend == 901)
        // `z`: the mail does not leave, the reply is open again.
        press("z")
        #expect(await eventually { model.composer != nil })
        #expect(core.calls("cancel_send").first?["id"] as? Int == 901)
        #expect(model.composer?.buffer.text == "Merci")
        #expect(model.flash == L("Sending cancelled: the reply is open again"))
    }

    @Test func tooLateToCancel() async {
        core.handler = { method, params in
            method == "cancel_send" ? CoreClient.RPCError(code: ComposeError.sendStarted, message: "too late") : Fixture.answer(method, params)
        }
        await reply()
        press("cmd-ret")
        _ = await eventually { model.composer == nil }
        press("z")
        #expect(await eventually { model.flash == L("Too late: the mail is leaving") })
        #expect(model.composer == nil)
    }

    @Test func escapeKeepsTheDraftAndRResumesIt() async {
        await reply()
        type("Merci")
        press("esc")
        #expect(await eventually { model.composer == nil })
        #expect(model.flash == L("Reply kept as a draft · r to resume"))
        // Saved at once, without waiting for the pause.
        #expect(await eventually { core.calls("update_draft").count == 1 })
        // Keys are commands again.
        press("j")
        #expect(model.selectedThreadId == model.threads[1].id)
    }

    @Test func commandDeleteAbandonsAndZRestores() async {
        await reply()
        type("Mon texte")
        press("cmd-del")
        #expect(await eventually { model.composer == nil })
        #expect(core.calls("discard_draft").first?["id"] as? Int == 901)
        #expect(model.flash == L("Reply abandoned · z to restore it"))
        press("z")
        #expect(await eventually { model.composer != nil })
        let restore = core.calls("create_draft").last
        #expect(restore?["reply_to"] as? Int == 10)
        #expect((restore?["body"] as? String)?.hasPrefix("Mon texte") == true)
    }

    @Test func theEventsOfTheSending() async {
        await reply()
        press("cmd-ret")
        _ = await eventually { model.composer == nil }
        // Accepted: the step that cancelled the sending becomes the one
        // that undoes the filing.
        model.draftEvent("DraftSent", ["draft_id": .int(901), "moved_to": .string("archive"), "undo": .string("5:9")])
        #expect(model.flash == L("Sent") + " · " + L("filed in %@", Role.name("archive")) + " · " + L("z: undo the filing"))
        #expect(model.undoStack.last?.tokens == ["5:9"])
        #expect(!model.undoStack.contains { $0.cancelSend == 901 })
        #expect(model.sending.isEmpty)
        press("z")
        #expect(await eventually { core.calls("undo").first?["token"] as? String == "5:9" })
        // Refused for good, or in part.
        model.draftEvent("SendFailed", ["draft_id": .int(902), "reason": .string("550 no such user")])
        #expect(model.flash == L("Not sent: %@ · r on the mail to reopen the reply", "550 no such user"))
        model.draftEvent("SendPartial", ["draft_id": .int(903), "refused": .strings(["bad@x.fr (550 unknown)"])])
        #expect(model.flash == L("Sent, but the server refused: %@", "bad@x.fr (550 unknown)"))
    }

    @Test func aMailThatCannotBeAnsweredSaysWhy() async {
        core.handler = { method, params in
            method == "create_draft" ? CoreClient.RPCError(code: ComposeError.cannotSend, message: "no [smtp]") : Fixture.answer(method, params)
        }
        press("r")
        #expect(await eventually { model.flash == L("Cannot send: %@", "no [smtp]") })
        #expect(model.composer == nil)
    }

    @Test func replyToTheSenderAndForwardAreNotThereYet() {
        press("R")
        #expect(model.flash == L("Not available yet: %@", L("reply to the sender only")))
        press("f")
        #expect(model.flash == L("Not available yet: %@", L("forward")))
    }

    @Test func oneColumnShowsTheReplyInTheWholeWindow() async {
        controller.columns = 1
        await reply()
        #expect(await eventually { controller.reading })
        press("esc")
        _ = await eventually { model.composer == nil }
        // Back to the mail, still in the whole window.
        #expect(controller.reading)
    }
}

/// The text view of the reply, as other apps see it through the
/// accessibility API and the Services (`ComposerNSTextView`).
@MainActor
struct ComposerAccessibilityTests {
    @Test func theSelectionIsReadAndWhatIsWrittenGoesToTheReply() {
        let view = ComposerNSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 200))
        view.isEditable = false
        view.isSelectable = true
        view.string = "Merci beaucop"
        view.setSelectedRange(NSRange(location: 6, length: 7))
        #expect(view.accessibilitySelectedText() == "beaucop")
        var replaced: [String] = []
        var selected: [NSRange] = []
        view.onReplaceSelection = { replaced.append($0) }
        view.onSelect = { selected.append($0) }
        // Not editable, yet the setters are allowed: they go to the reply.
        #expect(view.isAccessibilitySelectorAllowed(#selector(NSView.setAccessibilitySelectedText(_:))))
        #expect(view.isAccessibilitySelectorAllowed(#selector(NSView.setAccessibilityValue(_:))))
        view.setAccessibilitySelectedText("beaucoup")
        view.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 5))
        #expect(replaced == ["beaucoup"])
        #expect(selected == [NSRange(location: 0, length: 5)])
        // The whole value: everything selected, then replaced.
        view.setAccessibilityValue("Tout neuf")
        #expect(selected.last == NSRange(location: 0, length: 13))
        #expect(replaced.last == "Tout neuf")
        // A Service sends the selection and gets its answer back.
        #expect(view.validRequestor(forSendType: .string, returnType: .string) as? ComposerNSTextView === view)
        let pb = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        pb.clearContents()
        pb.setString("corrigé", forType: .string)
        #expect(view.readSelection(from: pb, type: .string))
        #expect(replaced.last == "corrigé")
    }
}
