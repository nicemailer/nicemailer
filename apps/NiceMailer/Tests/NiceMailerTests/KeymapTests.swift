import AppKit
import Foundation
import SwiftUI
import Testing
@testable import NiceMailer

/// The keymap (`Keymap.swift`, `raccourcis.md`): how a key press is
/// spelled, how a key is found in the contexts, and how it is shown. Pure
/// functions, without a controller.
struct KeymapTests {
    /// The spelling of a key press, as `KeyChord.of` reads an event.
    func spell(_ chars: String, ignoring: String? = nil, code: UInt16 = 0, _ flags: NSEvent.ModifierFlags = []) -> String {
        KeyChord.spell(characters: chars, ignoringModifiers: ignoring ?? chars, keyCode: code, flags: flags)
    }

    // MARK: The catalogue

    /// The app knows every action of the core's catalogue, and no other:
    /// `keymap-defaults.json` is written by the core (`nicemailer keymap
    /// show --defaults --json`), and a test of the core checks it is up to
    /// date.
    @Test func theAppHasTheActionsOfTheCore() {
        #expect(Keymap.catalogue.count > 100)
        #expect(Set(KeyAction.allCases.map(\.rawValue)) == Set(Keymap.catalogue))
        // Every action of the defaults is known to the app.
        for (_, list) in Keymap.defaults.contexts {
            for b in list { #expect(KeyAction(rawValue: b.action) != nil, "\(b.action)") }
        }
        for m in Keymap.defaults.menus {
            for b in m.entries { #expect(KeyAction(rawValue: b.action) != nil, "\(b.action)") }
        }
    }

    @Test func everyContextOfTheDefaultsIsKnown() {
        #expect(Set(Keymap.defaults.contexts.keys) == Set(KeyContext.allCases))
        #expect(KeyContext(name: "menu_navigation") == .menuNavigation)
        // The JSON decoder's snake-case conversion also renames the keys of
        // a dictionary: both spellings are read.
        #expect(KeyContext(name: "menuNavigation") == .menuNavigation)
        #expect(KeyContext(name: "aiPanel") == .aiPanel)
        #expect(KeyContext(name: "nowhere") == nil)
    }

    // MARK: Spelling a key press

    @Test func aKeyWithoutModifierIsTheCharacterItTypes() {
        #expect(spell("j") == "j")
        // A capital is Shift, whether the flag is there (the keyboard) or
        // not (the tests and UI scripts send "J" alone).
        #expect(spell("J") == "J")
        #expect(spell("J", .shift) == "J")
        // `?` is the key that types it, on any layout: Shift is part of it.
        #expect(spell("?", .shift) == "?")
        #expect(spell("*") == "*")
        #expect(spell("é") == "é")
        // A dead key types nothing yet.
        #expect(spell("") == "")
    }

    @Test func withAModifierTheKeyIsTheLetterOnIt() {
        #expect(spell("\u{4}", ignoring: "d", code: 2, .control) == "ctrl-d")
        // ⌃ may give a control character: the letter from the key's place.
        #expect(spell("\u{6}", ignoring: "\u{6}", code: 3, [.control, .command]) == "ctrl-cmd-f")
        #expect(spell("C", .control) == "ctrl-shift-c")
        #expect(spell("c", [.control, .shift]) == "ctrl-shift-c")
        #expect(spell("A", ignoring: "A", code: 0, [.command, .shift]) == "cmd-shift-a")
        // ⌥ types another character: the letter of the key is read.
        #expect(spell("~", ignoring: "n", code: 45, .option) == "alt-n")
        // `+` needs Shift on some layouts: Shift is part of the character.
        #expect(spell("+", ignoring: "+", code: 24, [.command, .shift]) == "cmd-+")
        #expect(spell("v", ignoring: "V", code: 9, [.command, .option, .shift]) == "cmd-alt-shift-v")
    }

    @Test func namedKeysByTheirCode() {
        #expect(spell("\r", code: KeyCode.returnKey) == "enter")
        #expect(spell("\r", code: KeyCode.returnKey, .command) == "cmd-enter")
        #expect(spell("\r", code: KeyCode.returnKey, [.command, .shift]) == "cmd-shift-enter")
        #expect(spell("\u{1b}", code: KeyCode.escape) == "esc")
        #expect(spell("\u{1b}", code: KeyCode.escape, .command) == "cmd-esc")
        #expect(spell("\u{7f}", code: KeyCode.delete) == "backspace")
        #expect(spell("\u{F728}", code: KeyCode.forwardDelete, .function) == "del")
        #expect(spell("\u{F701}", code: KeyCode.down, [.function, .shift]) == "shift-down")
        #expect(spell("\u{F702}", code: KeyCode.left, [.function, .option]) == "alt-left")
        #expect(spell(" ") == "space")
        #expect(spell(" ", code: 49, .shift) == "shift-space")
        #expect(spell("\t", code: 48, .shift) == "shift-tab")
    }

    // MARK: Showing a key

    @Test func keysAreShownWithTheSymbolsOfTheMac() {
        #expect(KeyChord.display("j") == "j")
        #expect(KeyChord.display("J") == "J")
        #expect(KeyChord.display("ctrl-d") == "⌃D")
        #expect(KeyChord.display("ctrl-shift-c") == "⌃⇧C")
        #expect(KeyChord.display("cmd-enter") == "⌘↩")
        #expect(KeyChord.display("cmd-shift-enter") == "⇧⌘↩")
        #expect(KeyChord.display("cmd-alt-shift-v") == "⌥⇧⌘V")
        #expect(KeyChord.display("ctrl-cmd-f") == "⌃⌘F")
        #expect(KeyChord.display("shift-space") == "⇧space")
        #expect(KeyChord.display("cmd--") == "⌘-")
        #expect(KeyChord.display("-") == "-")
        #expect(KeyChord.display("backspace") == "⌫")
        #expect(KeyChord.display("f5") == "F5")
    }

    @Test func spellingsInParts() {
        #expect(KeyChord.parts("cmd-shift-a") == (["cmd", "shift"], "a"))
        #expect(KeyChord.parts("cmd--") == (["cmd"], "-"))
        #expect(KeyChord.parts("-") == ([], "-"))
        #expect(KeyChord.withoutShift("cmd-shift-left") == "cmd-left")
        #expect(KeyChord.withoutShift("alt-left") == nil)
    }

    // MARK: Finding a key

    @Test func theMostPreciseContextDecides() {
        let mail: [KeyContext] = [.mail, .normal, .everywhere]
        let quarantine: [KeyContext] = [.quarantine, .mail, .normal, .everywhere]
        let k = Keymap.defaults
        #expect(k.action("d", in: mail) == .mailArchive)
        #expect(k.action("d", in: quarantine) == .moveDeleted)
        #expect(k.action("j", in: quarantine) == .listDown)
        #expect(k.action("?", in: [.contacts, .screen, .normal, .everywhere]) == .helpShow)
        #expect(k.action("ctrl-cmd-f", in: [.compose, .everywhere]) == .windowFullScreen)
        #expect(k.action("j", in: [.compose, .everywhere]) == nil)
        #expect(k.action("", in: mail) == nil)
    }

    @Test func noneHidesTheKeyOfAParent() {
        let k = Keymap.defaults.binding("d", to: "none", in: .mail).binding("?", to: "none", in: .contacts)
        #expect(k.action("d", in: [.mail, .normal, .everywhere]) == nil)
        #expect(k.action("d", in: [.quarantine, .mail, .normal, .everywhere]) == .moveDeleted)
        #expect(k.action("?", in: [.contacts, .screen, .normal, .everywhere]) == nil)
        #expect(k.action("?", in: [.tags, .screen, .normal, .everywhere]) == .helpShow)
        // An action this app does not know (a newer core): no action.
        #expect(Keymap.defaults.binding("x", to: "mail.teleport", in: .mail).action("x", in: [.mail]) == nil)
    }

    @Test func theKeysOfAnActionLeaveOutThoseAContextBeforeTakes() {
        let chain: [KeyContext] = [.quarantine, .mail, .normal, .everywhere]
        #expect(Keymap.defaults.keys(.listDown, in: [.mail]) == ["j", "down"])
        // In Quarantine, `d` deletes: it no longer archives there.
        #expect(Keymap.defaults.keys(.mailArchive, in: chain).isEmpty)
        #expect(Keymap.defaults.keys(.mailArchive, in: [.mail, .normal, .everywhere]) == ["d"])
        #expect(Keymap.defaults.label(.aiComposeMenu, in: Keymap.composeChain) == "⌃I")
        #expect(Keymap.defaults.mention(.aiComposeMenu, in: Keymap.composeChain) == "⌃I")
        // No key: the action's name, so that a message still says what to use.
        let none = Keymap.defaults.binding("ctrl-i", to: "none", in: .compose)
        #expect(none.mention(.aiComposeMenu, in: Keymap.composeChain) == "ai.compose_menu")
    }

    @Test func aMenuOfTheUserIsOpenedByItsKey() {
        let k = Keymap.defaults.withMenu("mine", [("a", "save.attachments")]).binding("x", to: "menu.mine", in: .mail)
        #expect(k.userMenu("x", in: [.mail, .normal, .everywhere]) == "mine")
        #expect(k.action("x", in: [.mail]) == nil)
        // A built-in menu is an action, not a menu of the user.
        #expect(k.userMenu("m", in: [.mail]) == nil)
        // A menu the keymap does not have: nothing.
        #expect(Keymap.defaults.binding("x", to: "menu.other", in: .mail).userMenu("x", in: [.mail]) == nil)
    }

    // MARK: The menu bar

    @Test func theMenuBarShowsTheKeysOfEverywhere() {
        #expect(Keymap.defaults.shortcut(.appReload) == KeyboardShortcut("r", modifiers: .command))
        #expect(Keymap.defaults.shortcut(.textBigger) == KeyboardShortcut("+", modifiers: .command))
        #expect(Keymap.shortcut("cmd-shift-a") == KeyboardShortcut("a", modifiers: [.command, .shift]))
        #expect(Keymap.shortcut("ctrl-cmd-f") == KeyboardShortcut("f", modifiers: [.command, .control]))
        #expect(Keymap.shortcut("cmd-enter") == KeyboardShortcut(.return, modifiers: .command))
        // Removed from `everywhere`: no shortcut shown.
        #expect(Keymap.defaults.binding("cmd-r", to: "none", in: .everywhere).shortcut(.appReload) == nil)
    }

    // MARK: The answer of the core

    /// `keymap_get` as the core sends it, read with the app's decoder (its
    /// snake-case conversion renames the context names).
    @Test func theAnswerOfTheCoreIsRead() throws {
        let json = """
        {"path": "/x/keymap.toml", "exists": true,
         "contexts": {"mail": [{"key": "e", "action": "mail.archive"}], "menu_navigation": [{"key": "down", "action": "menu.next"}]},
         "menus": [{"name": "mine", "builtin": false, "entries": [{"key": "a", "action": "save.pdf"}]}],
         "error": {"file": "keymap.toml", "line": 3, "key": "mail.esc", "message": "cannot be changed"},
         "warnings": []}
        """
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        let view = try d.decode(KeymapView.self, from: Data(json.utf8))
        let k = Keymap(contexts: view.contexts, menus: view.menus)
        #expect(k.action("e", in: [.mail]) == .mailArchive)
        #expect(k.action("down", in: [.menuNavigation]) == .menuNext)
        #expect(k.menu("mine")?.entries.first?.action == "save.pdf")
        #expect(view.error?.text == "keymap.toml, " + L("line %lld", 3) + ": mail.esc: cannot be changed")
    }

    // MARK: The help

    @Test func theHelpShowsTheKeysInEffect() {
        let help = Help.groups().flatMap(\.entries)
        #expect(help.first { $0.label == L("done (archive)") }?.key == "d")
        #expect(help.first { $0.label == L("next / previous mail") }?.key == "j k")
        #expect(help.first { $0.label == L("the same, with the arrows") }?.key == "⇧↓ ⇧↑")
        #expect(help.first { $0.label == L("scroll the mail a page up") }?.key == "⇧space ⌃B")
        #expect(help.first { $0.label == L("move to a folder") }?.key == "m…")
        #expect(help.first { $0.label == L("in Quarantine: move without %@", "m") }?.key == "i n p x b d")

        let mine = Keymap.defaults.binding("e", to: "mail.archive", in: .mail).binding("d", to: "none", in: .mail)
            .withMenu("mine", [("a", "save.attachments")]).binding("x", to: "menu.mine", in: .mail)
            .binding("ctrl-b", to: "none", in: .mail)
        let groups = Help.groups(mine)
        let entries = groups.flatMap(\.entries)
        #expect(entries.first { $0.label == L("done (archive)") }?.key == "e")
        #expect(entries.first { $0.label == L("scroll the mail a page up") }?.key == "⇧space")
        #expect(groups.last?.title == L("Your menus"))
        #expect(groups.last?.entries == [HelpEntry(key: "x…", label: "mine")])
        // An action without a key is left out.
        let noFlag = Help.groups(Keymap.defaults.binding("*", to: "none", in: .mail)).flatMap(\.entries)
        #expect(!noFlag.contains { $0.label == L("flag") })
    }
}

/// The keymap in the controller: keys redefined by `keymap.toml` act, and
/// the screen says the keys in effect.
@MainActor
struct KeymapControllerTests {
    let core: FakeCore
    let model = AppModel()
    let controller: GeekController

    init() async {
        core = FakeCore { method, params in
            switch method {
            case "list_folders":
                var folders = Fixture.folders
                folders.append(Fixture.folder(5, "quarantine", "Quarantine", "Q", targets: QuarantineTests.targets))
                return ["items": folders, "as_of": Fixture.position]
            case "list_threads":
                let folder = (params["folder_ids"] as? [Int])?.first ?? params["folder_id"] as? Int ?? 1
                guard folder == 5 else { return Fixture.answer(method, params) }
                let items = (51...54).map { Fixture.thread($0, folder: 5) }
                return ["items": items, "next_cursor": NSNull(), "as_of": Fixture.position]
            case "keymap_get":
                // A keymap.toml with `e` archiving and `d` removed, and an
                // error on a later save (the keys stay).
                var mail = Keymap.defaults.contexts[.mail]!.map { ["key": $0.key, "action": $0.action] }
                mail.append(["key": "e", "action": "mail.archive"])
                mail = mail.map { $0["key"] == "d" ? ["key": "d", "action": "none"] : $0 }
                var contexts: [String: Any] = [:]
                for (c, list) in Keymap.defaults.contexts { contexts[c.rawValue] = list.map { ["key": $0.key, "action": $0.action] } }
                contexts["mail"] = mail
                let menus = Keymap.defaults.menus.map { m in
                    ["name": m.name, "builtin": m.builtin, "entries": m.entries.map { ["key": $0.key, "action": $0.action] }] as [String: Any]
                }
                return ["path": "/tmp/keymap.toml", "exists": true, "contexts": contexts, "menus": menus,
                        "error": ["file": "keymap.toml", "line": 2, "key": "mail.esc", "message": "cannot be changed"],
                        "warnings": []]
            default:
                return Fixture.answer(method, params)
            }
        }
        let defaults = TestDefaults.make()
        controller = GeekController(model: model, defaults: defaults, textSize: TextSize(defaults: defaults))
        controller.pasteboard = NSPasteboard(name: NSPasteboard.Name("NiceMailerTests-\(UUID().uuidString)"))
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 && model.openThread != nil }
    }

    func press(_ chars: String, code: UInt16 = 0, _ mods: NSEvent.ModifierFlags = []) {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0, windowNumber: 0, context: nil,
            characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
        _ = controller.handle(event)
    }

    @Test func theKeysOfTheCoreAreApplied() async {
        // Asked when the session starts (`startSession`).
        #expect(await eventually { model.keymap.action("e", in: [.mail]) == .mailArchive })
        #expect(model.keymapProblem?.line == 2)
        press("d")
        #expect(model.flash == L("No command %@", "d"))
        #expect(core.moveCalls().isEmpty)
        press("e")
        #expect(await eventually { core.moveCalls().first?["target"] as? String == "archive" })
        // The help says so.
        let help = Help.groups(controller.keymap).flatMap(\.entries)
        #expect(help.first { $0.label == L("done (archive)") }?.key == "e")
    }

    @Test func aMovedMenuKeyOpensItAndTheStatusBarSaysSo() {
        model.keymap = Keymap.defaults.binding("m", to: "none", in: .mail).binding("M", to: "menu.move", in: .mail)
        #expect(controller.menuKeyLabel(.move) == "M")
        press("m")
        #expect(controller.overlay == .none)
        press("M")
        #expect(controller.overlay == .menu(.move))
        // The mode shown is the key pressed.
        #expect(controller.modeLabel == "M")
    }

    @Test func aMenuOfTheUser() async {
        model.keymap = Keymap.defaults
            .withMenu("mine", [("c", "save.clipboard"), ("v", "view.one_column")])
            .binding("x", to: "menu.mine", in: .mail)
        press("x")
        #expect(controller.overlay == .menu(.user("mine")))
        let rows = controller.menuEntries(.user("mine"))
        #expect(rows.map(\.key) == ["c", "v"])
        #expect(rows.map(\.label) == [L("to clipboard"), L("one column")])
        press("v")
        #expect(controller.overlay == .none)
        #expect(controller.columns == 1)
    }

    @Test func aMenuRowMovedToAnotherKey() {
        model.keymap = Keymap.defaults.withMenu("save", [("y", "save.text"), ("Y", "save.text_open")], builtin: true)
        let rows = controller.menuEntries(.save)
        // The variant that opens shares the row of its action.
        #expect(rows.count == 1)
        #expect(rows[0].key == "y" && rows[0].altKey == "Y")
        #expect(rows[0].detail == L("%@: and open", "Y"))
    }

    @Test func aDirectMoveWithoutTheMenu() async {
        model.keymap = Keymap.defaults.binding("A", to: "move.archive", in: .mail)
        press("A")
        #expect(await eventually { core.moveCalls().first?["target"] as? String == "archive" })
    }

    @Test func aDirectGoToWithoutTheMenu() async {
        model.keymap = Keymap.defaults.binding("Q", to: "goto.quarantine", in: .mail)
        press("Q")
        #expect(await eventually { model.currentRole == "quarantine" })
    }

    @Test func quarantineRefusesAnActionWhateverItsKey() async {
        model.keymap = Keymap.defaults.binding("y", to: "mail.toggle_seen", in: .mail)
        press(";")
        press("q")
        _ = await eventually { model.threads.first?.id == 51 }
        press("y")
        #expect(model.flash == L("In Quarantine, a mail can only be moved or flagged"))
    }

    @Test func aLineActionOnAnotherKeyAndShiftSelects() {
        model.keymap = Keymap.defaults.binding("ctrl-b", to: "line.start", in: .lineEdit)
        press("/")
        for c in "abc" { press(String(c)) }
        #expect(controller.pickerLine.cursor == 3)
        press("b", code: 11, .control)
        #expect(controller.pickerLine.cursor == 0)
        // Shift added to a move selects: ⇧⌘→ to the end.
        press("\u{F703}", code: KeyCode.right, [.function, .command, .shift])
        #expect(controller.pickerLine.selection == 0..<3)
    }

    @Test func theTextSizeKeysFollowTheKeymap() {
        model.keymap = Keymap.defaults.binding("cmd-+", to: "none", in: .everywhere)
            .binding("ctrl-+", to: "text.bigger", in: .everywhere)
        let before = controller.textSize.size
        press("+", code: 24, .command)
        #expect(controller.textSize.size == before)
        press("+", code: 24, .control)
        #expect(controller.textSize.size > before)
    }
}
