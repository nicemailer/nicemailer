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

    func draft(target: String?, chosen: String?) throws -> DraftView {
        var json = Fixture.draft(replyTo: 10)
        json["after_send_target"] = target ?? NSNull()
        json["after_send"] = chosen ?? NSNull()
        return try CoreClient.decoder.decode(DraftView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    /// The filing that applies (`reponse.md`, section 8): the one chosen
    /// with ⇧⌘↩ wins over the preselection, `stay` leaves the conversation,
    /// and nothing applies where the core offers no filing.
    @Test func theFilingChosenWinsOverThePreselection() throws {
        #expect(Compose.afterSend(try draft(target: "archive", chosen: nil)) == "archive")
        #expect(Compose.afterSend(try draft(target: "archive", chosen: "waiting")) == "waiting")
        #expect(Compose.afterSend(try draft(target: "archive", chosen: "stay")) == nil)
        #expect(Compose.afterSend(try draft(target: nil, chosen: "todo")) == nil)
        #expect(Compose.afterSendLine(try draft(target: nil, chosen: nil)) == nil)
        #expect(Compose.afterSendLine(try draft(target: "archive", chosen: nil)) == L("After sending: %@", Role.name("archive")))
        #expect(Compose.afterSendLine(try draft(target: "archive", chosen: "todo")) == L("After sending: %@", Role.name("todo")))
        #expect(Compose.afterSendLine(try draft(target: "archive", chosen: "stay")) == L("After sending: stays where it is"))
    }

    /// The answers of ⇧⌘↩: the three filings, then the trial option, the
    /// one ⌘↩ would apply marked.
    @Test func theChoicesOfSendThen() throws {
        let choices = Compose.sendChoices(try draft(target: "waiting", chosen: nil))
        #expect(choices.map(\.key) == ["a", "w", "t", "s"])
        #expect(choices.map(\.afterSend) == ["archive", "waiting", "todo", "stay"])
        #expect(choices.filter(\.current).map(\.key) == ["w"])
        #expect(choices.last?.label == L("leave it where it is (trial)"))
        let chosen = Compose.sendChoices(try draft(target: "waiting", chosen: "stay"))
        #expect(chosen.filter(\.current).map(\.key) == ["s"])
    }

    /// A draft of `kind`, with no conversation to file: a new mail, a
    /// forward.
    func ownDraft(kind: String, chosen: String?) throws -> DraftView {
        var json = Fixture.draft(replyTo: 10)
        json["kind"] = kind
        json["reply_to"] = NSNull()
        json["forward_of"] = kind == "forward" ? 10 : NSNull()
        json["thread_id"] = NSNull()
        json["after_send_target"] = NSNull()
        json["after_send"] = chosen ?? NSNull()
        return try CoreClient.decoder.decode(DraftView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    /// A new mail files itself (`reponse.md`, section 8, "Nouveau mail"):
    /// ⇧⌘↩ offers the three folders, Sent being where it stays; a forward
    /// files nothing at all.
    @Test func aNewMailFilesTheMailWritten() throws {
        let fresh = try ownDraft(kind: "new", chosen: nil)
        #expect(Compose.offersFiling(fresh) && Compose.filesItself(fresh))
        // Nothing chosen: it stays in Sent, and the headers say nothing.
        #expect(Compose.afterSend(fresh) == nil)
        #expect(Compose.afterSendLine(fresh) == nil)
        let choices = Compose.sendChoices(fresh)
        #expect(choices.map(\.afterSend) == ["archive", "waiting", "todo", "stay"])
        #expect(choices.filter(\.current).map(\.key) == ["s"])
        #expect(choices.last?.label == L("leave it in Sent"))
        // Chosen: it applies, and the headers show it.
        let waiting = try ownDraft(kind: "new", chosen: "waiting")
        #expect(Compose.afterSend(waiting) == "waiting")
        #expect(Compose.afterSendLine(waiting) == L("After sending: %@", Role.name("waiting")))
        #expect(Compose.sendChoices(waiting).filter(\.current).map(\.key) == ["w"])
        // `stay` chosen on a new mail: nothing to say, it is the default.
        #expect(Compose.afterSendLine(try ownDraft(kind: "new", chosen: "stay")) == nil)
        // A forward: nothing is offered, whatever a client stored.
        let forward = try ownDraft(kind: "forward", chosen: nil)
        #expect(!Compose.offersFiling(forward) && !Compose.filesItself(forward))
        #expect(Compose.afterSend(try ownDraft(kind: "forward", chosen: "todo")) == nil)
    }

    /// Tab goes through To, Cc, (Bcc when shown), Subject and the text,
    /// turning around; ⇧Tab the other way.
    @Test func tabGoesFromFieldToField() throws {
        var c = Composer(draft: try draft(target: "archive", chosen: nil))
        #expect(c.focus == .body)
        #expect(c.fields == [.to, .cc, .subject, .body])
        #expect(c.field(after: .body, backward: false) == .to)
        #expect(c.field(after: .to, backward: true) == .body)
        #expect(c.field(after: .cc, backward: false) == .subject)
        c.showsBcc = true
        #expect(c.field(after: .cc, backward: false) == .bcc)
    }

    /// An address is in one line only; ⌫ on an empty line takes the last
    /// one back, selected.
    @Test func addressesAreAddedOnceAndTakenBack() throws {
        var c = Composer(draft: try draft(target: "archive", chosen: nil))
        let sender = c.headers.to[0]
        let already = c.add([AddressView(addr: sender.addr.uppercased(), name: nil), AddressView(addr: "new@x.fr", name: nil)], to: .cc)
        #expect(already.map(\.addr) == [sender.addr.uppercased()])
        #expect(c.headers.cc.map(\.addr) == ["new@x.fr"])
        #expect(c.isDirty)
        let took = c.takeBackLast(of: .cc)
        #expect(took)
        #expect(c.line.text == "new@x.fr")
        #expect(c.headers.cc.isEmpty)
        let none = c.takeBackLast(of: .bcc)
        #expect(!none)
    }

    /// The address taken back is selected (`reponse.md`, section 6,
    /// "Corriger"): one ⌫ wipes it, typing replaces it, an arrow turns it
    /// into an ordinary text to correct. Any address can be taken, not
    /// only the last, which a click on it does.
    @Test func theAddressTakenBackIsSelected() throws {
        var c = Composer(draft: try draft(target: "archive", chosen: nil))
        c.add([AddressView(addr: "a@x.fr", name: nil), AddressView(addr: "b@x.fr", name: "Bee")], to: .cc)
        var took = c.takeBackLast(of: .cc)
        #expect(took && c.line.selectedAll)
        // One Delete, and nothing is left of it.
        c.line.deleteBackward()
        #expect(c.line.text.isEmpty)
        // Typing replaces it whole.
        took = c.takeBackLast(of: .cc)
        #expect(took)
        #expect((c.line.text, c.line.selectedAll) == ("a@x.fr", true))
        c.line.insert("z@x.fr")
        #expect(c.line.text == "z@x.fr")
        // An arrow keeps it, to correct it letter by letter.
        c.add([AddressView(addr: "c@x.fr", name: nil)], to: .cc)
        took = c.takeBackLast(of: .cc)
        #expect(took)
        c.line.moveRight()
        #expect((c.line.text, c.line.selectedAll, c.line.cursor) == ("c@x.fr", false, 6))
        // By index: the first of the line, the others untouched; a name
        // is left out, the address alone comes back.
        c.line = LineEdit()
        c.add([AddressView(addr: "d@x.fr", name: nil), AddressView(addr: "e@x.fr", name: "Eve")], to: .to)
        let all = c.headers.to.map(\.addr)
        took = c.takeBack(0, of: .to)
        #expect(took)
        #expect((c.line.text, c.line.selectedAll) == (all[0], true))
        #expect(c.headers.to.map(\.addr) == Array(all.dropFirst()))
        let none = c.takeBack(9, of: .to)
        #expect(!none)
    }

    /// `update_draft` carries the body, and the headers only when edited.
    @Test func onlyWhatChangedIsSaved() throws {
        var c = Composer(draft: try draft(target: "archive", chosen: nil))
        #expect(Set(Compose.saveParams(c).keys) == ["id", "revision"])
        c.buffer.insert("x")
        #expect(Set(Compose.saveParams(c).keys) == ["id", "revision", "body"])
        c.saved = c.buffer.text
        c.headers.bcc = [AddressView(addr: "b@x.fr", name: "B")]
        c.headers.subject = "Autre"
        let p = Compose.saveParams(c)
        #expect(Set(p.keys) == ["id", "revision", "bcc", "subject"])
        #expect((p["bcc"] as? [[String: Any]])?.first?["addr"] as? String == "b@x.fr")
    }

    func decode(_ json: [String: Any]) throws -> DraftView {
        try CoreClient.decoder.decode(DraftView.self, from: JSONSerialization.data(withJSONObject: json))
    }

    /// A proposal (`prepare_draft`, id 0) is written with what it was
    /// opened as and the text typed: the mail answered for a reply, the
    /// mail forwarded for a forward, the account for a new mail.
    @Test func aProposalIsWrittenWithWhatItWasOpenedAs() throws {
        let proposal = try decode(Fixture.opened("prepare_draft", Fixture.draft(replyTo: 10)))
        var c = Composer(draft: proposal, kind: .replySender, quoteHeader: "On Nov 7, Sender 1 wrote:")
        #expect(!c.isWritten)
        c.buffer.insert("Merci ")
        let p = Compose.createParams(c)
        #expect(p["kind"] as? String == "reply_sender")
        #expect(p["reply_to"] as? Int == 10)
        #expect(p["quote_header"] as? String == "On Nov 7, Sender 1 wrote:")
        #expect(p["body"] as? String == c.buffer.text)
        #expect(p["account"] == nil)
        var forward = Fixture.draft(replyTo: 10)
        forward["kind"] = "forward"
        forward["reply_to"] = NSNull()
        forward["forward_of"] = 10
        let f = Compose.createParams(Composer(draft: try decode(Fixture.opened("prepare_draft", forward))))
        #expect(f["kind"] as? String == "forward")
        #expect(f["reply_to"] as? Int == 10)
        var new = Fixture.draft(replyTo: 10)
        new["kind"] = "new"
        new["reply_to"] = NSNull()
        let n = Compose.createParams(Composer(draft: try decode(Fixture.opened("prepare_draft", new))))
        #expect(n["kind"] as? String == "new")
        #expect(n["account"] as? Int == 1)
        #expect(n["reply_to"] == nil)
    }

    /// Written, the composer takes the draft's id, revision and files; the
    /// text and the headers typed meanwhile stay, still to save.
    @Test func writtenTheComposerKeepsWhatWasTyped() throws {
        var forward = Fixture.draft(replyTo: 10)
        forward["kind"] = "forward"
        forward["attachments"] = [["id": 77, "filename": "devis.pdf", "mime_type": "application/pdf", "size": 8]]
        var c = Composer(draft: try decode(Fixture.opened("prepare_draft", forward)))
        #expect(c.attachments.map(\.id) == [-1])
        c.buffer.insert("Voici")
        let typed = c.buffer.text
        // The core wrote "Voi": the start of the text, typed before the call.
        c.written(try decode(forward.merging(["body": "Voi"]) { $1 }))
        #expect(c.isWritten && c.draft.id == 901 && c.revision == 1)
        #expect(c.attachments.map(\.id) == [77])
        #expect(c.buffer.text == typed)
        // "ci" typed while `create_draft` ran: saved next.
        #expect(c.isDirty)
    }

    /// Escape on a proposal left as it was: nothing was ever written.
    @Test func escapeOnAProposalSaysNothingWasKept() throws {
        let c = Composer(draft: try decode(Fixture.opened("prepare_draft", Fixture.draft(replyTo: 10))))
        #expect(Compose.keptFlash(c) == L("Nothing written: no draft kept"))
        var typed = c
        typed.buffer.insert("x")
        #expect(Compose.keptFlash(typed) == L("Reply kept in Drafts · r to resume"))
        let written = Composer(draft: try decode(Fixture.draft(replyTo: 10)))
        #expect(Compose.keptFlash(written) == L("Reply kept in Drafts · r to resume"))
        // An address still typed in a line is kept on the way out.
        var pending = c
        pending.focus = .cc
        pending.line = LineEdit("bob@x.fr")
        #expect(Compose.keptFlash(pending) == L("Reply kept in Drafts · r to resume"))
    }

    /// "Send from" (`reponse.md`, section 4 ter): every address of every
    /// account that can send, keyed 1 to 9, the current one marked; the
    /// sender changed goes with the next save, with its account.
    @Test func theSenderIsChosenAmongTheAccountsThatCanSend() throws {
        let senders = try CoreClient.decoder.decode([SenderView].self, from: JSONSerialization.data(withJSONObject: Fixture.senders))
        var c = Composer(draft: try draft(target: "archive", chosen: nil))
        let choices = Compose.senderChoices(senders, current: c.headers)
        #expect(choices.map(\.key) == ["1", "2", "3"])
        #expect(choices.filter(\.current).map(\.sender.addr) == ["me@example.org"])
        #expect(choices[1].label == "Me at work <me@work.example> · work")
        #expect(choices[2].label == "sales@work.example · work")
        #expect(Compose.fromLine(c.headers, senders: senders) == "Me <me@example.org> · perso")
        #expect(Compose.fromLine(c.headers, senders: []) == "Me <me@example.org>")
        c.headers.from = AddressView(addr: "sales@work.example", name: nil)
        c.headers.sendAccountId = 2
        #expect(c.isDirty)
        let p = Compose.saveParams(c)
        #expect(Set(p.keys) == ["id", "revision", "from"])
        let from = try #require(p["from"] as? [String: Any])
        #expect(from["account_id"] as? Int == 2)
        #expect(from["addr"] as? String == "sales@work.example")
        #expect(Compose.senderChoices(senders, current: c.headers).filter(\.current).map(\.key) == ["3"])
    }

    /// A reply the core started from another account than the mail's (it
    /// cannot send) says so; a reply from its own account, or a new mail,
    /// says nothing.
    @Test func aReplyFromAnotherAccountSaysSo() throws {
        let senders = try CoreClient.decoder.decode([SenderView].self, from: JSONSerialization.data(withJSONObject: Fixture.senders))
        #expect(Compose.otherSenderFlash(try draft(target: "archive", chosen: nil), senders: senders) == nil)
        var json = Fixture.draft(replyTo: 10)
        json["account_id"] = 7
        json["send_account_id"] = 2
        json["from"] = ["addr": "me@work.example", "name": "Me at work"]
        let d = try CoreClient.decoder.decode(DraftView.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(Compose.otherSenderFlash(d, senders: senders) == L("This account cannot send: from %@ · ⌥⌘F to change", "Me at work <me@work.example> · work"))
        json["kind"] = "new"
        let n = try CoreClient.decoder.decode(DraftView.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(Compose.otherSenderFlash(n, senders: senders) == nil)
    }

    @Test func theLinesOfTheStatusBar() {
        #expect(Compose.sendingFlash(seconds: 5) == L("Sending in %lld s · z to cancel", 5))
        #expect(Compose.sendingFlash(seconds: 0) == L("Sending…"))
        #expect(Compose.sentFlash(movedTo: "archive", undo: true) == L("Sent") + " · " + L("filed in %@", Role.name("archive")) + " · " + L("z: undo the filing"))
        #expect(Compose.sentFlash(movedTo: nil, undo: false) == L("Sent"))
        #expect(Compose.sentFlash(movedTo: nil, undo: true) == L("Sent") + " · " + L("z: undo"))
        // A new mail's own filing: said the same way, but `z` only offers
        // the decisions back (section 8, "Nouveau mail").
        #expect(Compose.sentFlash(movedTo: nil, filing: "waiting", undo: true)
            == L("Sent") + " · " + L("filed in %@", Role.name("waiting")) + " · " + L("z: undo"))
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
        let defaults = TestDefaults.make()
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
        case "shift-cmd-ret": chars = "\r"; code = KeyCode.returnKey; mods = [.command, .shift]
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
        case "cmd-up": chars = "\u{F700}"; code = KeyCode.up; mods = [.function, .command]
        case "cmd-left": chars = "\u{F702}"; code = KeyCode.left; mods = [.function, .command]
        case "shift-alt-down": chars = "\u{F701}"; code = KeyCode.down; mods = [.function, .shift, .option]
        case "shift-alt-right": chars = "\u{F703}"; code = KeyCode.right; mods = [.function, .shift, .option]
        case "cmd-q": chars = "q"; mods = .command
        case "cmd-A": chars = "A"; mods = [.command, .shift]
        case "tab": chars = "\t"; code = KeyCode.tab
        case "shift-tab": chars = "\u{19}"; code = KeyCode.tab; mods = .shift
        case "alt-cmd-b": chars = "b"; code = 11; mods = [.command, .option]
        case "alt-cmd-f": chars = "f"; code = 3; mods = [.command, .option]
        case "cmd-v": chars = "v"; mods = .command
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
        // Proposed, not written (`reponse.md`, section 5).
        #expect(core.calls("create_draft").isEmpty)
        #expect(model.composer?.isWritten == false)
        let calls = core.calls("prepare_draft")
        #expect(calls.count == 1)
        #expect(calls.first?["reply_to"] as? Int == 10)
        let header = try #require(calls.first?["quote_header"] as? String)
        #expect(header.contains("Sender 1 <sender1@example.org>"), "\(header)")
        let c = try #require(model.composer)
        #expect(c.draft.subject == "Re: Subject 1")
        #expect(c.buffer.cursor == 0)
        // `r` again while writing: a letter of the text, not a second draft.
        press("r")
        #expect(core.calls("prepare_draft").count == 1)
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
        // Saved like typing: the first save writes the draft with it.
        let saved = { (core.calls("update_draft").last ?? core.calls("create_draft").last)?["body"] as? String }
        #expect(await eventually { saved() == model.composer?.buffer.text })
        // An empty text deletes the selection, and nothing without one.
        controller.selectCompose(utf16: NSRange(location: 0, length: 6))
        controller.replaceComposeSelection("")
        #expect(model.composer?.buffer.text.hasPrefix("beaucoup\n") == true)
        controller.replaceComposeSelection("")
        #expect(model.composer?.buffer.text.hasPrefix("beaucoup\n") == true)
    }

    /// The first pause writes the draft proposed, with the text typed
    /// (`create_draft`); the next ones save it (`update_draft`).
    @Test func theTextIsSavedAfterAPause() async {
        await reply()
        type("Merci")
        #expect(core.calls("create_draft").isEmpty)
        #expect(await eventually { core.calls("create_draft").count == 1 })
        let write = core.calls("create_draft")[0]
        #expect(write["kind"] as? String == "reply_all")
        #expect(write["reply_to"] as? Int == 10)
        #expect((write["body"] as? String)?.hasPrefix("Merci\n\n") == true)
        #expect(await eventually { model.composer?.isWritten == true && model.composer?.isDirty == false })
        #expect(model.composer?.draft.id == 901)
        #expect(model.composer?.revision == 1)
        #expect(core.calls("update_draft").isEmpty)
        // The next save sends the revision of the draft written.
        type("!")
        #expect(await eventually { core.calls("update_draft").count == 1 })
        #expect(core.calls("update_draft")[0]["revision"] as? Int == 1)
        #expect(await eventually { model.composer?.revision == 2 })
        #expect(core.calls("create_draft").count == 1)
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
        // Said once `openComposer` is done, a moment after the reply is
        // back: waited for, not read at once.
        #expect(await eventually { model.flash == L("Sending cancelled: the reply is open again") })
    }

    /// ⇧⌘↩ asks where the conversation goes, then sends with that choice;
    /// Escape goes back to the text without sending.
    @Test func shiftCommandReturnChoosesTheFilingThenSends() async {
        await reply()
        type("Merci")
        press("shift-cmd-ret")
        #expect(controller.activeChoice == .sendThen)
        #expect(controller.modeLabel == "ASK")
        // The current filing marked; the trial option apart.
        let entries = controller.choiceEntries(.sendThen)
        #expect(entries.map(\.key) == ["a", "w", "t", "s", "esc"])
        #expect(entries.first?.detail == "•")
        #expect(entries[3].gapBefore)
        // Escape: back to the text, nothing sent, letters are text again.
        press("esc")
        #expect(controller.activeChoice == nil)
        #expect(model.composer != nil)
        type("!")
        #expect(model.composer?.buffer.text.hasPrefix("Merci!") == true)
        #expect(core.calls("send_draft").isEmpty)
        // `w`: sent, the conversation to go to Waiting.
        press("shift-cmd-ret")
        press("w")
        #expect(await eventually { model.composer == nil })
        let send = core.calls("send_draft")
        #expect(send.count == 1)
        #expect(send.first?["after_send"] as? String == "waiting")
        #expect((send.first?["body"] as? String)?.hasPrefix("Merci!") == true)
    }

    /// ⌘↩ sends without a choice: the core keeps the one made earlier, or
    /// applies its preselection.
    @Test func commandReturnSendsWithoutAChoice() async {
        await reply()
        press("cmd-ret")
        #expect(await eventually { core.calls("send_draft").count == 1 })
        #expect(core.calls("send_draft").first?["after_send"] == nil)
    }

    /// A new mail files itself (`reponse.md`, section 8, "Nouveau mail"):
    /// ⇧⌘↩ asks where the mail written goes, Sent being the preselection,
    /// and the choice is sent with it.
    @Test func shiftCommandReturnFilesANewMail() async {
        core.handler = { method, params in
            guard method == "create_draft" || method == "prepare_draft" else { return Fixture.answer(method, params) }
            var d = Fixture.draft(replyTo: 10)
            d["kind"] = "new"
            d["reply_to"] = NSNull()
            d["thread_id"] = NSNull()
            d["after_send_target"] = NSNull()
            return ["draft": Fixture.opened(method, d), "reused": false]
        }
        press("c")
        #expect(await eventually { model.composer != nil })
        type("Bonjour")
        press("shift-cmd-ret")
        #expect(controller.activeChoice == .sendThen)
        // The mail, not a conversation; Sent is where it stays by default.
        #expect(controller.choiceTitle(.sendThen) == L("Send, then file this mail in"))
        let entries = controller.choiceEntries(.sendThen)
        #expect(entries.map(\.key) == ["a", "w", "t", "s", "esc"])
        #expect(entries[3].label == L("leave it in Sent"))
        #expect(entries[3].detail == "•")
        #expect(entries.first?.detail == nil)
        // `w`: sent, the mail to go to Waiting.
        press("w")
        #expect(await eventually { core.calls("send_draft").count == 1 })
        #expect(core.calls("send_draft").first?["after_send"] as? String == "waiting")
    }

    /// A forward files nothing (`reponse.md`, section 8): ⇧⌘↩ says so and
    /// sends nothing.
    @Test func aForwardHasNothingToFile() async {
        core.handler = { method, params in
            guard method == "create_draft" || method == "prepare_draft" else { return Fixture.answer(method, params) }
            var d = Fixture.draft(replyTo: 10)
            d["kind"] = "forward"
            d["reply_to"] = NSNull()
            d["forward_of"] = 10
            d["thread_id"] = NSNull()
            d["after_send_target"] = NSNull()
            return ["draft": Fixture.opened(method, d), "reused": false]
        }
        press("F")
        #expect(await eventually { model.composer != nil })
        press("shift-cmd-ret")
        #expect(controller.activeChoice == nil)
        #expect(model.flash == L("A forward files nothing: ⌘↩ to send"))
        #expect(core.calls("send_draft").isEmpty)
    }

    /// No filing offered (a conversation in Archive, Newsletter…): ⇧⌘↩
    /// says so and sends nothing.
    @Test func nothingToChooseOutsideTheWorkFolders() async {
        core.handler = { method, params in
            guard method == "create_draft" || method == "prepare_draft" else { return Fixture.answer(method, params) }
            var d = Fixture.draft(replyTo: params["reply_to"] as? Int ?? 10)
            d["after_send_target"] = NSNull()
            return ["draft": Fixture.opened(method, d), "reused": false]
        }
        await reply()
        press("shift-cmd-ret")
        #expect(controller.activeChoice == nil)
        #expect(model.flash == L("This conversation stays where it is: nothing to choose · ⌘↩ to send"))
        #expect(core.calls("send_draft").isEmpty)
    }

    /// The recipient lines (`reponse.md`, section 6): typed text becomes
    /// an address on Return or a comma, the suggestions are chosen with the
    /// arrows, ⌫ takes the last one back, what is not an address stays.
    @Test func recipientsAreTypedAndSuggested() async {
        await reply()
        press("shift-tab")
        press("shift-tab")
        press("shift-tab")
        #expect(model.composer?.focus == .to)
        // Letters are text of the line, not commands nor the body.
        type("al")
        #expect(model.composer?.line.text == "al")
        #expect(await eventually { model.composer?.suggestions.map(\.addr) == ["alice@example.org", "albert@example.org"] })
        #expect(model.composer?.suggestionIndex == 0)
        press("down")
        press("ret")
        #expect(model.composer?.headers.to.last?.addr == "albert@example.org")
        #expect(model.composer?.line.text == "")
        // A typed address, ended by a comma.
        type("carl@example.org,")
        #expect(await eventually { model.composer?.headers.to.last?.addr == "carl@example.org" })
        // Not an address: it stays, said in the status bar.
        type("nope")
        press("ret")
        #expect(await eventually { model.flash == L("Not an address: %@", "nope") })
        #expect(model.composer?.line.text == "nope")
        // ⌫ until the line is empty, then once more: carl comes back,
        // selected, and one more ⌫ removes it whole.
        for _ in 0..<5 { press("del") }
        #expect(model.composer?.line.text == "carl@example.org")
        #expect(model.composer?.line.selectedAll == true)
        #expect(model.composer?.headers.to.last?.addr == "albert@example.org")
        press("del")
        #expect(model.composer?.line.text == "")
        #expect(model.composer?.line.selectedAll == false)
        // Saved with the next pause: the whole To line.
        #expect(await eventually { core.calls("update_draft").contains { ($0["to"] as? [[String: Any]])?.count == 2 } })
        #expect(core.calls("update_draft").allSatisfy { $0["body"] == nil })
    }

    /// A click on an address takes that one back into its line, selected,
    /// wherever it sits (`reponse.md`, section 6, "Corriger"): removing or
    /// correcting an address no longer means taking back every one after
    /// it. What was being typed becomes an address first, so nothing typed
    /// is lost.
    @Test func aClickOnAnAddressTakesItBack() async {
        await reply()
        controller.focusCompose(.to)
        type("zoe@example.org,")
        #expect(await eventually { model.composer?.headers.to.count == 2 })
        let first = model.composer?.headers.to.first?.addr ?? ""
        type("dan@example.org")
        controller.clickRecipient(.to, 0)
        #expect(await eventually { model.composer?.line.text == first })
        #expect(model.composer?.line.selectedAll == true)
        #expect(model.composer?.focus == .to)
        let left = model.composer?.headers.to.map(\.addr) ?? []
        #expect(!left.contains(first))
        #expect(left == ["zoe@example.org", "dan@example.org"])
        // Delete: the address is gone, and the line is free again.
        press("del")
        #expect(model.composer?.line.text == "")
        #expect(model.composer?.headers.to.map(\.addr) == ["zoe@example.org", "dan@example.org"])
    }

    /// ⌥⌘B shows the Bcc line; an address typed and not ended is sent
    /// with the mail: saved before `send_draft`.
    @Test func bccAndAPendingAddressGoWithTheMail() async {
        await reply()
        press("alt-cmd-b")
        #expect(model.composer?.showsBcc == true)
        #expect(model.composer?.focus == .bcc)
        type("boss@example.org")
        press("cmd-ret")
        #expect(await eventually { core.calls("send_draft").count == 1 })
        let update = core.calls("update_draft").last
        #expect((update?["bcc"] as? [[String: Any]])?.first?["addr"] as? String == "boss@example.org")
    }

    /// A text that is not an address stops the sending.
    @Test func aWrongAddressStopsTheSending() async {
        await reply()
        press("shift-tab")
        press("shift-tab")
        press("shift-tab")
        type("nope")
        press("cmd-ret")
        #expect(await eventually { model.flash == L("Not an address: %@", "nope") })
        #expect(core.calls("send_draft").isEmpty)
        #expect(model.composer != nil)
    }

    /// The subject is a line of its own: ⇧Tab from the text, Return back
    /// to the text, saved like the rest.
    @Test func theSubjectIsEdited() async {
        await reply()
        press("shift-tab")
        #expect(model.composer?.focus == .subject)
        #expect(model.composer?.subjectLine.text == "Re: Subject 1")
        type(" (suite)")
        #expect(model.composer?.headers.subject == "Re: Subject 1 (suite)")
        press("ret")
        #expect(model.composer?.focus == .body)
        type("x")
        #expect(model.composer?.buffer.text.hasPrefix("x") == true)
        #expect(await eventually { core.calls("update_draft").last?["subject"] as? String == "Re: Subject 1 (suite)" })
    }

    /// The subject answers the editing keys of every macOS field: ⇧⌥← a
    /// word, ⌘A all, ⌘C ⌘X ⌘V, typing over a selection.
    @Test func theSubjectSelectsLikeAMacField() async {
        await reply()
        press("shift-tab")
        #expect(model.composer?.focus == .subject)
        press("shift-alt-left")
        #expect(model.composer?.subjectLine.selectedText == "1")
        press("shift-alt-left")
        #expect(model.composer?.subjectLine.selectedText == "Subject 1")
        press("cmd-c")
        #expect(controller.pasteboard.string(forType: .string) == "Subject 1")
        type("Devis")
        #expect(model.composer?.headers.subject == "Re: Devis")
        press("cmd-a")
        #expect(model.composer?.subjectLine.selectedAll == true)
        press("cmd-x")
        #expect(model.composer?.headers.subject == "")
        #expect(controller.pasteboard.string(forType: .string) == "Re: Devis")
        press("cmd-v")
        #expect(model.composer?.headers.subject == "Re: Devis")
        // ⇧⌥→ from the start selects the first word; ⌘A is not the list's
        // "mark all" here.
        press("cmd-left")
        press("shift-alt-right")
        #expect(model.composer?.subjectLine.selectedText == "Re")
        #expect(model.marked.isEmpty)
        #expect(await eventually { core.calls("update_draft").last?["subject"] as? String == "Re: Devis" })
    }

    /// ⇧⌥↓ in the text selects paragraph after paragraph.
    @Test func shiftOptionDownSelectsSeveralParagraphs() async {
        await reply()
        type("Bonjour,")
        press("ret")
        type("merci.")
        press("cmd-up")
        press("shift-alt-down")
        #expect(model.composer?.buffer.selectedText == "Bonjour,")
        press("shift-alt-down")
        #expect(model.composer?.buffer.selectedText == "Bonjour,\nmerci.")
        press("shift-alt-down")
        #expect(model.composer?.buffer.selectedText == "Bonjour,\nmerci.\n", "on into the empty line under it")
    }

    /// Escape closes the suggestions first, then leaves the reply.
    @Test func escapeClosesTheSuggestionsFirst() async {
        await reply()
        press("shift-tab")
        press("shift-tab")
        press("shift-tab")
        type("b")
        #expect(await eventually { model.composer?.suggestions.isEmpty == false })
        press("esc")
        #expect(model.composer?.suggestions.isEmpty == true)
        #expect(model.composer != nil)
        // `b` is not an address: the line is emptied before leaving.
        press("del")
        press("esc")
        #expect(await eventually { model.composer == nil })
    }

    /// A temporary folder standing for `incoming.noindex`, and a file to
    /// attach outside it.
    func attachmentFixture() throws -> (incoming: URL, file: URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nm-attach-\(UUID().uuidString)")
        let incoming = root.appendingPathComponent("incoming.noindex")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let file = root.appendingPathComponent("devis été.pdf")
        try Data("%PDF".utf8).write(to: file)
        model.incomingDirectory = incoming
        return (incoming, file)
    }

    /// ⇧⌘A (`brouillons.md`, section 7): the file chosen is copied into
    /// `incoming.noindex` under a plain name, then given to the core; Tab
    /// reaches the line of files, ⌫ removes the one selected.
    @Test func filesAreAttachedAndRemoved() async throws {
        let (incoming, file) = try attachmentFixture()
        model.chooseFiles = { [file] }
        await reply()
        press("cmd-A")
        #expect(await eventually { model.composer?.attachments.count == 1 })
        let call = core.calls("add_draft_attachment").first
        #expect(call?["filename"] as? String == "devis été.pdf")
        #expect(call?["mime_type"] as? String == "application/pdf")
        #expect(call?["revision"] as? Int == 1)
        let staged = try #require(call?["staged"] as? String)
        #expect(staged.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" })
        #expect(FileManager.default.fileExists(atPath: incoming.appendingPathComponent(staged).path))
        #expect(model.composer?.revision == 2)
        #expect(model.flash == L("Attached %@", "devis été.pdf"))
        // The line of files is a field: ⇧Tab from the text reaches it.
        press("shift-tab")
        #expect(model.composer?.focus == .attachments)
        press("del")
        #expect(await eventually { model.composer?.attachments.isEmpty == true })
        #expect(core.calls("remove_draft_attachment").first?["revision"] as? Int == 2)
        #expect(model.composer?.focus == .body)
    }

    /// Files copied in the Finder and pasted with ⌘V in the text are
    /// attached, not typed; a folder is refused.
    @Test func pastedFilesAreAttachedAndFoldersRefused() async throws {
        let (incoming, file) = try attachmentFixture()
        await reply()
        controller.pasteboard.clearContents()
        controller.pasteboard.writeObjects([file as NSURL])
        press("cmd-v")
        #expect(await eventually { model.composer?.attachments.count == 1 })
        #expect(model.composer?.buffer.text.hasPrefix("\n") == true)
        await model.attach([incoming.deletingLastPathComponent()])
        #expect(model.flash == L("A folder cannot be attached: %@", incoming.deletingLastPathComponent().lastPathComponent))
        #expect(core.calls("add_draft_attachment").count == 1)
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
        #expect(model.flash == L("Reply kept in Drafts · r to resume"))
        // Written at once, without waiting for the pause, then closed: the
        // core files it in Drafts.
        #expect(await eventually { core.calls("create_draft").count == 1 })
        #expect((core.calls("create_draft").first?["body"] as? String)?.hasPrefix("Merci") == true)
        #expect(await eventually { core.calls("close_draft").first?["id"] as? Int == 901 })
        // Keys are commands again.
        press("j")
        #expect(model.selectedThreadId == model.threads[1].id)
    }

    @Test func commandDeleteAbandonsAndZRestores() async {
        await reply()
        type("Mon texte")
        // Written at the pause, then abandoned: the core drops it.
        #expect(await eventually { core.calls("create_draft").count == 1 })
        press("cmd-del")
        #expect(await eventually { model.composer == nil })
        #expect(core.calls("discard_draft").first?["id"] as? Int == 901)
        #expect(model.flash == L("Reply abandoned · z to restore it"))
        // `z`: proposed again with its text, open at once, written at the
        // pause even if nothing more is typed.
        press("z")
        #expect(await eventually { model.composer != nil })
        let restore = core.calls("prepare_draft").last
        #expect(restore?["reply_to"] as? Int == 10)
        #expect((restore?["body"] as? String)?.hasPrefix("Mon texte") == true)
        #expect(await eventually { core.calls("create_draft").count == 2 })
        #expect((core.calls("create_draft").last?["body"] as? String)?.hasPrefix("Mon texte") == true)
    }

    /// Abandoned before the pause: never written, nothing to discard in
    /// the core; `z` gives the text back all the same.
    @Test func aReplyAbandonedBeforeThePauseWasNeverWritten() async {
        await reply()
        type("Vite")
        press("cmd-del")
        #expect(await eventually { model.composer == nil })
        #expect(core.calls("create_draft").isEmpty)
        #expect(core.calls("discard_draft").isEmpty)
        #expect(model.flash == L("Reply abandoned · z to restore it"))
        press("z")
        #expect(await eventually { model.composer?.buffer.text.hasPrefix("Vite") == true })
    }

    /// `r` then Escape without a word: nothing written, nothing closed,
    /// and the status bar says so. During an import, `r` never waits for
    /// the database (`reponse.md`, section 5).
    @Test func aReplyLeftAsProposedLeavesNothing() async {
        await reply()
        press("esc")
        #expect(await eventually { model.composer == nil })
        #expect(model.flash == L("Nothing written: no draft kept"))
        try? await Task.sleep(for: .milliseconds(700))
        #expect(core.calls("create_draft").isEmpty)
        #expect(core.calls("update_draft").isEmpty)
        #expect(core.calls("close_draft").isEmpty)
    }

    /// Sent without a word typed: written first, then sent, the id of the
    /// draft written.
    @Test func aProposalSentIsWrittenFirst() async {
        await reply()
        press("cmd-ret")
        #expect(await eventually { core.calls("send_draft").count == 1 })
        let order = core.calls.map(\.method).filter { ["create_draft", "send_draft"].contains($0) }
        #expect(order == ["create_draft", "send_draft"])
        #expect(core.calls("send_draft").first?["id"] as? Int == 901)
        #expect(core.calls("send_draft").first?["revision"] as? Int == 1)
    }

    /// Several things need the draft written at once (a file dropped while
    /// the pause saves): it is written once, the others wait for it.
    @Test func theDraftIsWrittenOnce() async {
        await reply()
        model.composer?.buffer = TextBuffer("Merci", cursor: 5)
        async let a = model.ensureWritten()
        async let b = model.ensureWritten()
        let (x, y) = await (a, b)
        #expect(x && y)
        #expect(core.calls("create_draft").count == 1)
        #expect(model.composer?.draft.id == 901)
    }

    /// The core cannot write it (the mail's body is gone…): the reply stays
    /// open with its text, the status bar says why, nothing else is called.
    @Test func aDraftThatCannotBeWrittenStaysOpen() async {
        core.handler = { method, params in
            method == "create_draft" ? CoreClient.RPCError(code: ComposeError.cannotSend, message: "no [smtp]") : Fixture.answer(method, params)
        }
        await reply()
        type("Merci")
        #expect(await eventually { model.flash == L("Cannot send: %@", "no [smtp]") })
        #expect(model.composer?.isWritten == false)
        #expect(model.composer?.buffer.text.hasPrefix("Merci") == true)
        #expect(core.calls("update_draft").isEmpty)
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
        // A new mail filed with ⇧⌘↩: the core applies it once the copy in
        // Sent is made (`filing`), so `z` only offers the decisions back.
        model.draftEvent("DraftSent", ["draft_id": .int(904), "filing": .string("waiting"), "undo": .string("11:12")])
        #expect(model.flash == L("Sent") + " · " + L("filed in %@", Role.name("waiting")) + " · " + L("z: undo"))
        #expect(model.undoStack.last?.label == L("decisions of the sending"))
    }

    /// ⌥⌘F: "Send from", the addresses of every account that can send;
    /// a key chooses one, saved with the draft and sent with it. A click
    /// on "From" opens the same choice; Escape leaves the sender as it was.
    @Test func commandOptionFChoosesTheSender() async {
        await reply()
        #expect(await eventually { model.senders.count == 3 })
        press("alt-cmd-f")
        #expect(controller.activeChoice == .sendFrom)
        let entries = controller.choiceEntries(.sendFrom)
        #expect(entries.map(\.key) == ["1", "2", "3", "esc"])
        #expect(entries.first?.detail == "•")
        press("esc")
        #expect(controller.activeChoice == nil)
        #expect(model.composer?.headers.sendAccountId == 1)
        controller.chooseSender()
        press("2")
        #expect(controller.activeChoice == nil)
        #expect(model.composer?.headers.from.addr == "me@work.example")
        #expect(model.composer?.headers.sendAccountId == 2)
        #expect(model.flash == L("Sending from %@", "Me at work <me@work.example> · work"))
        // Letters are text again.
        type("x")
        #expect(model.composer?.buffer.text.hasPrefix("x") == true)
        press("cmd-ret")
        #expect(await eventually { core.calls("send_draft").count == 1 })
        let from = core.calls("update_draft").compactMap { $0["from"] as? [String: Any] }.last
        #expect(from?["account_id"] as? Int == 2)
        #expect(from?["addr"] as? String == "me@work.example")
    }

    /// A single address that can send: nothing to choose, the status bar
    /// says which.
    @Test func withOneSenderThereIsNothingToChoose() async {
        core.handler = { method, params in
            method == "list_senders" ? ["items": [Fixture.senders[0]]] : Fixture.answer(method, params)
        }
        await reply()
        #expect(await eventually { model.senders.count == 1 })
        press("alt-cmd-f")
        #expect(controller.activeChoice == nil)
        #expect(model.flash == L("Only one address can send: %@", "Me <me@example.org> · perso"))
    }

    /// `r` on a mail of an account that cannot send: the core chose another
    /// account, and the status bar says from which.
    @Test func aReplyFromAnotherAccountIsAnnounced() async {
        core.handler = { method, params in
            guard method == "create_draft" || method == "prepare_draft" else { return Fixture.answer(method, params) }
            var d = Fixture.draft(replyTo: params["reply_to"] as? Int ?? 10)
            d["send_account_id"] = 2
            d["from"] = ["addr": "me@work.example", "name": "Me at work"]
            return ["draft": Fixture.opened(method, d), "reused": false]
        }
        await reply()
        #expect(await eventually { model.flash == L("This account cannot send: from %@ · ⌥⌘F to change", "Me at work <me@work.example> · work") })
    }

    @Test func aMailThatCannotBeAnsweredSaysWhy() async {
        core.handler = { method, params in
            method == "prepare_draft" ? CoreClient.RPCError(code: ComposeError.cannotSend, message: "no [smtp]") : Fixture.answer(method, params)
        }
        press("r")
        #expect(await eventually { model.flash == L("Cannot send: %@", "no [smtp]") })
        #expect(model.composer == nil)
    }

    /// `R`: a reply to the sender alone, the keys in the text.
    @Test func capitalRRepliesToTheSenderOnly() async {
        press("R")
        #expect(await eventually { model.composer != nil })
        #expect(core.calls("prepare_draft").first?["kind"] as? String == "reply_sender")
        #expect(model.composer?.focus == .body)
    }

    /// `f`: a forward, with the block that introduces the mail in the
    /// app's language; the keys go to To, empty.
    @Test func fForwardsTheMail() async {
        core.handler = { method, params in
            guard method == "create_draft" || method == "prepare_draft" else { return Fixture.answer(method, params) }
            var d = Fixture.draft(replyTo: params["reply_to"] as? Int ?? 10)
            d["kind"] = "forward"
            d["reply_to"] = NSNull()
            d["forward_of"] = params["reply_to"] ?? NSNull()
            d["to"] = []
            d["after_send_target"] = NSNull()
            return ["draft": Fixture.opened(method, d), "reused": false, "warning": NSNull()]
        }
        press("F")
        #expect(await eventually { model.composer != nil })
        let call = core.calls("prepare_draft").first
        #expect(call?["kind"] as? String == "forward")
        let header = call?["quote_header"] as? String ?? ""
        #expect(header.hasPrefix(L("---------- Forwarded message ---------")))
        #expect(header.contains(L("Subject: %@", "Subject 1")))
        #expect(model.composer?.focus == .to)
        type("x")
        #expect(model.composer?.line.text == "x")
        // Not an address: Escape keeps the draft open, to correct it.
        press("esc")
        #expect(await eventually { model.flash == L("Not an address: %@", "x") })
        #expect(model.composer != nil)
        press("del")
        type("dan@example.org")
        press("esc")
        #expect(await eventually { model.composer == nil })
        #expect(model.flash == L("Forward kept in Drafts · f to resume"))
        // Written as a forward of the mail, then given its recipient.
        let written = core.calls("create_draft").first
        #expect(written?["kind"] as? String == "forward")
        #expect(written?["reply_to"] as? Int == 10)
        #expect(written?["quote_header"] as? String == header)
        #expect(await eventually { (core.calls("update_draft").first?["to"] as? [[String: Any]])?.first?["addr"] as? String == "dan@example.org" })
    }

    /// `c`: a new mail from the account of the view; left empty, Escape
    /// says it is dropped.
    @Test func cWritesANewMail() async {
        core.handler = { method, params in
            guard method == "create_draft" || method == "prepare_draft" else { return Fixture.answer(method, params) }
            var d = Fixture.draft(replyTo: 10)
            d["kind"] = "new"
            d["reply_to"] = NSNull()
            d["to"] = []
            d["cc"] = []
            d["subject"] = ""
            d["body"] = ""
            d["thread_id"] = NSNull()
            d["after_send_target"] = NSNull()
            return ["draft": Fixture.opened(method, d), "reused": false]
        }
        press("c")
        #expect(await eventually { model.composer != nil })
        let call = core.calls("prepare_draft").first
        #expect(call?["kind"] as? String == "new")
        #expect(call?["account"] as? Int == 1)
        #expect(call?["reply_to"] == nil)
        #expect(model.composer?.focus == .to)
        #expect(model.composer?.isBlank == true)
        press("esc")
        #expect(await eventually { model.composer == nil })
        #expect(model.flash == L("Empty mail dropped"))
        // Never written: nothing to write, nothing to close.
        #expect(core.calls("create_draft").isEmpty)
        #expect(core.calls("close_draft").isEmpty)
    }

    /// Quarantine refuses `R` and `f` like `r`.
    @Test func quarantineRefusesTheRepliesAndTheForward() {
        #expect(GeekController.quarantineRefused.isSuperset(of: ["r", "R", "F"]))
        #expect(!GeekController.quarantineRefused.contains("c"))
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
