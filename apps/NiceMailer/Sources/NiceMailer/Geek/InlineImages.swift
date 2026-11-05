import Foundation
import WebKit

// Images embedded in a mail (`cid:`), shown by the HTML reader
// (`docs/thread-display.md`, "HTML of the sender").
//
// A web view cannot resolve `<img src="cid:logo@x">`: the image is a part
// of the message, not a file. The core gives such an image a source in the
// app's scheme, `nm-cid://image/<attachment id>` (`attachments.rs`,
// `resolve_inline_images`), and this file serves it from the part the core
// extracts (`get_attachment`). Nothing leaves the machine: these images are
// shown in the html and images displays alike, unlike remote images.
//
// - `InlineImages`: the rules, as pure functions (`InlineImagesTests`);
// - `InlineImageLoader`: the web view's handler of the `nm-cid:` scheme.

/// The rules of embedded images, as pure functions.
enum InlineImages {
    /// The app's scheme for embedded images (`attachments.rs`,
    /// `INLINE_SOURCE_PREFIX`).
    static let scheme = "nm-cid"
    static let sourcePrefix = "nm-cid://image/"
    /// Larger parts are not shown: a logo or a signature weighs a few kB,
    /// and the page must stay quick to draw.
    static let maxBytes = 10 * 1024 * 1024

    /// The attachment ids of the `nm-cid:` sources of `html`. Only the core
    /// writes such a source (a mail's own is removed by the sanitizer), so
    /// the ids found in the bodies are those the page may load.
    static func ids(in html: String) -> Set<Int> {
        var out = Set<Int>()
        var rest = html[...]
        let marker = " src=\"\(sourcePrefix)"
        while let start = rest.range(of: marker) {
            let digits = rest[start.upperBound...].prefix(while: \.isASCIIDigit)
            if let id = Int(digits) { out.insert(id) }
            rest = rest[start.upperBound...]
        }
        return out
    }

    /// The attachment id of an `nm-cid:` URL, or nil for anything else:
    /// digits only, as the core writes them.
    static func attachmentId(_ url: URL) -> Int? {
        let text = url.absoluteString
        guard text.hasPrefix(sourcePrefix) else { return nil }
        let digits = text.dropFirst(sourcePrefix.count)
        guard !digits.isEmpty, digits.count <= 18, digits.allSatisfy(\.isASCIIDigit) else { return nil }
        return Int(digits)
    }

    /// Whether a part may be shown as an image: an image type, and not too
    /// large. SVG is left out: a drawing that is a document, able to hold
    /// links and references to other files.
    static func showable(mimeType: String, size: Int) -> Bool {
        let type = mimeType.lowercased()
        return type.hasPrefix("image/") && !type.hasPrefix("image/svg") && size > 0 && size <= maxBytes
    }
}

/// The web view's handler of `nm-cid:` sources: it answers only the
/// attachments of the page it was given (`allowed`), with the bytes `load`
/// returns (the part extracted by the core, `AppModel.inlineImage`).
///
/// Swift note: a closure stored in a property, `load`, lets the tests give
/// the handler its parts without a core.
@MainActor
final class InlineImageLoader: NSObject, WKURLSchemeHandler {
    /// Attachment ids the current page may load (`InlineImages.ids`).
    var allowed: Set<Int> = []
    /// The bytes and MIME type of an attachment, nil if it cannot be shown.
    var load: (Int) async -> (data: Data, mimeType: String)? = { _ in nil }
    /// Tasks WebKit has not stopped: answering a stopped task raises an
    /// exception.
    private var active: Set<ObjectIdentifier> = []

    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        guard let url = task.request.url, let id = InlineImages.attachmentId(url), allowed.contains(id) else {
            task.didFailWithError(URLError(.resourceUnavailable))
            return
        }
        let key = ObjectIdentifier(task)
        active.insert(key)
        Task { @MainActor in
            let image = await load(id)
            guard active.remove(key) != nil else { return }
            guard let image else {
                task.didFailWithError(URLError(.resourceUnavailable))
                return
            }
            task.didReceive(URLResponse(url: url, mimeType: image.mimeType,
                                        expectedContentLength: image.data.count, textEncodingName: nil))
            task.didReceive(image.data)
            task.didFinish()
            if UIScript.isActive { UIScript.inlineImagesShown += 1 }
        }
    }

    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {
        active.remove(ObjectIdentifier(task))
    }
}

private extension Character {
    /// `0` to `9` only: `isNumber` would also accept "٣" or "½".
    var isASCIIDigit: Bool { isASCII && isNumber }
}
