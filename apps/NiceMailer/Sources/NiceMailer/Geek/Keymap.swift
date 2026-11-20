import AppKit
import Foundation
import SwiftUI

// The keys of the geek theme, redefined by the user in `keymap.toml`
// (`specifications/raccourcis.md`, `docs/keymap.md`).
//
// The core reads the file, checks it and merges it into the defaults; the
// app receives the result (`keymap_get`) and never reads the file. What is
// here:
//
// - `KeyContext`: where a key is looked for (the mail list, a picker, a
//   reply being written…);
// - `KeyAction`: what a key can do, the same list as the core's catalogue
//   (`nm-engine`, `keymap/catalog.rs`; a test checks they agree);
// - `KeyChord`: the spelling of a key press, built from an `NSEvent` the
//   same way the core spells the keys of the file, so that finding a key
//   is comparing two strings;
// - `Keymap`: the keys in effect, turned into dictionaries once, so a key
//   press costs a few lookups.
//
// Everything here is pure (no view, no controller), and unit-tested
// (`KeymapTests`).

/// Where a key is looked for (`raccourcis.md`, section 5). The raw value is
/// the name of the table in `keymap.toml`.
enum KeyContext: String, CaseIterable {
    case everywhere, normal, mail, quarantine, screen, contacts, tags, groups, history
    case menuNavigation = "menu_navigation"
    case picker
    case contactFilter = "contact_filter"
    case lineEdit = "line_edit"
    case choice
    case aiPanel = "ai_panel"
    case compose

    /// The context of a table name, as the file writes it (`menu_navigation`)
    /// or as the JSON decoder may have turned it (`menuNavigation`: the
    /// decoder's snake-case conversion also applies to the keys of a
    /// dictionary).
    init?(name: String) {
        let flat = name.replacingOccurrences(of: "_", with: "").lowercased()
        guard let c = Self.allCases.first(where: { $0.rawValue.replacingOccurrences(of: "_", with: "") == flat }) else {
            return nil
        }
        self = c
    }
}

