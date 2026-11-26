import Foundation

/// One control of the reader's title line. Which ones are shown, and in
/// which order, is the theme's (`reader.title_actions`,
/// `specifications/themes.md`, section 6.6): first what the line says,
/// then what can be done. A control without an object for the mail shown
/// is hidden, but its key still works.
enum ReaderTitleAction: String, Identifiable, CaseIterable {
    /// The folder of the mail, in "Everything" (`reader-folder`).
    case folderName = "folder_name"
    /// `T`: quotes and signatures are shown too.
    case showAll = "show_all"
    /// The tracking pixels the core removed (`reader-pixels`).
    case pixels
    /// "unsubscribed on …", once it is done.
    case unsubscribed
    /// Answer everyone (`r`, `compose.reply_all`); hidden in Quarantine.
    case replyAll = "reply_all"
    /// Answer the sender only (`R`, `compose.reply_sender`); hidden in
    /// Quarantine.
    case replySender = "reply_sender"
    /// Forward (`F`, `compose.forward`); hidden in Quarantine.
    case forward
    /// Unsubscribe (`⌘U`), when the mail gives a way to.
    case unsubscribe
    /// The flag of the mail shown (`*`); also shown in Quarantine.
    case flag
    /// The `text | html | images` switch (`vh`, `vi`).
    case textOrHtml = "text_or_html"

    var id: String { rawValue }

    /// What the mail shown offers: each control appears only when this has
    /// an object for it.
    struct Context: Equatable {
        /// The folder of the mail, in "Everything"; nil in a folder.
        var folderLabel: String?
        /// `T` is on.
        var showAll = false
        /// Tracking pixels the core removed from the conversation.
        var trackingPixels = 0
        /// The sender was unsubscribed at this time (a timestamp).
        var unsubscribedAt: Int?
        /// The mail gives a way to unsubscribe, not done yet.
        var unsubscribeOffered = false
        /// The flag of the mail shown; nil when no mail is open.
        var flagged: Bool?
        /// The mail has an HTML part.
        var hasHtml = false
        /// In Quarantine, answering and forwarding are refused
        /// (`raccourcis.md`), so their buttons are hidden.
        var quarantine = false
    }

    /// The controls to show, in the theme's order: `configured` filtered by
    /// what `c` offers. The core refuses an unknown name, so none can
    /// arrive; one is dropped here all the same rather than break the line.
    static func shown(configured: [String], _ c: Context) -> [ReaderTitleAction] {
        configured.compactMap { name in
            guard let action = ReaderTitleAction(rawValue: name) else { return nil }
            switch action {
            case .folderName: return c.folderLabel != nil ? action : nil
            case .showAll: return c.showAll ? action : nil
            case .pixels: return c.trackingPixels > 0 ? action : nil
            case .unsubscribed: return c.unsubscribedAt != nil ? action : nil
            case .unsubscribe: return c.unsubscribeOffered ? action : nil
            case .replyAll, .replySender, .forward: return c.quarantine ? nil : action
            case .flag: return c.flagged != nil ? action : nil
            case .textOrHtml: return c.hasHtml ? action : nil
            }
        }
    }
}
