import Foundation
import OSLog
import WebKit

/// A mail made for a white page, shown in the dark theme
/// (`docs/thread-display.md`, "HTML of the sender").
///
/// A mail that brings its own dark mode is shown with it (`MailFrame`).
/// Most do not: their CSS writes dark text on white. Showing them on a white
/// sheet breaks the dark theme; dropping their colours loses their buttons
/// and headings. Dark Reader (`Resources/DarkReader`, MIT License), the
/// browser extension's engine, rewrites each colour of the page for a dark
/// background, keeping its hue: a white background becomes the theme's,
/// dark text becomes light, an orange button stays orange.
///
/// How it runs, and why it is safe:
///
/// - The page's own JavaScript is off (`allowsContentJavaScript`), but the
///   app may still run scripts in a **content world** of its own
///   (`WKContentWorld`): Dark Reader runs there, isolated from the page,
///   which has no script anyway.
/// - **No network**: Dark Reader asks to download the images it analyses
///   and the style sheets it cannot read through the function it is given
///   (`setFetchMethod`); ours refuses every request. Its attempt to add a
///   script to the page itself is refused by the page's Content Security
///   Policy (`ContentPolicy`): nothing of it runs beside the mail.
/// - It follows the app's appearance (`DarkReader.auto`): active in the dark
///   theme, inactive in the light one, where a mail keeps its colours.
/// - `immediateModify`: by default Dark Reader waits until the page is
///   visible before rewriting its colours, and only puts a rough fallback
///   sheet meanwhile (it removed the background of buttons). The reader
///   wants the final colours at once, without a white flash, and the UI
///   scenarios run off screen.
/// - `requestAnimationFrame` is replaced, in its world only, by a 16 ms
///   timer: WebKit stops animation frames in a window that is hidden or
///   off screen, and Dark Reader schedules part of its work on them.
///
/// It is only added to a page holding a mail without a dark mode of its own
/// (`needed`); its source (360 kB) is read once.
enum DarkMode {
    /// The world Dark Reader runs in.
    static let world = WKContentWorld.world(name: "NiceMailer-DarkMode")

    private static let log = Logger(subsystem: "NiceMailer", category: "DarkMode")

    /// Dark Reader's source, from the app's resources; nil if missing (the
    /// mails are then shown on a white sheet, `MailFrame`).
    static let library: String? = {
        guard let url = AppResources.bundle.url(forResource: "darkreader", withExtension: "js", subdirectory: "DarkReader"),
              let source = try? String(contentsOf: url, encoding: .utf8) else {
            log.error("Dark Reader is missing from the app's resources")
            return nil
        }
        return source
    }()

    /// Whether Dark Reader can run (its source was found).
    static var available: Bool { library != nil }

    /// Whether a page needs Dark Reader: it shows an HTML message without a
    /// dark mode of its own.
    static func needed(_ bodies: [String]) -> Bool {
        bodies.contains { MailFrame.colours($0) == .paper }
    }

    /// The theme given to Dark Reader: the theme's dark colours, the mail's
    /// contrast as is.
    static var theme: String {
        let hex = { (rgb: UInt32) in String(format: "#%06x", rgb) }
        return """
        {immediateModify: true, brightness: 100, contrast: 100, sepia: 0, \
        darkSchemeBackgroundColor: '\(hex(Palette.darkBackground))', darkSchemeTextColor: '\(hex(Palette.darkText))'}
        """
    }

    /// Lines of the app's own page that keep the theme's colours: Dark
    /// Reader would give them a background of its own.
    static let fixes = """
    {css: 'summary, .att a, pre, .m, .q { background-color: transparent !important; }'}
    """

    /// The script that sets Dark Reader up, after its source.
    static var setup: String {
        """
        window.requestAnimationFrame = callback => setTimeout(() => callback(performance.now()), 16);
        window.cancelAnimationFrame = id => clearTimeout(id);
        window.__nmDarkFetches = 0;
        DarkReader.setFetchMethod(url => {
            window.__nmDarkFetches++;
            return Promise.reject(new Error('NiceMailer: no network for Dark Reader'));
        });
        DarkReader.auto(\(theme), \(fixes));
        """
    }

    /// The user script to add to a page that needs it; nil without Dark
    /// Reader. At the start of the document: the fallback colours come
    /// before the first paint, and Dark Reader follows the frames as the
    /// parser adds them.
    static func userScript() -> WKUserScript? {
        guard let library else { return nil }
        return WKUserScript(source: library + "\n" + setup, injectionTime: .atDocumentStart,
                            forMainFrameOnly: true, in: world)
    }
}
