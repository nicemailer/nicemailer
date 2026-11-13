import SwiftUI

/// A typed line (`LineEdit`) drawn with its caret: a thin bar between two
/// letters that takes no room, as in every macOS text field.
///
/// The caret used to be a character, "▏" (U+258F, left one-eighth block),
/// put between the two halves of the text. In the monospaced geek font
/// that character is as wide as a letter, with its bar on the left edge of
/// its cell: when the cursor was moved back into a word, the rest of the
/// cell showed as a gap between the caret and the next letter, and the
/// text after the cursor shifted by one column. Here the bar is drawn over
/// the text instead, so the letters stay exactly where they would be
/// without it (`CaretTextTests` checks the width).
///
/// The text is one `Text`, not two halves side by side: SwiftUI rounds
/// the width of each view to the pixel, so a split line would still move
/// the letters after the cursor by up to a pixel as the cursor moves. Only
/// the caret's own position is rounded here.
struct CaretText: View {
    let line: LineEdit

    var body: some View {
        if line.selectedAll {
            // A whole-text selection (a recipient taken back with ⌫, the
            // current words of Subject…): highlighted, with no caret,
            // since typing or ⌫ replaces all of it at once.
            Text(line.text).background(Palette.cursor)
        } else {
            caret
        }
    }

    private var caret: some View {
        // A zero-width space (U+200B) has no width but the height of a
        // line of the current font: an empty line keeps its height, and the
        // caret is as tall as the letters around it, whatever the text size.
        Text(line.text + Self.zeroWidth)
            .overlay(alignment: .leading) {
                // Where the caret goes: the text before the cursor, laid
                // out in the same font but not drawn; the bar sits on its
                // right edge.
                //
                // Swift note: `.hidden()` keeps a view's size in the layout
                // without drawing it; the overlay added after it is a new
                // view, drawn normally. A `Shape` with no fill is painted in
                // the current foreground style, so the caret takes the
                // colour of the text. An overlay is not clipped to the view
                // it covers: centred on the edge by the offset, the bar
                // spills half its width onto each neighbouring letter.
                Text(line.beforeCursor + Self.zeroWidth)
                    .hidden()
                    .overlay(alignment: .trailing) {
                        Rectangle()
                            .frame(width: Self.width)
                            .offset(x: Self.width / 2)
                    }
            }
    }

    private static let zeroWidth = "\u{200B}"

    /// Width of the bar, in points: that of the macOS text field caret at
    /// the default text size, growing with ⌘+ like the letters.
    static var width: CGFloat { max(1, scaled(1.5)) }
}