/// What a key can do: the catalogue of `raccourcis.md`, section 6.
///
/// Swift note: an `enum` with `String` raw values gets `init(rawValue:)`
/// for free, which returns `nil` for an unknown name: an action a newer
/// core knows, but not this app, is ignored rather than crashing.
enum KeyAction: String, CaseIterable {
    case windowFullScreen = "window.full_screen"
    case textBigger = "text.bigger"
    case textSmaller = "text.smaller"
    case textReset = "text.reset"
    case appReload = "app.reload"
    case menuGoto = "menu.goto"
    case helpShow = "help.show"
    case undoLast = "undo.last"
    case gotoInbox = "goto.inbox"
    case gotoTodo = "goto.todo"
    case gotoWaiting = "goto.waiting"
    case gotoWork = "goto.work"
    case gotoArchive = "goto.archive"
    case gotoNewsletter = "goto.newsletter"
    case gotoPaperwork = "goto.paperwork"
    case gotoSent = "goto.sent"
    case gotoDrafts = "goto.drafts"
    case gotoAllMail = "goto.all_mail"
    case gotoQuarantine = "goto.quarantine"
    case gotoDeleted = "goto.deleted"
    case gotoJunk = "goto.junk"
    case gotoBanned = "goto.banned"
    case gotoEverything = "goto.everything"
    case gotoContacts = "goto.contacts"
    case gotoTags = "goto.tags"
    case gotoGroups = "goto.groups"
    case gotoHistory = "goto.history"
    case listDown = "list.down"
    case listUp = "list.up"
    case listMarkDown = "list.mark_down"
    case listMarkUp = "list.mark_up"
    case listMarkAll = "list.mark_all"
    case listHalfPageDown = "list.half_page_down"
    case listHalfPageUp = "list.half_page_up"
    case messageNext = "message.next"
    case messagePrevious = "message.previous"
    case mailOpen = "mail.open"
    case readerOpenOrPageDown = "reader.open_or_page_down"
    case readerPageDown = "reader.page_down"
    case readerPageUp = "reader.page_up"
    case readerToggleAll = "reader.toggle_all"
    case readerSkipQuoted = "reader.skip_quoted"
    case pickerLinks = "picker.links"
    case senderCardOrCopy = "sender.card_or_copy"
    case senderMails = "sender.mails"
    case mailArchive = "mail.archive"
    case menuMove = "menu.move"
    case mailDelete = "mail.delete"
    case mailToggleSeen = "mail.toggle_seen"
    case mailMarkSeen = "mail.mark_seen"
    case mailToggleFlag = "mail.toggle_flag"
    case pickerTag = "picker.tag"
    case pickerGroup = "picker.group"
    case menuSave = "menu.save"
    case mailUnsubscribe = "mail.unsubscribe"
    case composeReplyAll = "compose.reply_all"
    case composeReplySender = "compose.reply_sender"
    case composeForward = "compose.forward"
    case composeNew = "compose.new"
    case aiMenu = "ai.menu"
    case pickerSearch = "picker.search"
    case pickerAttributes = "picker.attributes"
    case pickerAccounts = "picker.accounts"
    case menuView = "menu.view"
    case moveInbox = "move.inbox"
    case moveTodo = "move.todo"
    case moveWaiting = "move.waiting"
    case moveArchive = "move.archive"
    case moveNewsletter = "move.newsletter"
    case movePaperwork = "move.paperwork"
    case moveQuarantine = "move.quarantine"
    case moveDeleted = "move.deleted"
    case moveJunk = "move.junk"
    case moveBanned = "move.banned"
    case saveClipboard = "save.clipboard"
    case saveEml = "save.eml"
    case saveText = "save.text"
    case saveTextOpen = "save.text_open"
    case saveMarkdown = "save.markdown"
    case saveMarkdownOpen = "save.markdown_open"
    case saveAttachments = "save.attachments"
    case saveAttachmentsNoGraphics = "save.attachments_no_graphics"
    case savePdf = "save.pdf"
    case savePdfOpen = "save.pdf_open"
    case savePdfEvidence = "save.pdf_evidence"
    case savePdfEvidenceOpen = "save.pdf_evidence_open"
    case viewOneColumn = "view.one_column"
    case viewTwoColumns = "view.two_columns"
    case viewThreeColumns = "view.three_columns"
    case viewSortReceived = "view.sort_received"
    case viewSortModified = "view.sort_modified"
    case viewToggleHtml = "view.toggle_html"
    case viewToggleRemoteImages = "view.toggle_remote_images"
    case viewCycleStatusBar = "view.cycle_status_bar"
    case screenDown = "screen.down"
    case screenUp = "screen.up"
    case screenOpen = "screen.open"
    case screenHalfPageDown = "screen.half_page_down"
    case screenHalfPageUp = "screen.half_page_up"
    case historyToggleRead = "history.toggle_read"
    case historyFilterSource = "history.filter_source"
    case contactsFilter = "contacts.filter"
    case contactsDecideAddress = "contacts.decide_address"
    case contactsDecideDomain = "contacts.decide_domain"
    case contactsSenderKind = "contacts.sender_kind"
    case labelRename = "label.rename"
    case labelDelete = "label.delete"
    case menuNext = "menu.next"
    case menuPrevious = "menu.previous"
    case menuChoose = "menu.choose"
    case menuChooseAlt = "menu.choose_alt"
    case pickerNext = "picker.next"
    case pickerPrevious = "picker.previous"
    case pickerChoose = "picker.choose"
    case pickerChooseAlt = "picker.choose_alt"
    case contactFilterClose = "contact_filter.close"
    case lineStart = "line.start"
    case lineEnd = "line.end"
    case lineWordLeft = "line.word_left"
    case lineWordRight = "line.word_right"
    case lineDeleteWord = "line.delete_word"
    case lineDeleteToStart = "line.delete_to_start"
    case lineSelectAll = "line.select_all"
    case lineCopy = "line.copy"
    case lineCut = "line.cut"
    case linePaste = "line.paste"
    case choiceNext = "choice.next"
    case choicePrevious = "choice.previous"
    case choiceChoose = "choice.choose"
    case aiAccept = "ai.accept"
    case aiRefuse = "ai.refuse"
    case aiCopy = "ai.copy"
    case aiConfirm = "ai.confirm"
    case aiOpen = "ai.open"
    case aiNext = "ai.next"
    case aiPrevious = "ai.previous"
    case composeSend = "compose.send"
    case composeFileAndSend = "compose.file_and_send"
    case composeAbandon = "compose.abandon"
    case aiComposeMenu = "ai.compose_menu"
    case composeTogglePlain = "compose.toggle_plain"
    case composeAttach = "compose.attach"
    case composeFrom = "compose.from"
    case composeBcc = "compose.bcc"
    case composeNextField = "compose.next_field"
    case composePreviousField = "compose.previous_field"
    case composePaste = "compose.paste"
    case composePastePlain = "compose.paste_plain"

