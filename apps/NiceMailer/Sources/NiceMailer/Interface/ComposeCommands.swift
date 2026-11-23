import SwiftUI

/// The commands of the reply shown under its subject (`reponse.md`,
/// section 6; `docs/sending.md`): send, send then file, attach files, the
/// address it is sent from, the format, keep (Escape) and abandon. Each is a button that
/// reminds its key; a click runs it as the key does
/// (`InterfaceController.runComposeCommand`).
///
/// Swift note: `CaseIterable` gives `allCases`, every case in the order
/// written, which is the order of the buttons.
enum ComposeCommand: String, CaseIterable {
    case send
    case sendThen = "send-then"
    case attach
    case from
    case format
    case keep
    case abandon

    /// The action of `keymap.toml` behind it; `nil` for keep, which is
    /// Escape, a key no keymap changes.
    var action: KeyAction? {
        switch self {
        case .send: .composeSend
        case .sendThen: .composeFileAndSend
        case .attach: .composeAttach
        case .from: .composeFrom
        case .format: .composeTogglePlain
        case .keep: nil
        case .abandon: .composeAbandon
        }
    }

    /// Its key as `keymap` gives it now (⌘↩, ⇧⌘↩…): a key changed in
    /// `keymap.toml` shows here at once. Escape is written as every other
    /// key of the app writes it (`KeyChord.display`, "esc", as in "⌘esc"
    /// of abandon), not in words of its own.
    func key(_ keymap: Keymap) -> String {
        guard let action else { return KeyChord.display("esc") }
        return keymap.mention(action, in: Keymap.composeChain)
    }

    /// The SF Symbol of the button when the line is too narrow for the
    /// words: the icon says what the command does, which a key alone does
    /// not (the key goes to the tooltip).
    var icon: String {
        switch self {
        case .send: "paperplane"
        case .sendThen: "tray.and.arrow.down"
        case .attach: "paperclip"
        case .from: "person.crop.circle"
        case .format: "textformat"
        case .keep: "arrow.uturn.backward"
        case .abandon: "trash"
        }
    }

    /// The word on the button.
    var name: String {
        switch self {
        case .send: L("send")
        case .sendThen: L("send then…")
        case .attach: L("attach")
        case .from: L("from")
        case .format: L("format")
        case .keep: L("keep")
        case .abandon: L("abandon")
        }
    }

    /// What it does, in the tooltip: the short word on the button is a
    /// reminder for whoever knows it, the sentence explains it.
    var help: String {
        switch self {
        case .send: L("Send the mail")
        case .sendThen: L("Choose where the conversation goes, then send")
        case .attach: L("Attach files to the mail")
        case .from: L("Choose the address the mail is sent from")
        case .format: L("Switch between text only and formatted text; the next mails start in the mode chosen")
        case .keep: L("Back to the mail; the reply is kept in Drafts")
        case .abandon: L("Abandon the reply, without keeping a draft")
        }
    }
}

/// One command of the reply as a button: its key in the accent color, then
/// its word, or its icon alone when the line is too narrow (`compact`), in
/// a thin frame rounded as the other buttons of the title line
/// (`Look.titleControlRadius`: square in geek). The frame says it can be
/// clicked; the pointer over it fills it, as a button of a toolbar, and the
/// tooltip says what it does.
struct ComposeCommandButton: View {
    let command: ComposeCommand
    let key: String
    /// Its icon alone, without key nor word: the line is too narrow for
    /// them. The key is then in the tooltip.
    var compact = false
    let action: () -> Void

    var body: some View {
        FramedButton(help: "\(command.help) (\(key))", accessibility: command.help, action: action) {
            if compact {
                // Every icon in a box of the same size: the symbols have
                // widths of their own ("Aa" is narrower than the paper
                // plane), and the frames must make one row of equal
                // squares.
                Image(systemName: command.icon)
                    .foregroundStyle(Palette.muted)
                    .frame(width: scaled(18), height: scaled(16))
            } else {
                HStack(spacing: scaled(3)) {
                    Text(key).foregroundStyle(Palette.accentText)
                    Text(command.name).foregroundStyle(Palette.muted)
                }
            }
        }
        .uiTarget("compose-command-\(command.rawValue)")
    }
}

/// The frame of the small buttons of the reply (its commands, the Bcc
/// button): a thin line rounded as the buttons of the reader's title line
/// (`Look.titleControlRadius`: square in geek), room inside so that the
/// text breathes, filled while the pointer is over it or while it is on
/// (`selected`), and a tooltip. The line says it can be clicked; the
/// tooltip says what it does.
struct FramedButton<Label: View>: View {
    /// The tooltip: what a click does, and its key.
    let help: String
    /// What VoiceOver says: the same sentence, without the key.
    let accessibility: String
    /// On: the Bcc line shown. Filled with the selection color.
    var selected = false
    let action: () -> Void
    @ViewBuilder let label: () -> Label

    /// The pointer is over the button.
    ///
    /// Swift note: `@State` is storage SwiftUI keeps for this view across
    /// redraws; changing it redraws the view.
    @State private var hovered = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Look.titleControlRadius, style: .continuous)
        label()
            .lineLimit(1)
            .fixedSize()
            // More room on the sides than above and below, as a button of
            // a macOS toolbar.
            .padding(.horizontal, scaled(6))
            .padding(.vertical, scaled(2))
            .background(hovered || selected ? Palette.cursor : Color.clear, in: shape)
            .overlay(shape.stroke(Palette.separator, lineWidth: 1))
            .contentShape(Rectangle())
            .onHover { hovered = $0 }
            .help(help)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(accessibility)
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .onTapGesture(perform: action)
    }
}
