import Foundation

/// Deep links to a mail: `nicemailer://m/<id>` (`docs/deep-links.md`).
///
/// Each message has its own URL, as short as possible: the scheme, `m` for
/// "message", and the message's id in the database. The typical use is a
/// note app: in Obsidian, `[Devis toiture](nicemailer://m/48213)` opens the
/// mail in NiceMailer. macOS gives the URL to the app because its bundle
/// declares the scheme (`make bundle`, `CFBundleURLTypes`).
///
/// The id is the core's `message.id`: short and exact, but tied to this
/// database. A link outlives a deleted mail or a rebuilt database only as a
/// "not found" (the app says so).
///
/// Pure functions, tested on their own (`DeepLinkTests`).
enum DeepLink {
    static let scheme = "nicemailer"
    /// The "host" of a message link: `nicemailer://m/48213`.
    static let messageHost = "m"

    /// The link of the message `id`.
    static func url(messageId id: Int) -> URL {
        // Swift note: `URL(string:)` returns an optional; a scheme, a host
        // and digits always make a valid URL, so `!` cannot fail here.
        URL(string: "\(scheme)://\(messageHost)/\(id)")!
    }

    /// The message id of a link, or nil when the URL is not a message link
    /// of this app. Lenient on what a note app may do to it: the scheme and
    /// host in any case, a slash at the end. Strict on the id: digits only,
    /// positive, so a hand-edited link cannot ask anything else.
    static func messageId(from url: URL) -> Int? {
        guard url.scheme?.lowercased() == scheme, url.host()?.lowercased() == messageHost else { return nil }
        let path = url.path().trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard !path.isEmpty, path.count <= 18, path.allSatisfy(\.isASCIIDigit), let id = Int(path), id > 0 else { return nil }
        return id
    }

    /// A Markdown link to the message, for a note: `[subject](url)`. The
    /// brackets and backslashes of the subject are escaped, so a subject
    /// such as "[Spam] Offre" does not break the link; an empty subject
    /// gives "(no subject)".
    static func markdown(subject: String, messageId id: Int) -> String {
        let trimmed = subject.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = (trimmed.isEmpty ? L("(no subject)") : trimmed)
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "[", with: "\\[")
            .replacingOccurrences(of: "]", with: "\\]")
            // A link text is one line.
            .replacingOccurrences(of: "\n", with: " ")
        return "[\(text)](\(url(messageId: id).absoluteString))"
    }
}

private extension Character {
    /// `0` to `9` only: `isNumber` would also accept "٣" or "½".
    var isASCIIDigit: Bool { isASCII && isNumber }
}