    /// The workflow role of `move.<role>` (`inbox`…), or `nil`.
    var moveRole: String? {
        rawValue.hasPrefix("move.") ? String(rawValue.dropFirst(5)) : nil
    }

    /// The role of `goto.<role>` for a folder of the workflow, or `nil`
    /// (the virtual folders and the screens have their own properties).
    var gotoRole: String? {
        guard rawValue.hasPrefix("goto."), gotoVirtual == nil, gotoScreen == nil else { return nil }
        return String(rawValue.dropFirst(5))
    }

    /// The virtual folder of `goto.work`, `goto.all_mail`, `goto.everything`.
    var gotoVirtual: AppModel.Virtual? {
        switch self {
        case .gotoWork: .work
        case .gotoAllMail: .allMail
        case .gotoEverything: .everything
        default: nil
        }
    }

    /// The screen of `goto.contacts`, `goto.tags`, `goto.groups`, `goto.history`.
    var gotoScreen: GeekController.Screen? {
        switch self {
        case .gotoContacts: .contacts
        case .gotoTags: .tags
        case .gotoGroups: .groups
        case .gotoHistory: .history
        default: nil
        }
    }

    /// The action for a folder of the menu `;`: `goto.<role>`, or the one
    /// of a virtual folder.
    static func goto(role: String?, virtual: AppModel.Virtual?) -> KeyAction? {
        switch virtual {
        case .work: return .gotoWork
        case .allMail: return .gotoAllMail
        case .everything: return .gotoEverything
        case nil: return role.flatMap { KeyAction(rawValue: "goto." + $0) }
        }
    }

    /// `save.text` → `save.text_open`: the variant that saves, then opens
    /// the file. A menu with both shows them on one row, the variant's key
    /// on the right ("T: and open"), and ⌘-Return runs it.
    var openVariant: KeyAction? {
        rawValue.hasPrefix("save.") ? KeyAction(rawValue: rawValue + "_open") : nil
    }

    /// Whether it is the `_open` variant of another action.
    var isOpenVariant: Bool { rawValue.hasPrefix("save.") && rawValue.hasSuffix("_open") }

    /// Refused in Quarantine, whatever its key (`raccourcis.md`, section
    /// 6.1, `quarantine`): the actions that change a mail other than by
    /// moving it or flagging it (`GeekController.quarantineRefused`).
    var refusedInQuarantine: Bool {
        switch self {
        case .mailToggleSeen, .mailMarkSeen, .pickerTag, .pickerGroup, .composeReplyAll, .composeReplySender,
             .composeForward, .menuSave:
            true
        default:
            false
        }
    }

    /// The moves of a line being typed, which Shift turns into a selection
    /// (`raccourcis.md`, section 5.2): `alt-left` moves a word left,
    /// `alt-shift-left` selects it, without a binding of its own.
    var isLineMove: Bool {
        switch self {
        case .lineStart, .lineEnd, .lineWordLeft, .lineWordRight: true
        default: false
        }
    }

