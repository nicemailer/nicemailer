import Foundation

/// Deep links to a mail: `nicemailer://m/<key>` (`docs/deep-links.md`).
///
/// Each message has its own URL: the scheme, `m` for "message", and the
/// message's **link key**, 12 hexadecimal characters that depend only on
/// the mail (its `Message-ID`; `modele de donnees.md`, `link_key`). The
/// same mail has the same link on every Mac, whatever number the local
/// database gives it. The typical use is a note app: in Obsidian,
/// `[Devis toiture](nicemailer://m/3f9a0c2b7e41)` opens the mail in
/// NiceMailer. macOS gives the URL to the app because its bundle declares
/// the scheme (`make bundle`, `CFBundleURLTypes`).
///
/// The core writes the link (`MessageView.link`); the app only reads it
/// back. Older links carry the message's id in the database instead
/// (`nicemailer://m/48213`), and a mail without `Message-ID` whose body is
/// not downloaded yet still gets one: they keep working on the database
/// that made them.
///
/// Pure functions, tested on their own (`DeepLinkTests`).
enum DeepLink {
    static let scheme = "nicemailer"
    /// The "host" of a message link: `nicemailer://m/3f9a0c2b7e41`.
    static let messageHost = "m"
    /// Characters of a key (48 bits), as `nm_api::deep_link::KEY_LEN`.
    static let keyLength = 12
    /// Longest id an old link may carry (`nm_api::deep_link::MAX_ID_LEN`):
    /// a link of 12 characters is always a key, even of digits only.
    static let maxIdLength = 11

    /// What a link names. Swift note: an `enum` with associated values is
    /// a tagged union: a value is one case, with its own payload, and a
    /// `switch` must handle every case.
    enum Target: Equatable {
        /// The key of a mail, lowercase: found on any Mac that has it.
        case key(String)
        /// A message id of this database (an old link).
        case id(Int)

        /// The params of `locate_message` that find it.
        var params: [String: Any] {
            switch self {
            case .key(let k): ["link_key": k]
            case .id(let id): ["message_id": id]
            }
        }
    }

    /// The link of a message by its id: what an old core gave, and the
    /// fallback when the core gives no link.
    static func url(messageId id: Int) -> URL {
        // Swift note: `URL(string:)` returns an optional; a scheme, a host
        // and digits always make a valid URL, so `!` cannot fail here.
        URL(string: "\(scheme)://\(messageHost)/\(id)")!
    }

    /// The link of a message: the one the core wrote (by its key), or else
    /// its id. A link from the core that this app would not read back is
    /// not used: a copied link must always open.
    static func url(of message: MessageView) -> URL {
        if let text = message.link, let url = URL(string: text), target(from: url) != nil { return url }
        return url(messageId: message.id)
    }

    /// What a link names, or nil when the URL is not a message link of this
    /// app. Lenient on what a note app may do to it: the scheme, host and
    /// key in any case, a slash at the end. Strict on the rest: exactly 12
    /// hexadecimal characters for a key, 1 to 11 digits for an id, so a
    /// hand-edited link cannot ask anything else.
    static func target(from url: URL) -> Target? {
        guard url.scheme?.lowercased() == scheme, url.host()?.lowercased() == messageHost else { return nil }
        let path = url.path().trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if path.count == keyLength, path.allSatisfy(\.isASCIIHexDigit) {
            return .key(path.lowercased())
        }
        guard !path.isEmpty, path.count <= maxIdLength, path.allSatisfy(\.isASCIIDigit), let id = Int(path), id > 0 else {
            return nil
        }
        return .id(id)
    }

    /// A Markdown link to the message, for a note: `[subject](url)`. The
    /// brackets and backslashes of the subject are escaped, so a subject
    /// such as "[Spam] Offre" does not break the link; an empty subject
    /// gives "(no subject)".
    static func markdown(subject: String, url: URL) -> String {
        let trimmed = subject.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = (trimmed.isEmpty ? L("(no subject)") : trimmed)
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "[", with: "\\[")
            .replacingOccurrences(of: "]", with: "\\]")
            // A link text is one line.
            .replacingOccurrences(of: "\n", with: " ")
        return "[\(text)](\(url.absoluteString))"
    }
}

private extension Character {
    /// `0` to `9` only: `isNumber` would also accept "٣" or "½".
    var isASCIIDigit: Bool { isASCII && isNumber }
    /// `0` to `9`, `a` to `f`, `A` to `F` only.
    var isASCIIHexDigit: Bool { isASCII && isHexDigit }
}
