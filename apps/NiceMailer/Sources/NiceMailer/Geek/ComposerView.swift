import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The reply being written, in place of the reader (`reponse.md`,
/// section 6): who it goes to, its subject, where the conversation goes
/// once it is sent, then the text, typed with the caret drawn in it.
///
/// Nothing here is editable by AppKit: the keys are read and applied by
/// `GeekController.handleComposeKey` on the model's `TextBuffer`, and this
/// view only draws the result. That keeps every key testable without a
/// window, and no key can be eaten by a view holding the focus.
struct ComposerView: View {
    @Environment(AppModel.self) private var model
    let controller: GeekController

    var body: some View {
        if let c = model.composer {
            VStack(alignment: .leading, spacing: 0) {
                ReaderTitleBar(subject: c.headers.subject) {
                    Text(L("⌘↩ send · ⇧⌘↩ send then… · ⌥⌘F from · Esc keep · ⌘⌫ abandon")).foregroundStyle(Palette.muted).lineLimit(1)
                }
                Palette.separator.frame(height: 1)
                VStack(alignment: .leading, spacing: scaled(2)) {
                    // Who sends, and from which account; a click (or ⌥⌘F)
                    // chooses another address of an account that can send.
                    //
                    // While "Send from" is open it takes this line's
                    // place: the answer is read where the address it
                    // changes is, in the headers, not in a corner of the
                    // window (`reponse.md`, section 4 ter).
                    if controller.activeChoice == .sendFrom {
                        ChoicePanel(controller: controller, choice: .sendFrom)
                            .padding(.vertical, scaled(2))
                    } else {
                        headerLine(L("From"), focused: false) {
                            Text(Compose.fromLine(c.headers, senders: model.senders)).lineLimit(1)
                        }
                        .contentShape(Rectangle())
                        .onTapGesture { controller.chooseSender() }
                        .uiTarget("compose-from")
                    }
                    recipientLine(.to, L("To"), c)
                    recipientLine(.cc, L("Cc"), c)
                    if c.showsBcc {
                        recipientLine(.bcc, L("Bcc"), c)
                    }
                    headerLine(L("Subject"), focused: c.focus == .subject) {
                        if c.focus == .subject {
                            CaretText(line: c.subjectLine).lineLimit(1)
                        } else {
                            Text(c.headers.subject).lineLimit(1).truncationMode(.tail)
                        }
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { controller.focusCompose(.subject) }
                    .uiTarget("compose-subject")
                    if !c.attachments.isEmpty {
                        headerLine(L("Files"), focused: c.focus == .attachments) {
                            FlowLayout(spacing: scaled(4)) {
                                ForEach(Array(c.attachments.enumerated()), id: \.element.id) { i, a in
                                    let selected = c.focus == .attachments && i == c.attachmentIndex
                                    Label(Compose.attachmentLabel(a), systemImage: "paperclip")
                                        .lineLimit(1)
                                        .padding(.horizontal, scaled(4))
                                        .background(RoundedRectangle(cornerRadius: 3).fill(selected ? Palette.accent.opacity(0.45) : Palette.cursor))
                                        .contentShape(Rectangle())
                                        .onTapGesture { controller.clickAttachment(i) }
                                        .uiTarget("compose-file-\(i)")
                                }
                            }
                        }
                    }
                    // ⇧⌘↩, like ⌥⌘F, is answered in the headers and not
                    // in a corner of the window: its panel takes the place
                    // of the line "After sending: …", which is what it
                    // changes (`reponse.md`, section 8).
                    if controller.activeChoice == .sendThen {
                        ChoicePanel(controller: controller, choice: .sendThen)
                            .padding(.top, scaled(2))
                    } else if let after = Compose.afterSendLine(c.draft) {
                        Text(after).foregroundStyle(Palette.accentText).lineLimit(1)
                    }
                }
                .font(.geek(12))
                .padding(.horizontal, 12)
                .padding(.vertical, scaled(6))
                Palette.separator.frame(height: 1)
                ComposerTextView(text: c.buffer.text, cursor: c.buffer.utf16Cursor, selection: c.buffer.utf16Selection,
                                 focused: c.focus == .body, controller: controller)
                    .uiTarget("compose-text")
            }
            // Files dropped anywhere on the reply are attached.
            //
            // Swift note: `onDrop` hands `NSItemProvider`s; each file URL is
            // loaded asynchronously, then all are attached together.
            .onDrop(of: [.fileURL], isTargeted: nil) { providers in
                Task { @MainActor in
                    var urls: [URL] = []
                    for p in providers {
                        if let url = try? await p.loadItem(forTypeIdentifier: UTType.fileURL.identifier) as? Data,
                           let u = URL(dataRepresentation: url, relativeTo: nil) {
                            urls.append(u)
                        }
                    }
                    await model.attach(urls)
                }
                return true
            }
        }
    }

    /// A line of the headers: its label (in the accent colour while it has
    /// the keys), then its content.
    private func headerLine(_ label: String, focused: Bool, @ViewBuilder content: () -> some View) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: scaled(6)) {
            Text(label)
                .foregroundStyle(focused ? Palette.accentText : Palette.muted)
                .frame(width: scaled(52), alignment: .leading)
            content()
            Spacer(minLength: 0)
        }
    }

    /// To, Cc or Bcc: an address per tag, then, while the line has the
    /// keys, the text typed with its caret, and the suggestions under it.
    /// A click gives the line the keys; a click on an address takes that
    /// one back into the line, selected, to remove or correct it
    /// (`reponse.md`, section 6, "Corriger").
    @ViewBuilder private func recipientLine(_ field: ComposeField, _ label: String, _ c: Composer) -> some View {
        let focused = c.focus == field
        headerLine(label, focused: focused) {
            FlowLayout(spacing: scaled(4)) {
                ForEach(Array(c.headers[field].enumerated()), id: \.offset) { i, a in
                    Text(Compose.label(a))
                        .lineLimit(1)
                        .padding(.horizontal, scaled(4))
                        .background(RoundedRectangle(cornerRadius: 3).fill(Palette.cursor))
                        .contentShape(Rectangle())
                        .onTapGesture { controller.clickRecipient(field, i) }
                        .uiTarget("compose-\(field.rawValue)-\(i)")
                }
                if focused {
                    CaretText(line: c.line).lineLimit(1)
                }
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { controller.focusCompose(field) }
        .uiTarget("compose-\(field.rawValue)")
        if focused, !c.suggestions.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(c.suggestions.enumerated()), id: \.offset) { i, a in
                    Text(Compose.label(a))
                        .lineLimit(1)
                        .padding(.horizontal, scaled(4))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(i == c.suggestionIndex ? Palette.cursor : Color.clear)
                        .contentShape(Rectangle())
                        .onTapGesture { model.chooseSuggestion(i) }
                        .uiTarget("suggestion-\(i)")
                }
            }
            .padding(scaled(2))
            .background(RoundedRectangle(cornerRadius: 4).fill(Palette.popup))
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(Palette.separator))
            .padding(.leading, scaled(58))
            .frame(maxWidth: scaled(520), alignment: .leading)
        }
    }
}