    /// What the row of a menu says for the action, also used by the help.
    /// A folder is named in English here; the menus name it as the file
    /// does (`GeekController.label(of:)`). Only the actions that may be a row of a menu (those of the mail
    /// screen and of every screen) need a label; the others, which move in
    /// a panel, return their name.
    var label: String {
        if let role = moveRole { return Role.name(role) }
        if let role = gotoRole { return Role.name(role) }
        if let v = gotoVirtual { return v.name }
        switch self {
        case .windowFullScreen: return L("full screen")
        case .textBigger: return L("bigger text")
        case .textSmaller: return L("smaller text")
        case .textReset: return L("default text size")
        case .appReload: return L("reload")
        case .menuGoto: return L("go to a folder or a screen")
        case .helpShow: return L("this list of keys")
        case .undoLast: return L("undo the last move or decision")
        case .gotoContacts: return L("Contacts")
        case .gotoTags: return L("Tags")
        case .gotoGroups: return L("Groups")
        case .gotoHistory: return L("History")
        case .listDown: return L("next mail")
        case .listUp: return L("previous mail")
        case .listMarkDown: return L("mark and move down")
        case .listMarkUp: return L("mark and move up")
        case .listMarkAll: return L("mark every mail of the list")
        case .listHalfPageDown: return L("half a page down in the list")
        case .listHalfPageUp: return L("half a page up in the list")
        case .messageNext: return L("next message of the conversation")
        case .messagePrevious: return L("previous message of the conversation")
        case .mailOpen: return L("read the mail in the whole window (one column)")
        case .readerOpenOrPageDown, .readerPageDown: return L("scroll the mail a page down")
        case .readerPageUp: return L("scroll the mail a page up")
        case .readerToggleAll: return L("show / hide quotes, signatures, folded mails")
        case .readerSkipQuoted: return L("skip to the text after the next quote")
        case .pickerLinks: return L("links of the mail")
        case .senderCardOrCopy: return L("sender's card in Contacts, or copy address")
        case .senderMails: return L("mails of the sender")
        case .mailArchive: return L("done (archive)")
        case .menuMove: return L("move to a folder")
        case .mailDelete: return L("delete")
        case .mailToggleSeen: return L("read / unread")
        case .mailMarkSeen: return L("mark as read")
        case .mailToggleFlag: return L("flag")
        case .pickerTag: return L("add / remove a tag")
        case .pickerGroup: return L("add / remove a group")
        case .menuSave: return L("save")
        case .mailUnsubscribe: return L("unsubscribe from the list")
        case .composeReplyAll: return L("reply to all")
        case .composeReplySender: return L("reply to the sender only")
        case .composeForward: return L("forward")
        case .composeNew: return L("write a new mail")
        case .aiMenu: return L("AI menu of the mail: summary, translation, quick replies, security")
        case .pickerSearch: return L("search everywhere, in All Mail")
        case .pickerAttributes: return L("filter by attributes")
        case .pickerAccounts: return L("go to an account")
        case .menuView: return L("display")
        case .saveClipboard: return L("to clipboard")
        case .saveEml: return L("as .eml file")
        case .saveText: return L("as text file")
        case .saveTextOpen: return L("as text file, then open it")
        case .saveMarkdown: return L("as Markdown file")
        case .saveMarkdownOpen: return L("as Markdown file, then open it")
        case .saveAttachments: return L("all attachments")
        case .saveAttachmentsNoGraphics: return L("attachments but graphics")
        case .savePdf: return L("as PDF")
        case .savePdfOpen: return L("as PDF, then open it")
        case .savePdfEvidence: return L("as PDF with technical information")
        case .savePdfEvidenceOpen: return L("as PDF with technical information, then open it")
        case .viewOneColumn: return L("one column")
        case .viewTwoColumns: return L("two columns")
        case .viewThreeColumns: return L("three columns")
        case .viewSortReceived: return L("sort by reception date")
        case .viewSortModified: return L("sort by modification date")
        case .viewToggleHtml: return L("text / HTML")
        case .viewToggleRemoteImages: return L("remote images")
        case .viewCycleStatusBar: return L("status bar: bottom, top, hidden")
        case .aiComposeMenu: return L("in a reply: AI menu (proofread, rewrite, translate, subjects)")
        case .composeTogglePlain: return L("in a reply: text only or formatted text")
        default: return rawValue
        }
    }
}

// MARK: - Key presses

