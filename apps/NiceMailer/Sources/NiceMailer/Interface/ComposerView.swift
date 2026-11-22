import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The reply being written, in place of the reader (`reponse.md`,
/// section 6): who it goes to, its subject, where the conversation goes
/// once it is sent, then the text.
///
/// The headers are the app's own fields: their keys are read and applied
/// by `InterfaceController.handleComposeKey`, and this view only draws them.
/// The text is the editor (`EditorWebView.swift`, `edition.md`): a
/// standard text component, which takes every key the app does not.
struct ComposerView: View {
    @Environment(AppModel.self) private var model
    let controller: InterfaceController

    var body: some View {
        if let c = model.composer {
            VStack(alignment: .leading, spacing: 0) {
                ReaderTitleBar(subject: c.headers.subject) {
                    // The keys of `[compose]` in effect (`keymap.toml`).
                    let k = { (a: KeyAction) in model.keymap.mention(a, in: Keymap.composeChain) }
                    Text(L("%@ send · %@ send then… · %@ from · %@ format · Esc keep · %@ abandon",
                           k(.composeSend), k(.composeFileAndSend), k(.composeFrom), k(.composeTogglePlain),
                           k(.composeAbandon)))
                        .foregroundStyle(Palette.muted).lineLimit(1)
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
                    } else if let after = Compose.afterSendLine(c.draft, names: model.folderNames) {
                        Text(after).foregroundStyle(Palette.accentText).lineLimit(1)
                    }
                }
                .font(.interface(12))
                .padding(.horizontal, 12)
                .padding(.vertical, scaled(6))
                Palette.separator.frame(height: 1)
                EditorHost(focused: c.focus == .body, controller: controller)
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
        HStack(alignment: .firstTextBaseline, spacing: scaled(6)) {
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
            // Beside the line rather than in it: a click on the button does
            // not also give the keys to Cc.
            if field == .cc { bccToggle(c) }
        }
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

    /// The button that shows or hides the Bcc line (`InterfaceController.toggleBcc`):
    /// highlighted while the line is shown, dimmed while it holds addresses
    /// and so cannot be hidden.
    private func bccToggle(_ c: Composer) -> some View {
        Text(L("Bcc"))
            .lineLimit(1)
            .fixedSize()
            .foregroundStyle(c.showsBcc ? Palette.accentText : Palette.muted)
            .padding(.horizontal, scaled(4))
            .background(RoundedRectangle(cornerRadius: 3).fill(c.showsBcc ? Palette.cursor : Color.clear))
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(Palette.separator))
            .opacity(c.showsBcc && !c.canHideBcc ? 0.5 : 1)
            .contentShape(Rectangle())
            .onTapGesture { controller.toggleBcc() }
            .accessibilityAddTraits(c.showsBcc ? [.isButton, .isSelected] : .isButton)
            .uiTarget("compose-bcc-toggle")
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

/// The editor's web view in the reply (`EditorBridge.shared`, created at
/// launch and kept: this view only shows it, and gives it the keys when the
/// text has the focus).
struct EditorHost: NSViewRepresentable {
    /// The text has the keys (`Composer.focus`).
    var focused: Bool
    let controller: InterfaceController

    /// Holds the web view; once in a window, gives the text the keys if it
    /// has the focus: before, neither AppKit nor the page can take them.
    final class Container: NSView {
        var onWindow: (() -> Void)?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window != nil { onWindow?() }
        }
        /// Light or dark: the editor's colours follow.
        override func viewDidChangeEffectiveAppearance() {
            super.viewDidChangeEffectiveAppearance()
            (subviews.first as? EditorWebView).map { _ in EditorBridge.shared.themeChanged() }
        }
    }

    func makeNSView(context: Context) -> Container {
        // A container: SwiftUI may make and drop several of these while the
        // reply is open; the one web view moves into the newest.
        Container()
    }

    func updateNSView(_ container: Container, context: Context) {
        let web = controller.editorView
        let model = controller.model
        container.onWindow = { [weak controller] in
            // A new window content (the text size rebuilds it): the theme
            // again, then the keys if the text has them.
            (model.editor as? EditorBridge)?.themeChanged()
            if controller?.model.composer?.focus == .body { model.editor?.focus() }
        }
        if web.superview !== container {
            web.removeFromSuperview()
            web.frame = container.bounds
            web.autoresizingMask = [.width, .height]
            container.addSubview(web)
        }
        if container.window != nil { (model.editor as? EditorBridge)?.themeChanged() }
        if focused, let w = container.window, w.firstResponder !== web { model.editor?.focus() }
    }

    static func dismantleNSView(_ container: Container, coordinator: ()) {
        // The web view lives on, out of any window, until the next reply.
        for v in container.subviews { v.removeFromSuperview() }
    }
}

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
    // The keys the panel's hints cite (`keymap.toml`).
    @Environment(AppModel.self) private var model
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
                Text(AI.hint(run.phase, keymap: model.keymap)).foregroundStyle(Palette.muted).lineLimit(1)
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
        .font(.interface(12))
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
            Text(L("AI: %lld suggestions · %@ on a row moves it there", items.count, model.keymap.mention(.mailOpen, in: Keymap.mailChain)))
                .foregroundStyle(Palette.muted)
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