/// Lays its children out left to right, going to the next line when the
/// width is used up: the addresses of a recipient line.
///
/// Swift note: `Layout` (macOS 13) is SwiftUI's protocol for a container
/// that places its children itself: `sizeThatFits` measures, then
/// `placeSubviews` puts each child at its point.
struct FlowLayout: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(subviews, width: proposal.width ?? .infinity)
        let width = rows.map { $0.width }.max() ?? 0
        let height = rows.map { $0.height }.reduce(0, +) + spacing / 2 * CGFloat(max(0, rows.count - 1))
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(subviews, width: bounds.width) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(width: min(size.width, bounds.width), height: size.height))
                x += size.width + spacing
            }
            y += row.height + spacing / 2
        }
    }

    private struct Row {
        var items: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows: [Row] = [Row()]
        for (i, view) in subviews.enumerated() {
            let size = view.sizeThatFits(.unspecified)
            let needed = rows[rows.count - 1].items.isEmpty ? size.width : rows[rows.count - 1].width + spacing + size.width
            if needed > width, !rows[rows.count - 1].items.isEmpty {
                rows.append(Row())
            }
            var row = rows[rows.count - 1]
            row.width = row.items.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
            rows[rows.count - 1] = row
        }
        return rows
    }
}

/// The text of the reply, drawn by AppKit (wrapping, long texts, the
/// position of each letter) with a caret of its own.
struct ComposerTextView: NSViewRepresentable {
    let text: String
    /// The caret, in UTF-16 units (AppKit's).
    let cursor: Int
    /// The text selected, in UTF-16 units; drawn highlighted, without caret.
    let selection: NSRange?
    /// The text has the keys: its caret is drawn.
    var focused = true
    let controller: GeekController

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        let view = ComposerNSTextView(frame: .zero)
        // Not editable: the keys are the controller's. Selectable, so that
        // the view has a selection of its own, kept equal to the reply's,
        // for the accessibility API and the Services (see
        // `ComposerNSTextView`); its own highlight is transparent, the
        // reply's is drawn below.
        view.isEditable = false
        view.isSelectable = true
        view.selectedTextAttributes = [.backgroundColor: NSColor.clear]
        view.isRichText = false
        view.drawsBackground = false
        view.textContainerInset = NSSize(width: 12, height: 12)
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.autoresizingMask = [.width]
        view.textContainer?.widthTracksTextView = true
        view.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        scroll.documentView = view
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? ComposerNSTextView else { return }
        controller.composerView = view
        view.onClick = { [weak controller] offset, extend in controller?.clickCompose(utf16: offset, extend: extend) }
        view.onReplaceSelection = { [weak controller] text in controller?.replaceComposeSelection(text) }
        view.onSelect = { [weak controller] range in controller?.selectCompose(utf16: range) }
        let font = Fonts.scaled(14)
        // The text is set again only when it changed: a move of the caret
        // alone redraws the caret, not the whole layout.
        if view.string != text || view.font != font {
            let paragraph = NSMutableParagraphStyle()
            paragraph.lineSpacing = 2
            view.textStorage?.setAttributedString(NSAttributedString(string: text, attributes: [
                .font: font, .foregroundColor: NSColor(Palette.text), .paragraphStyle: paragraph,
            ]))
            view.font = font
        }
        let length = (text as NSString).length
        view.caret = min(cursor, length)
        view.caretColor = NSColor(Palette.accentText)
        // The selection, highlighted with a temporary attribute of the
        // layout: it changes no character, and no layout is redone.
        // While something is selected, there is no caret, as in macOS.
        if let lm = view.layoutManager {
            lm.removeTemporaryAttribute(.backgroundColor, forCharacterRange: NSRange(location: 0, length: length))
            if let sel = selection, NSMaxRange(sel) <= length {
                lm.addTemporaryAttribute(.backgroundColor, value: NSColor(Palette.cursor), forCharacterRange: sel)
            }
        }
        view.showsCaret = selection == nil && focused
        // The view's own selection follows the reply's, and the view is the
        // window's focused element: what an app such as Kerlig reads.
        let own = selection.flatMap { NSMaxRange($0) <= length ? $0 : nil } ?? NSRange(location: view.caret, length: 0)
        if view.selectedRange() != own { view.setSelectedRange(own) }
        if let window = view.window, window.firstResponder !== view { window.makeFirstResponder(view) }
        view.needsDisplay = true
        view.scrollToCaret()
        if UIScript.isActive { UIScript.composeLines = view.drawnLines }
    }
}