/// The spelling of a key press, as the core spells the keys of the file
/// (`nm-engine`, `keymap/notation.rs`): `j`, `J`, `?`, `ctrl-d`,
/// `cmd-shift-a`, `shift-space`, `enter`.
///
/// Without ⌃, ⌥ nor ⌘, a key is the character it types (`event.characters`):
/// `?` is the key that types `?`, wherever the layout puts it. With one of
/// them, it is the letter printed on the key (`charactersIgnoringModifiers`):
/// ⌥ types other characters on a Mac, ⌃ control characters. Shift is kept
/// as `shift-` only for letters and named keys; for the other characters
/// it is part of the character (`?`, not `shift-/`).
enum KeyChord {
    /// The keys that type no character (or an invisible one), by their
    /// virtual key code: the same on every layout.
    static let namedCodes: [UInt16: String] = [
        36: "enter", 76: "enter", 48: "tab", 49: "space", 51: "backspace", 53: "esc", 117: "del",
        123: "left", 124: "right", 125: "down", 126: "up", 115: "home", 119: "end", 116: "pageup", 121: "pagedown",
        122: "f1", 120: "f2", 99: "f3", 118: "f4", 96: "f5", 97: "f6", 98: "f7", 100: "f8", 101: "f9", 109: "f10",
        103: "f11", 111: "f12",
    ]

    /// The letters by their place on an ANSI keyboard, for ⌃ with a
    /// letter: AppKit may then give the control character (0x06 for F)
    /// rather than the letter. The place is QWERTY's, a fallback only.
    static let letterCodes: [UInt16: Character] = [
        0: "a", 1: "s", 2: "d", 3: "f", 4: "h", 5: "g", 6: "z", 7: "x", 8: "c", 9: "v", 11: "b", 12: "q", 13: "w",
        14: "e", 15: "r", 16: "y", 17: "t", 31: "o", 32: "u", 34: "i", 35: "p", 37: "l", 38: "j", 40: "k", 45: "n",
        46: "m",
    ]

    /// The spelling of `event`, or `""` for a key that types nothing
    /// usable (a dead key, a lone modifier).
    static func of(_ event: NSEvent) -> String {
        // Swift note: `characters` throws for events that are not key
        // events; the monitor only gives key events.
        spell(characters: event.characters, ignoringModifiers: event.charactersIgnoringModifiers,
              keyCode: event.keyCode, flags: event.modifierFlags)
    }

    /// The pure part of `of`, tested without events.
    static func spell(characters: String?, ignoringModifiers: String?, keyCode: UInt16, flags: NSEvent.ModifierFlags) -> String {
        let f = flags.intersection(.deviceIndependentFlagsMask)
        let ctrl = f.contains(.control), cmd = f.contains(.command), alt = f.contains(.option)
        var shift = f.contains(.shift)
        let bare = !ctrl && !cmd && !alt
        var key: String
        if let named = namedCodes[keyCode] {
            key = named
        } else {
            let text = (bare ? characters : ignoringModifiers) ?? ""
            guard text.count == 1, var c = text.first else { return "" }
            if c == " " { key = "space" } else {
                if let scalar = c.unicodeScalars.first, scalar.value < 0x20 || scalar.value == 0x7f {
                    // A control character: the letter from the key's place,
                    // with ⌃ only (a bare control character is a named key
                    // the table above missed: ignored).
                    guard ctrl, let letter = letterCodes[keyCode] else { return "" }
                    c = letter
                }
                let lower = Character(c.lowercased())
                let hasCase = c.lowercased() != c.uppercased()
                if hasCase {
                    // A capital means Shift (or Caps Lock), held or not in
                    // the flags: the tests and UI scripts send "J" alone.
                    shift = bare ? c != lower : (shift || c != lower)
                    key = String(lower)
                } else {
                    shift = false
                    key = String(c)
                }
            }
            if bare, shift, key.count == 1, key.lowercased() != key.uppercased() {
                return key.uppercased()
            }
        }
        var s = ""
        if ctrl { s += "ctrl-" }
        if cmd { s += "cmd-" }
        if alt { s += "alt-" }
        if shift { s += "shift-" }
        return s + key
    }

