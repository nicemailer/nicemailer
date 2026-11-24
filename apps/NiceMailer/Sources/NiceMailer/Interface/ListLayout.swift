import AppKit
import SwiftUI

/// How the list of mails shows each conversation (`ui.md`, "Liste",
/// "Deux présentations"; `v` `l` switches from one to the other).
///
/// - `columns`: one line per mail, in columns (`ThreadRow`), as the list
///   has always been: dot, date, sender, paperclip, subject, count, flag.
/// - `stacked`: three lines per mail, as Apple Mail (`StackedThreadRow`):
///   the sender with the date on the right, then the subject, then the
///   beginning of the latest message. The list column can then be much
///   narrower than in columns, where the subject shares its line.
///
/// The theme gives the first one (`window.list_layout`: columns in geek,
/// stacked in clean and regular); once the user chooses with `v` `l`, the
/// app remembers it (`InterfaceController.listLayout`), as the number of
/// columns, and each presentation keeps its own width of the list. Pure
/// functions here, tested on their own (`ListLayoutTests`).
///
/// Swift note: an enum with a `String` raw value converts to and from the
/// text stored in `UserDefaults` (`ListLayout(rawValue:)`), and gives `nil`
/// for a text it does not know, which the caller replaces by the default.
enum ListLayout: String, CaseIterable {
    case columns
    case stacked

    /// Where the choice is remembered (`AppDefaults.store`), next to
    /// `geek.columns`.
    static let defaultsKey = "geek.listLayout"

    /// Where the width of the list in this presentation is remembered:
    /// columns keeps the key it always had, so an existing width stays.
    var fractionKey: String {
        switch self {
        case .columns: "geek.listFraction"
        case .stacked: "geek.listFraction.stacked"
        }
    }

    /// Width of the list, as a fraction of the window, until the user drags
    /// the separator: "un petit tiers" in columns, where the subject
    /// shares its line with the date, the sender and the count; a quarter
    /// in stacked, about Apple Mail's list (340 pt in a window of 1,400),
    /// since the subject has a line of its own.
    var defaultFraction: Double {
        switch self {
        case .columns: 0.32
        case .stacked: 0.24
        }
    }

    /// The other presentation: `v` `l` goes from one to the other.
    var toggled: ListLayout { self == .columns ? .stacked : .columns }

    /// What the status bar says once switched.
    var flash: String {
        switch self {
        case .columns: L("List: one line per mail")
        case .stacked: L("List: three lines per mail")
        }
    }

    /// Room above the first line and below the last one of a stacked row,
    /// written for a 12 pt text.
    static let stackedPadding: CGFloat = 7

    /// Height of a stacked row: three lines of `lineHeight` (the
    /// interface font of the theme at the current size), and
    /// `stackedPadding` (already scaled) above and below. Rounded up to a
    /// whole point, so that the rows of the list do not blur on fractions.
    static func stackedRowHeight(lineHeight: CGFloat, padding: CGFloat) -> CGFloat {
        (3 * lineHeight + 2 * padding).rounded(.up)
    }

    /// Height of a row of the list in `layout`, in the theme and at the
    /// text size in effect: the theme's (`layout.row_height`) in columns,
    /// three lines of text in stacked. `ctrl-d` / `ctrl-u` count the rows
    /// that fit with it (`InterfaceController.visibleRows`).
    @MainActor
    static func rowHeight(_ layout: ListLayout) -> CGFloat {
        switch layout {
        case .columns: return Look.rowHeight
        case .stacked:
            // The height AppKit gives a line of that font: what a `Text`
            // of one line takes in SwiftUI too.
            let font = Fonts.scaled(RowLayout.fontSize)
            let line = NSLayoutManager().defaultLineHeight(for: font)
            return stackedRowHeight(lineHeight: line, padding: scaled(stackedPadding))
        }
    }

    /// The third line of a stacked row: the beginning of the latest
    /// message (`ThreadSummary.snippet`), on one line. The core keeps the
    /// line breaks and runs of spaces of the text; on a single line they
    /// would leave holes, so every run of white space (line breaks
    /// included) becomes one space. `nil` or blank: an empty line, so that
    /// every row keeps the same height.
    static func snippetLine(_ snippet: String?) -> String {
        guard let snippet else { return "" }
        return snippet.split(whereSeparator: { $0.isWhitespace || $0.isNewline }).joined(separator: " ")
    }
}

