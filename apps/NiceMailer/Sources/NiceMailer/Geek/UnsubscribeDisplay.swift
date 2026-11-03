import Foundation

/// The unsubscribe button of the reader's title line (`ui.md`,
/// "Désabonnement"), as pure functions so that they are tested without a
/// view (`Tests/NiceMailerTests/UnsubscribeTests.swift`).
enum UnsubscribeDisplay {
    /// What the button shows: its text, an SF Symbol saying what a click
    /// does, and a tooltip.
    struct Label: Equatable {
        let text: String
        let symbol: String?
        let help: String
    }

    /// Folders where no unsubscribe is offered: unsubscribing from a spam
    /// confirms to the spammer that the address is read.
    static let hiddenIn: Set<String> = ["junk", "quarantine", "banned"]

    /// The unsubscribe offered for a message, or `nil`: none found, or a
    /// spam folder.
    static func offer(_ m: MessageView?) -> UnsubscribeView? {
        guard let m, let u = m.unsubscribe else { return nil }
        if let role = m.folderRole, hiddenIn.contains(role) { return nil }
        return u
    }

    /// The button's look. The source shows: a link of the List-Unsubscribe
    /// header (clean: done by the core in one click, or a page / address
    /// the list gives for that) or a link found in the mail's text (a page
    /// of the sender, opened in the browser).
    static func label(_ u: UnsubscribeView) -> Label {
        if u.source == "body" {
            return Label(text: L("unsubscribe"), symbol: "link",
                         help: L("Opens the unsubscribe link found in the text of the mail, in the browser (⌘U)"))
        }
        switch u.method {
        case "one_click":
            return Label(text: L("unsubscribe"), symbol: nil,
                         help: L("Unsubscribes at once, without opening anything (⌘U)"))
        case "mail":
            return Label(text: L("unsubscribe"), symbol: "envelope",
                         help: L("Opens the unsubscribe mail of the list in Mail (⌘U)"))
        default:
            return Label(text: L("unsubscribe"), symbol: "arrow.up.right.square",
                         help: L("Opens the unsubscribe page of the list in the browser (⌘U)"))
        }
    }

    /// "unsubscribed on 3 Nov", shown instead of the button.
    static func done(_ at: Int) -> String {
        L("unsubscribed %@", Format.geekListDate(at))
    }
}