    /// The modifiers and the key of a spelling: `cmd-shift-a` → (["cmd",
    /// "shift"], "a"). A single character is a key, even `-`.
    static func parts(_ spelling: String) -> (mods: Set<String>, key: String) {
        var rest = Substring(spelling)
        var mods: Set<String> = []
        while rest.count > 1, let dash = rest.firstIndex(of: "-"), dash != rest.startIndex {
            let m = String(rest[..<dash])
            guard ["ctrl", "cmd", "alt", "shift"].contains(m) else { break }
            mods.insert(m)
            rest = rest[rest.index(after: dash)...]
        }
        return (mods, String(rest))
    }

    /// The spelling without Shift: `alt-shift-left` → `alt-left`. Used to
    /// find the move a Shift turns into a selection.
    static func withoutShift(_ spelling: String) -> String? {
        let (mods, key) = parts(spelling)
        guard mods.contains("shift") else { return nil }
        var s = ""
        if mods.contains("ctrl") { s += "ctrl-" }
        if mods.contains("cmd") { s += "cmd-" }
        if mods.contains("alt") { s += "alt-" }
        return s + key
    }

    /// How a key is shown in the help, the menus and the status bar, with
    /// the symbols of the Mac in their order (⌃⌥⇧⌘): `ctrl-shift-c` →
    /// `⌃⇧C`, `cmd-enter` → `⌘↩`, `J` → `J`, `shift-space` → `⇧space`.
    /// With a modifier, a letter is a capital, as in the menus of macOS.
    static func display(_ spelling: String) -> String {
        let (mods, key) = parts(spelling)
        var s = ""
        if mods.contains("ctrl") { s += "⌃" }
        if mods.contains("alt") { s += "⌥" }
        if mods.contains("shift") { s += "⇧" }
        if mods.contains("cmd") { s += "⌘" }
        let names: [String: String] = [
            "enter": "↩", "tab": "⇥", "esc": "esc", "backspace": "⌫", "del": "⌦", "up": "↑", "down": "↓",
            "left": "←", "right": "→", "home": "↖", "end": "↘", "pageup": "⇞", "pagedown": "⇟", "space": "space",
        ]
        if let n = names[key] {
            s += n
        } else if key.count > 1, key.hasPrefix("f") {
            s += key.uppercased()
        } else {
            s += mods.isEmpty ? key : key.uppercased()
        }
        return s
    }

    /// Whether the spelling names an arrow key (`up`, `shift-down`…): the
    /// help leaves them out when an action has another key.
    static func isArrow(_ spelling: String) -> Bool {
        ["up", "down", "left", "right"].contains(parts(spelling).key)
    }
}

// MARK: - The keymap

/// The keys in effect: the defaults with `keymap.toml` applied, as the core
/// sends them (`keymap_get`).
struct Keymap: Equatable {
    /// One key and its action (`none`: the key does nothing here, and hides
    /// the action of a parent context).
    struct Binding: Codable, Equatable {
        let key: String
        let action: String
    }

    /// `[menu.<name>]`.
    struct Menu: Codable, Equatable {
        let name: String
        let builtin: Bool
        let entries: [Binding]
    }

    /// The bindings of each context, in the order of the file.
    let contexts: [KeyContext: [Binding]]
    let menus: [Menu]
    /// `contexts` as dictionaries, built once: a key press is a lookup.
    private let lookup: [KeyContext: [String: String]]

    init(contexts: [KeyContext: [Binding]], menus: [Menu]) {
        self.contexts = contexts
        self.menus = menus
        // Swift note: `mapValues` keeps the keys and transforms each value;
        // `uniquingKeysWith` keeps the first binding if a key came twice
        // (the core never sends that).
        lookup = contexts.mapValues { Dictionary($0.map { ($0.key, $0.action) }, uniquingKeysWith: { a, _ in a }) }
    }

    /// From the JSON of `keymap_get` (or of the embedded defaults): the
    /// contexts by name, unknown names (a newer core) left out.
    init(contexts named: [String: [Binding]], menus: [Menu]) {
        var contexts: [KeyContext: [Binding]] = [:]
        for (name, list) in named {
            if let c = KeyContext(name: name) { contexts[c] = list }
        }
        self.init(contexts: contexts, menus: menus)
    }