/// A conversation of the list on three lines (`ListLayout.stacked`):
///
/// 1. the unread dot, the person (bold while unread, "→" when the user
///    wrote last, the icon person / robot in Quarantine), then on the
///    right the paperclip, the flag and the date;
/// 2. the subject and its tags, then on the right the folder (in
///    "Everything"), the AI's suggestion and the number of messages;
/// 3. the beginning of the latest message, muted.
///
/// The same marks as in columns (`ThreadRow`), in the same colours, so that
/// switching presentation changes where things are, never what they mean.
struct StackedThreadRow: View {
    let thread: ThreadSummary
    let cursor: Bool
    let marked: Bool
    /// The widths of the list in columns: the unread dot and the gap
    /// between marks are the same in both presentations.
    let layout: RowLayout
    /// Its place in the list, for the alternate rows (`layout.zebra`).
    var index = 0
    /// Folder of the row, shown in "Everything" only (`nil` elsewhere).
    var folder: AppModel.FolderLabel? = nil
    /// The icon person / robot before the name (Quarantine only).
    var showsKind = false
    /// The target the AI suggests for the conversation (`a` `s`).
    var suggestion: AISuggestion? = nil
    /// The names of the folders, for the suggestion (`FolderNames`).
    var names: FolderNames = .english

    var body: some View {
        // The row under the cursor on a fill of the accent (regular): every
        // text and icon in the selection's text color (`Look.ink`).
        let on = cursor
        // The unread dot has the column it has in columns (`RowLayout.dot`);
        // the two other lines start under the name, as in Apple Mail.
        let dot = layout.dot
        let gap = layout.char
        HStack(spacing: 0) {
            (marked ? Palette.markedBar : Color.clear).frame(width: 2)
            VStack(alignment: .leading, spacing: 0) {
                // 1. Who, and when.
                HStack(spacing: gap) {
                    Text(thread.unreadCount > 0 ? RowLayout.unreadDot : " ")
                        .foregroundStyle(Look.ink(Palette.unread, selected: on))
                        .frame(width: dot)
                    if showsKind, let symbol = SenderColumn.kindSymbol(thread) {
                        Image(systemName: symbol)
                            .font(.system(size: scaled(10), weight: .medium))
                            .foregroundStyle(Look.ink(thread.correspondentKind == "person" ? Palette.accentText : Palette.muted, selected: on))
                            .accessibilityHidden(true)
                    }
                    Text(SenderColumn.text(thread))
                        .fontWeight(thread.unreadCount > 0 ? .bold : .regular)
                        .foregroundStyle(Look.ink(Palette.text, selected: on))
                        .accessibilityLabel(SenderColumn.spoken(thread, showsKind: showsKind))
                    Spacer(minLength: gap)
                    if thread.hasAttachments {
                        Image(systemName: "paperclip")
                            .font(.system(size: scaled(11), weight: .medium))
                            .foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                            .accessibilityLabel(L("with attachments"))
                    }
                    if thread.isFlagged {
                        Image(systemName: "flag.fill")
                            .font(.system(size: scaled(11), weight: .semibold))
                            .foregroundStyle(Look.ink(Palette.flag, selected: on))
                            .accessibilityLabel(L("flagged"))
                            .uiTarget("flag-\(thread.id)")
                    }
                    Text(Format.geekListDate(thread.dateLastReceived))
                        .foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                        .fixedSize()
                }
                // 2. What about.
                HStack(spacing: gap) {
                    Text(thread.subject.isEmpty ? L("(no subject)") : thread.subject).layoutPriority(1)
                        .foregroundStyle(Look.ink(Palette.text, selected: on))
                    ForEach(thread.tags, id: \.self) { Text("#\($0)").foregroundStyle(Look.ink(Palette.accentText, selected: on)) }
                    Spacer(minLength: 0)
                    if let folder {
                        Text(folder.folder).foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                            .fixedSize()
                    }
                    if let s = suggestion {
                        Text(AI.suggestionTag(names.name(s.target), narrow: true))
                            .foregroundStyle(Look.ink(Palette.accentText, selected: on))
                            .fixedSize()
                            .accessibilityLabel(L("AI suggests %@", names.name(s.target)))
                    }
                    if thread.messageCount > 1 {
                        Text("\(thread.messageCount)").foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                            .fixedSize()
                    }
                }
                .padding(.leading, dot + gap)
                // 3. How it begins.
                Text(ListLayout.snippetLine(thread.snippet))
                    .foregroundStyle(Look.ink(Palette.muted, selected: on, faint: true))
                    .padding(.leading, dot + gap)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .uiTarget("snippet-\(thread.id)")
            }
            .padding(.leading, scaled(CGFloat(Look.theme.layout.rowInset)) + scaled(4))
            // Inside the fill of the selected row, as in columns
            // (`RowLayout.trailingPadding`).
            .padding(.trailing, RowLayout.trailingPadding)
            .padding(.vertical, scaled(ListLayout.stackedPadding))
        }
        .font(.interface(RowLayout.fontSize, weight: .regular))
        .lineLimit(1)
        .frame(height: ListLayout.rowHeight(.stacked), alignment: .top)
        .rowFill(cursor ? Palette.cursor : marked ? Palette.marked : nil, index: index)
    }
}