/// A text view that draws a caret where it is told, without being edited
/// nor having the focus.
final class ComposerNSTextView: NSTextView {
    /// Where the caret is, in UTF-16 units.
    var caret = 0
    var caretColor: NSColor = .textColor
    /// No caret while text is selected.
    var showsCaret = true
    /// A click in the text: its position, in UTF-16 units, and whether
    /// Shift was held (the selection extends to there).
    var onClick: ((Int, Bool) -> Void)?

    /// The rectangle of the caret: on the left edge of the character at
    /// `caret`, as tall as its line; after the last character, on the
    /// extra line fragment of a text ending with a line break.
    func caretRect() -> NSRect? {
        guard let lm = layoutManager, let tc = textContainer else { return nil }
        let length = (string as NSString).length
        let origin = textContainerOrigin
        if length == 0 || caret >= length {
            // At the very end: after the last glyph, or on the empty last
            // line when the text ends with a line break.
            if lm.extraLineFragmentTextContainer != nil, length == 0 || (string as NSString).character(at: length - 1) == 10 {
                let r = lm.extraLineFragmentRect
                return NSRect(x: r.minX + origin.x + tc.lineFragmentPadding, y: r.minY + origin.y, width: 1, height: r.height)
            }
            let glyph = lm.glyphIndexForCharacter(at: max(0, length - 1))
            let r = lm.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: tc)
            return NSRect(x: r.maxX + origin.x, y: r.minY + origin.y, width: 1, height: r.height)
        }
        let glyph = lm.glyphIndexForCharacter(at: caret)
        var line = lm.lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
        let x = lm.location(forGlyphAt: glyph).x
        line.origin.x += origin.x
        line.origin.y += origin.y
        return NSRect(x: line.minX + x, y: line.minY, width: 1, height: line.height)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard showsCaret, let r = caretRect() else { return }
        caretColor.setFill()
        NSRect(x: r.minX - 0.75, y: r.minY, width: max(1, scaled(1.5)), height: r.height).fill()
    }

    func scrollToCaret() {
        guard let r = caretRect() else { return }
        scrollToVisible(r.insetBy(dx: -4, dy: -scaled(14)))
    }

    /// The position one drawn line above (`delta` -1) or below (+1) the
    /// caret, at the horizontal position `goalX` (the caret's own when
    /// nil), and that position; nil at either end of the text.
    func verticalMove(from caret: Int, delta: Int, goalX: CGFloat?) -> (Int, CGFloat)? {
        let saved = self.caret
        self.caret = caret
        defer { self.caret = saved }
        guard let lm = layoutManager, let tc = textContainer, let r = caretRect() else { return nil }
        let x = goalX ?? r.minX
        let origin = textContainerOrigin
        let length = (string as NSString).length
        // The bottom of the text as laid out, its empty last line included.
        var bottom = lm.usedRect(for: tc).maxY + origin.y
        if lm.extraLineFragmentTextContainer != nil { bottom = max(bottom, lm.extraLineFragmentRect.maxY + origin.y) }
        // On the first line ↑ goes to the start, on the last ↓ to the
        // end, as in every macOS text view.
        if delta < 0, r.minY - 2 < origin.y { return (0, x) }
        if delta > 0, r.maxY + 2 > bottom { return (length, x) }
        let y = delta < 0 ? r.minY - 2 : r.maxY + 2
        let i = characterIndexForInsertion(at: NSPoint(x: x, y: y))
        return (min(max(0, i), length), x)
    }

    /// Lines of the text as drawn (a wrapped paragraph counts its lines),
    /// for the UI scripts.
    var drawnLines: Int {
        guard let lm = layoutManager else { return 0 }
        var lines = 0
        var index = 0
        let glyphs = lm.numberOfGlyphs
        while index < glyphs {
            var range = NSRange()
            lm.lineFragmentRect(forGlyphAt: index, effectiveRange: &range)
            index = NSMaxRange(range)
            lines += 1
        }
        return lines + (lm.extraLineFragmentTextContainer != nil ? 1 : 0)
    }

    // MARK: Other apps

    // An app that works on the text selected in the front app (Kerlig, a
    // grammar checker, the Services menu) reads it through the accessibility
    // API: the focused element, its selected text and its range. NSTextView
    // gives them from its own selection, kept equal to the reply's
    // (`ComposerTextView.updateNSView`). What such an app writes back (the
    // text corrected) would be refused, the view not being editable: it is
    // handed to the controller instead, which puts it in the reply as if
    // typed, so that it is saved and the cursor follows.

    /// Replaces the text selected (the cursor's place when none).
    var onReplaceSelection: ((String) -> Void)?
    /// Selects this range, in UTF-16 units.
    var onSelect: ((NSRange) -> Void)?

    override func setAccessibilitySelectedText(_ text: String?) {
        onReplaceSelection?(text ?? "")
    }

    override func setAccessibilitySelectedTextRange(_ range: NSRange) {
        onSelect?(range)
    }

    /// The whole text replaced: everything selected, then replaced.
    override func setAccessibilityValue(_ value: Any?) {
        guard let text = value as? String else { return }
        onSelect?(NSRange(location: 0, length: (string as NSString).length))
        onReplaceSelection?(text)
    }

    /// The setters above are allowed although the view is not editable:
    /// otherwise an app would see a read-only text and not offer to replace.
    override func isAccessibilitySelectorAllowed(_ selector: Selector) -> Bool {
        switch selector {
        case #selector(setAccessibilitySelectedText(_:)), #selector(setAccessibilitySelectedTextRange(_:)),
             #selector(setAccessibilityValue(_:)):
            return true
        default:
            return super.isAccessibilitySelectorAllowed(selector)
        }
    }

    /// The Services menu: the selection is offered as text, and the text a
    /// service returns replaces it.
    override func validRequestor(forSendType sendType: NSPasteboard.PasteboardType?,
                                 returnType: NSPasteboard.PasteboardType?) -> Any? {
        let ok = { (t: NSPasteboard.PasteboardType?) in t == nil || t == .string }
        if ok(sendType), ok(returnType), sendType != nil || returnType != nil { return self }
        return super.validRequestor(forSendType: sendType, returnType: returnType)
    }

    override func readSelection(from pboard: NSPasteboard, type: NSPasteboard.PasteboardType) -> Bool {
        guard let text = pboard.string(forType: .string) else { return false }
        onReplaceSelection?(text)
        return true
    }

    override func readSelection(from pboard: NSPasteboard) -> Bool {
        readSelection(from: pboard, type: .string)
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        onClick?(characterIndexForInsertion(at: point), event.modifierFlags.contains(.shift))
    }

    override func mouseUp(with event: NSEvent) {}
}