    /// The action of `key` in the first context of `chain` that has it:
    /// `nil` when none has it, when a `none` hides it, or when this app
    /// does not know the action.
    func action(_ key: String, in chain: [KeyContext]) -> KeyAction? {
        guard !key.isEmpty else { return nil }
        for c in chain {
            if let a = lookup[c]?[key] { return KeyAction(rawValue: a) }
        }
        return nil
    }

    /// The keys that run `action` in `chain`, in file order: those of each
    /// context, unless a context before it in the chain takes the key.
    func keys(_ action: KeyAction, in chain: [KeyContext]) -> [String] {
        keys(raw: action.rawValue, in: chain)
    }

    /// The same for an action given by its name: `menu.<name>` of a menu of
    /// the user, which is not a `KeyAction`.
    func keys(raw action: String, in chain: [KeyContext]) -> [String] {
        var out: [String] = []
        for (i, c) in chain.enumerated() {
            for b in contexts[c] ?? [] where b.action == action {
                let hidden = chain[..<i].contains { lookup[$0]?[b.key] != nil }
                if !hidden, !out.contains(b.key) { out.append(b.key) }
            }
        }
        return out
    }

    /// The first key of `action` in `chain`, shown (`⌃I`), or `""`.
    func label(_ action: KeyAction, in chain: [KeyContext]) -> String {
        label(raw: action.rawValue, in: chain)
    }

    func label(raw action: String, in chain: [KeyContext]) -> String {
        keys(raw: action, in: chain).first.map(KeyChord.display) ?? ""
    }

    /// The menu of the user that `key` opens in `chain` (`menu.mine`), if it
    /// is one: its opener is not a `KeyAction`, the names being the user's.
    func userMenu(_ key: String, in chain: [KeyContext]) -> String? {
        guard !key.isEmpty else { return nil }
        for c in chain {
            guard let a = lookup[c]?[key] else { continue }
            guard a.hasPrefix("menu."), KeyAction(rawValue: a) == nil else { return nil }
            let name = String(a.dropFirst(5))
            return menu(name) != nil ? name : nil
        }
        return nil
    }

    /// The key of `action` in `chain`, for a message that cites it
    /// ("⇧⌘T for text only"): shown as the help shows it, or the action's
    /// name when no key runs it, so that the message still says what to
    /// use.
    func mention(_ action: KeyAction, in chain: [KeyContext]) -> String {
        let l = label(action, in: chain)
        return l.isEmpty ? action.rawValue : l
    }

    /// The chains of the messages that cite a key, whatever has the keys
    /// when they are shown.
    static let composeChain: [KeyContext] = [.compose, .everywhere]
    static let aiChain: [KeyContext] = [.aiPanel, .everywhere]
    static let pickerChain: [KeyContext] = [.picker, .lineEdit, .everywhere]
    static let mailChain: [KeyContext] = [.mail, .normal, .everywhere]

    func menu(_ name: String) -> Menu? {
        menus.first { $0.name == name }
    }

    /// The first key of the row `action` of the menu `name`, or `nil`.
    func menuKey(_ name: String, _ action: KeyAction) -> String? {
        menu(name)?.entries.first { $0.action == action.rawValue }?.key
    }

