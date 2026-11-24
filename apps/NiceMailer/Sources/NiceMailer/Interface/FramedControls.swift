import SwiftUI

// The small buttons of the title lines, the same in the reader and in a
// reply (`ui.md`, "Boutons de l'en-tête"): `FramedButton` for one action,
// `FramedSegments` for a choice among a few (text | html | images). Each is
// framed by a thin rounded line, as high as the others, and filled while
// the pointer is over it.

/// Room inside a framed button or segment, written for a 12 pt text: more
/// on the sides than above and below, as a button of a macOS toolbar.
enum FramedControl {
    static let horizontalPadding: CGFloat = 6
    static let verticalPadding: CGFloat = 2
}

/// A small button of a title line: the commands of a reply and its Bcc
/// button, and in the reader unsubscribe and the flag (`ui.md`, "Boutons
/// de l'en-tête"). One design for reading and writing: a thin line rounded
/// as the rows of the list, for its size (`Look.titleControlRadius`:
/// square in geek), room inside so that the text breathes, filled while
/// the pointer is over it or while it is on (`selected`), and a tooltip.
/// The line says it can be clicked; the tooltip says what it does.
struct FramedButton<Label: View>: View {
    /// The tooltip: what a click does, and its key.
    let help: String
    /// What VoiceOver says: the same sentence, without the key.
    let accessibility: String
    /// On: the Bcc line shown, the mail flagged. Filled with the selection
    /// color.
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
            .padding(.horizontal, scaled(FramedControl.horizontalPadding))
            .padding(.vertical, scaled(FramedControl.verticalPadding))
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

/// A choice among a few words in one frame, as a segmented control of
/// macOS: the text | html | images switch of the reader. Each word is a
/// segment, clicked on its own, with the room of a `FramedButton`; the
/// current one is filled with the selection color (its word in the
/// selection's text color, white on the accent of regular: `Look.ink`), a
/// segment under the pointer with the same color, and a thin line sets the
/// segments apart.
struct FramedSegments: View {
    struct Segment: Identifiable {
        /// Its name, for the UI scripts (`format-<id>`) and `current`.
        let id: String
        let label: String
        /// The tooltip: what a click does, and its key.
        let help: String
    }

    let segments: [Segment]
    /// The id of the segment that is on.
    let current: String
    let choose: (String) -> Void

    /// The segment under the pointer, if any.
    @State private var hovered: String?

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Look.titleControlRadius, style: .continuous)
        HStack(spacing: 0) {
            ForEach(Array(segments.enumerated()), id: \.element.id) { i, s in
                if i > 0 { Palette.separator.frame(width: 1) }
                let on = s.id == current
                Text(s.label)
                    .foregroundStyle(Look.ink(Palette.muted, selected: on))
                    .lineLimit(1)
                    .fixedSize()
                    .padding(.horizontal, scaled(FramedControl.horizontalPadding))
                    .padding(.vertical, scaled(FramedControl.verticalPadding))
                    .frame(maxHeight: .infinity)
                    .background(on || hovered == s.id ? Palette.cursor : Color.clear)
                    .contentShape(Rectangle())
                    .onHover { inside in
                        if inside { hovered = s.id } else if hovered == s.id { hovered = nil }
                    }
                    .help(s.help)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(s.label)
                    .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
                    .onTapGesture { choose(s.id) }
                    .uiTarget("format-\(s.id)")
            }
        }
        .fixedSize()
        // The fills follow the rounded corners of the frame.
        .clipShape(shape)
        .overlay(shape.stroke(Palette.separator, lineWidth: 1))
    }
}