/// The AI panel of the reply: the prompt and the keys it answers to, then
/// the text as it comes, the differences proposed (removed struck out in
/// the error colour, added underlined in the accent colour), the subjects
/// to choose from, or why it failed. Its keys are read by the controller
/// (`AI.action`); a click does nothing, as the keys say everything.
/// An appointment in the AI panel (`dates`): its number, when, what, then
/// where and how the date was read, each on its own line.
struct AIEventCard: View {
    let index: Int
    let event: AIEvent
    let selected: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: scaled(10)) {
            Text("\(index + 1)").foregroundStyle(Palette.accentText)
            VStack(alignment: .leading, spacing: scaled(3)) {
                Text(AI.eventWhen(event)).foregroundStyle(Palette.accentText)
                Text(event.title).foregroundStyle(Palette.text)
                if !event.location.isEmpty {
                    Text(event.location).foregroundStyle(Palette.muted)
                }
                if !event.details.isEmpty {
                    Text(event.details).foregroundStyle(Palette.muted)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, scaled(6))
        .padding(.horizontal, scaled(8))
        .background(RoundedRectangle(cornerRadius: 4).fill(selected ? Palette.cursor : Color.clear))
    }
}

/// The sender's card in the AI panel (`contact`): the name in the accent
/// colour, then each piece on its own line (`AI.contactLines`), as a card
/// of the address book reads. Highlighted as a whole: it is what Return
/// saves.
struct AIContactCard: View {
    let contact: AIContact