    /// The keymap of the defaults, embedded in the app
    /// (`Resources/keymap-defaults.json`, written by the core: `nicemailer
    /// keymap show --defaults --json`). In effect until the core answers,
    /// and in the tests.
    static let defaults: Keymap = {
        guard let url = Bundle.module.url(forResource: "keymap-defaults", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let file = try? JSONDecoder().decode(DefaultsFile.self, from: data)
        else { fatalError("keymap-defaults.json is missing from the app's resources") }
        return Keymap(contexts: file.contexts, menus: file.menus)
    }()

    /// The actions listed in the embedded defaults: the core's catalogue.
    static let catalogue: [String] = {
        guard let url = Bundle.module.url(forResource: "keymap-defaults", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let file = try? JSONDecoder().decode(DefaultsFile.self, from: data)
        else { return [] }
        return file.actions
    }()

    /// The shape of `keymap-defaults.json`.
    private struct DefaultsFile: Decodable {
        let actions: [String]
        let contexts: [String: [Binding]]
        let menus: [Menu]
    }

    /// A copy with `key` bound to `action` (`"none"` to remove it) in
    /// `context`, as a line of `keymap.toml` would: for the tests.
    func binding(_ key: String, to action: String, in context: KeyContext) -> Keymap {
        var c = contexts
        var list = c[context] ?? []
        if let i = list.firstIndex(where: { $0.key == key }) {
            list[i] = Binding(key: key, action: action)
        } else {
            list.append(Binding(key: key, action: action))
        }
        c[context] = list
        return Keymap(contexts: c, menus: menus)
    }

    /// A copy with the menu `name` replaced (or added): for the tests.
    func withMenu(_ name: String, _ entries: [(String, String)], builtin: Bool = false) -> Keymap {
        var m = menus.filter { $0.name != name }
        let menu = Menu(name: name, builtin: builtin, entries: entries.map { Binding(key: $0.0, action: $0.1) })
        if let i = menus.firstIndex(where: { $0.name == name }) { m.insert(menu, at: i) } else { m.append(menu) }
        return Keymap(contexts: contexts, menus: m)
    }
}

// MARK: - The menu bar

extension Keymap {
    /// The shortcut of a menu item of the menu bar for `action`: its first
    /// key in `[everywhere]`, or `nil` (no shortcut shown) when it has none
    /// there or the key cannot be a menu's (a function key alone).
    ///
    /// Swift note: `KeyboardShortcut` is SwiftUI's; `keyboardShortcut(nil)`
    /// on a button removes its shortcut.
    func shortcut(_ action: KeyAction) -> KeyboardShortcut? {
        guard let spelling = keys(action, in: [.everywhere]).first else { return nil }
        return Self.shortcut(spelling)
    }

    /// The shortcut of a spelling: `cmd-shift-a` → ⇧⌘A.
    static func shortcut(_ spelling: String) -> KeyboardShortcut? {
        let (mods, key) = KeyChord.parts(spelling)
        var m: EventModifiers = []
        if mods.contains("cmd") { m.insert(.command) }
        if mods.contains("ctrl") { m.insert(.control) }
        if mods.contains("alt") { m.insert(.option) }
        if mods.contains("shift") { m.insert(.shift) }
        let named: [String: KeyEquivalent] = [
            "enter": .return, "tab": .tab, "space": .space, "esc": .escape, "backspace": .delete,
            "del": .deleteForward, "up": .upArrow, "down": .downArrow, "left": .leftArrow, "right": .rightArrow,
            "home": .home, "end": .end, "pageup": .pageUp, "pagedown": .pageDown,
        ]
        if let k = named[key] { return KeyboardShortcut(k, modifiers: m) }
        guard key.count == 1, let c = key.first else { return nil }
        // A capital without ⌃ ⌥ ⌘ (`J`) is Shift on its letter.
        if c.isUppercase {
            m.insert(.shift)
            return KeyboardShortcut(KeyEquivalent(Character(c.lowercased())), modifiers: m)
        }
        return KeyboardShortcut(KeyEquivalent(c), modifiers: m)
    }
}

/// What `keymap_get` returns.
struct KeymapView: Decodable {
    let path: String
    let exists: Bool
    let contexts: [String: [Keymap.Binding]]
    let menus: [Keymap.Menu]
    let error: KeymapProblem?
    let warnings: [KeymapProblem]
}

/// A problem of `keymap.toml`: why it is refused, or a warning.
struct KeymapProblem: Decodable, Equatable {
    let file: String?
    let line: Int?
    let key: String?
    let message: String

    /// "keymap.toml, line 3: mail.d: …", as the core writes it.
    var text: String {
        var s = file ?? "keymap.toml"
        if let line { s += ", " + L("line %lld", line) }
        s += ": "
        if let key { s += key + ": " }
        return s + message
    }
}