    var body: some View {
        let lines = AI.contactLines(contact)
        VStack(alignment: .leading, spacing: scaled(3)) {
            ForEach(Array(lines.enumerated()), id: \.offset) { i, line in
                Text(line).foregroundStyle(i == 0 ? Palette.accentText : Palette.text)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, scaled(6))
        .padding(.horizontal, scaled(8))
        .background(RoundedRectangle(cornerRadius: 4).fill(Palette.cursor))
    }
}

struct AIPanel: View {
    let run: AIRun
    /// How tall the text may grow before it scrolls.
    var maxTextHeight: CGFloat = scaled(220)

    var body: some View {
        VStack(alignment: .leading, spacing: scaled(4)) {
            HStack(spacing: scaled(8)) {
                Text(run.title).foregroundStyle(Palette.accentText)
                if case .running = run.phase {
                    ProgressView().controlSize(.small)
                }
                Text(AI.hint(run.phase)).foregroundStyle(Palette.muted).lineLimit(1)
                Spacer(minLength: 0)
            }
            ScrollView {
                content.frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
            }
            // At most about a third of a window: the text below stays in
            // sight, and a long answer scrolls.
            .frame(maxHeight: maxTextHeight)
            .fixedSize(horizontal: false, vertical: true)
        }
        .font(.geek(12))
        .padding(.horizontal, 12)
        .padding(.vertical, scaled(6))
        .background(Palette.popup)
        .uiTarget("ai-panel")
    }

    @ViewBuilder private var content: some View {
        switch run.phase {
        case .running:
            Text(run.text.isEmpty ? L("The AI is reading…") : run.text).foregroundStyle(Palette.muted)
        case .failed(let message):
            Text(message).foregroundStyle(Palette.error)
        case .proposal(.replace(let segments, let unchanged)):
            if unchanged {
                Text(L("The AI changes nothing.")).foregroundStyle(Palette.muted)
            } else {
                VStack(alignment: .leading, spacing: scaled(6)) {
                    ForEach(Array(segments.enumerated()), id: \.offset) { _, s in
                        Text(Self.diff(s.diff))
                    }
                }
            }
        case .proposal(.subjects(let items)), .proposal(.choices(let items, _)):
            VStack(alignment: .leading, spacing: scaled(2)) {
                ForEach(Array(items.enumerated()), id: \.offset) { i, subject in
                    HStack(alignment: .firstTextBaseline, spacing: scaled(10)) {
                        Text("\(i + 1)").foregroundStyle(Palette.accentText)
                        Text(subject).foregroundStyle(Palette.text)
                        Spacer(minLength: 0)
                    }
                    // The text Return takes, highlighted as the row of a
                    // picker (`AIRun.choice`): the first until ↑ ↓ move it.
                    .background(i == run.choice ? Palette.cursor : Color.clear)
                }
            }
        case .proposal(.events(let items)) where items.isEmpty:
            Text(L("No date or appointment in this mail.")).foregroundStyle(Palette.muted)
        case .proposal(.events(let items)):
            // One block per appointment, with room between them: each is
            // read as an entry of a calendar (when, what, where, how the
            // date was read), not picked as a line of a menu. The one Return
            // saves is highlighted as a whole.
            VStack(alignment: .leading, spacing: scaled(8)) {
                ForEach(Array(items.enumerated()), id: \.offset) { i, e in
                    AIEventCard(index: i, event: e, selected: i == run.choice)
                }
            }
        case .proposal(.contact(let c)):
            VStack(alignment: .leading, spacing: scaled(4)) {
                AIContactCard(contact: c)
                if let line = AI.droppedLine(c) {
                    Text(line).foregroundStyle(Palette.muted)
                }
            }
        case .proposal(.text(let text)):
            Text(text).foregroundStyle(Palette.text)
        case .proposal(.targets(let items, _)):
            // A sorting shows on the rows of the list, not in a panel;
            // listed here only for completeness.
            Text(L("AI: %lld suggestions · ↩ on a row moves it there", items.count)).foregroundStyle(Palette.muted)
        case .proposal(.summary(let summary, let actions)):
            VStack(alignment: .leading, spacing: scaled(4)) {
                Text(summary).foregroundStyle(Palette.text)
                if !actions.isEmpty {
                    Text(L("To do")).foregroundStyle(Palette.accentText).padding(.top, scaled(4))
                    ForEach(Array(actions.enumerated()), id: \.offset) { _, a in
                        HStack(alignment: .firstTextBaseline, spacing: scaled(6)) {
                            Text("–").foregroundStyle(Palette.muted)
                            Text(a.task).foregroundStyle(Palette.text)
                            if !a.owner.isEmpty { Text(a.owner).foregroundStyle(Palette.muted) }
                            if !a.due.isEmpty { Text(a.due).foregroundStyle(Palette.unread) }
                        }
                    }
                }
            }
        case .proposal(.verdict(let risk, let reasons, let raised)):
            VStack(alignment: .leading, spacing: scaled(2)) {
                // The colour says the risk at a glance: high in the error
                // colour, medium in the unread one, low in the accent one.
                Text(AI.riskLabel(risk))
                    .foregroundStyle(risk == "high" ? Palette.error : risk == "medium" ? Palette.unread : Palette.accentText)
                if raised {
                    Text(L("Raised by NiceMailer: what it found itself in the mail")).foregroundStyle(Palette.muted)
                }
                ForEach(Array(reasons.enumerated()), id: \.offset) { _, r in
                    HStack(alignment: .firstTextBaseline, spacing: scaled(6)) {
                        Text("–").foregroundStyle(Palette.muted)
                        Text(r).foregroundStyle(Palette.text)
                    }
                }
            }
        }
    }

    /// The pieces of a diff as one text with its marks.
    static func diff(_ pieces: [AIDiffPiece]) -> AttributedString {
        var out = AttributedString()
        for p in pieces {
            var a = AttributedString(p.text)
            switch p.kind {
            case "removed":
                a.foregroundColor = Palette.error
                a.strikethroughStyle = .single
            case "added":
                a.foregroundColor = Palette.accentText
                a.underlineStyle = .single
            default:
                a.foregroundColor = Palette.text
            }
            out += a
        }
        return out
    }
}
