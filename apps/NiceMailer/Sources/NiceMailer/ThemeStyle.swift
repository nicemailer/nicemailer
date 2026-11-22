import AppKit
import Observation
import SwiftUI

// The theme in effect (`specifications/themes.md`, `docs/themes.md`): what
// the core resolved from `ui.theme` and the files of `themes/`, received
// with `theme_get` and read again at each `ThemeChanged`. The app never
// reads a theme file nor walks the inheritance itself: every key of the
// catalog comes with its value.

/// Result of `theme_get` (`nm_api::ThemeView`).
///
/// Swift note: `Decodable` with the client's `.convertFromSnakeCase` turns
/// `row_height` into `rowHeight`; the embedded file is read the same way
/// (`ThemeStore.builtins`), so both follow one shape.
struct ThemeView: Decodable, Equatable {
    let name: String
    /// From the theme to the built-in one it rests on: `["sepia", "clean"]`.
    let chain: [String]
    /// The folder of the themes, next to `config.toml`.
    let path: String
    let window: Window
    let fonts: Fonts
    let layout: Layout
    let motion: Motion
    let colors: ThemeColors
    /// Why a file of the chain is refused: the theme is then made of its
    /// last valid version.
    let error: KeymapProblem?
    /// Unknown keys, a contrast below WCAG AA, a `ui.theme` naming no theme.
    let warnings: [KeymapProblem]

    /// `[window]` (`themes.md`, section 6.1).
    struct Window: Decodable, Equatable {
        /// `status_bar` or `toolbar`.
        let chrome: String
        /// `top`, `bottom` or `hidden`.
        let statusBarPosition: String
        /// `shown`, `hidden` or `merged`.
        let titleBar: String
        let keyHints: Bool
        /// The blocks of actions say their name beside their icon.
        let blockLabels: Bool
        /// Height of the status bar, in points for a 12 pt text of the bar.
        let statusBarHeight: Double
        /// `plain` or `pill`.
        let blockStyle: String
        /// A line between the status bar and the mails.
        let barLine: Bool
        /// The blocks shown, in order, with the separators `|` and `->`.
        let blocks: [String]
        let defaultColumns: Int
    }

    /// `[fonts]` (section 6.2).
    struct Fonts: Decodable, Equatable {
        let interface: String
        let reading: String
        let readingScale: Double
        /// Points added between two lines of the body, whatever the text
        /// size.
        let readingLineSpacing: Double
        let titleScale: Double
    }

    /// `[layout]` (section 6.3): lengths in points for a 12 pt text.
    struct Layout: Decodable, Equatable {
        let rowHeight: Double
        let rowRadius: Double
        let rowInset: Double
        let zebra: Bool
        /// 0: the height of the text of the row.
        let menuRowHeight: Double
        let popupRadius: Double
        let popupShadow: Bool
        let menuKeys: String
        let messageCards: Bool
        let avatars: Bool
        let readerMaxWidth: Double
        let scrollers: String
        let separators: String
    }

    /// `[motion]` (section 6.4), in milliseconds.
    struct Motion: Decodable, Equatable {
        let sidebarMs: Int
        let popupMs: Int
    }
}

/// `[colors]` (section 6.5): every color of the catalog.
struct ThemeColors: Decodable, Equatable {
    let background, sidebar, list, bar, text, muted, accent, accentText: ThemeColor
    let selection, selectionText, marked, markedBar, separator, popup: ThemeColor
    let unread, flag, error, virtualFolder: ThemeColor
}

/// A color of a theme: a pair `#RRGGBB` / `#RRGGBBAA` for the light and the
/// dark appearance, or a color of macOS by its name (section 5.1).
///
/// Swift note: an `enum` with associated values; its `init(from:)` tells
/// the two JSON forms apart by their keys, `{"light", "dark"}` or
/// `{"system"}`.
enum ThemeColor: Decodable, Equatable {
    case pair(light: String, dark: String)
    case system(String)

    private enum Keys: String, CodingKey { case light, dark, system }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        if let name = try c.decodeIfPresent(String.self, forKey: .system) {
            self = .system(name)
        } else {
            self = .pair(light: try c.decode(String.self, forKey: .light), dark: try c.decode(String.self, forKey: .dark))
        }
    }

    /// The color for `appearance`. A color of the system is resolved in that
    /// appearance, so that it can be given a web page as a hexadecimal
    /// value too (`hex`).
    func resolved(for appearance: NSAppearance) -> NSColor {
        let dark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        switch self {
        case .pair(let light, let darkHex):
            return NSColor(hex: dark ? darkHex : light) ?? .magenta
        case .system(let name):
            let color = ThemeColor.systemColor(name)
            var out = color
            // Swift note: a dynamic system color has no components until it
            // is resolved in an appearance; `performAsCurrentDrawingAppearance`
            // makes `appearance` the current one for the closure.
            appearance.performAsCurrentDrawingAppearance {
                out = color.usingColorSpace(.sRGB) ?? color
            }
            return out
        }
    }

    /// `#rrggbb` (or `#rrggbbaa`) for one appearance: what the editor and
    /// Dark Reader receive. Lowercase, as the palette always wrote them.
    func hex(dark: Bool) -> String {
        switch self {
        case .pair(let light, let d):
            return (dark ? d : light).lowercased()
        case .system:
            let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
            let c = resolved(for: appearance).usingColorSpace(.sRGB) ?? .black
            let v = { (x: CGFloat) in Int((x * 255).rounded()) }
            return c.alphaComponent < 1
                ? String(format: "#%02x%02x%02x%02x", v(c.redComponent), v(c.greenComponent), v(c.blueComponent), v(c.alphaComponent))
                : String(format: "#%02x%02x%02x", v(c.redComponent), v(c.greenComponent), v(c.blueComponent))
        }
    }

    /// The AppKit color of a name of `catalog::SYSTEM_COLORS` (the core
    /// refuses any other). Each follows the appearance, the increased
    /// contrast and the accent color of System Settings by itself.
    static func systemColor(_ name: String) -> NSColor {
        switch name {
        case "accent": .controlAccentColor
        case "label": .labelColor
        case "secondary_label": .secondaryLabelColor
        case "tertiary_label": .tertiaryLabelColor
        case "separator": .separatorColor
        case "window_background": .windowBackgroundColor
        case "control_background": .controlBackgroundColor
        case "under_page_background": .underPageBackgroundColor
        case "text_background": .textBackgroundColor
        case "selected_content_background": .selectedContentBackgroundColor
        case "unemphasized_selected_content_background": .unemphasizedSelectedContentBackgroundColor
        case "alternating_content_background": NSColor.alternatingContentBackgroundColors.last ?? .controlBackgroundColor
        case "find_highlight": .findHighlightColor
        case "red": .systemRed
        case "orange": .systemOrange
        case "yellow": .systemYellow
        case "green": .systemGreen
        case "blue": .systemBlue
        case "purple": .systemPurple
        case "gray": .systemGray
        default: .magenta
        }
    }
}

extension NSColor {
    /// A color from `#RRGGBB` or `#RRGGBBAA`; nil for anything else.
    convenience init?(hex: String) {
        guard hex.hasPrefix("#") else { return nil }
        let digits = hex.dropFirst()
        guard digits.count == 6 || digits.count == 8, let v = UInt64(digits, radix: 16) else { return nil }
        let rgb = digits.count == 8 ? v >> 8 : v
        let alpha = digits.count == 8 ? CGFloat(v & 0xFF) / 255 : 1
        self.init(
            srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255,
            green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255,
            alpha: alpha
        )
    }
}

/// The theme in effect, shared by the whole app (`ui.md`, « Fenêtres »: the
/// theme is common to every window).
///
/// Two readers: SwiftUI views, which observe `current` (Swift note:
/// `@Observable` records which properties a `body` read, and redraws it when
/// one changes); and the dynamic colors of `Palette`, which AppKit asks for
/// at drawing time from any thread. Those read `ThemeStore.colors`, a copy
/// behind a lock, set together with `current`.
@MainActor
@Observable
final class ThemeStore {
    /// The store of the app, the one the colors of `Palette` follow. A unit
    /// test makes its own (`ThemeStore()`), which leaves them alone: tests
    /// run in parallel.
    static let shared = ThemeStore(drivesPalette: true)

    /// The theme the views draw. Geek until the core answers: the theme the
    /// app opens on by default.
    private(set) var current: ThemeView

    /// Changes at each new theme: the window is rebuilt with it
    /// (`WindowContent`, `.id`), as for a new text size. A color already
    /// resolved by SwiftUI would otherwise keep its old value.
    private(set) var revision = 0

    /// Whether this store sets the colors of `Palette`: only the app's.
    @ObservationIgnored private let drivesPalette: Bool

    init(_ theme: ThemeView = ThemeStore.builtins["geek"]!, drivesPalette: Bool = false) {
        current = theme
        self.drivesPalette = drivesPalette
        if drivesPalette { ThemeStore.active = theme }
    }

    /// Applies a theme received from the core; nothing when it is the same.
    func apply(_ theme: ThemeView) {
        guard theme != current else { return }
        current = theme
        if drivesPalette { ThemeStore.active = theme }
        revision += 1
    }

    /// The theme of the app's store, for what is drawn outside SwiftUI's
    /// observation: the dynamic colors of `Palette`, which AppKit asks for
    /// from any thread, and the fonts (`Fonts.interface`).
    ///
    /// Swift note: `nonisolated(unsafe)` lets code outside the main actor
    /// read the property; the lock makes that safe.
    nonisolated static var active: ThemeView {
        get { lock.withLock { stored } }
        set { lock.withLock { stored = newValue } }
    }
    /// The colors of `active`.
    nonisolated static var colors: ThemeColors { active.colors }
    private nonisolated(unsafe) static var stored: ThemeView = builtins["geek"]!
    private nonisolated static let lock = NSLock()

    /// The three built-in themes resolved, embedded in the app
    /// (`Resources/themes-builtin.json`, written by the core: `nicemailer
    /// theme show --builtins --json`). In effect until the core answers, and
    /// in the tests.
    nonisolated static let builtins: [String: ThemeView] = {
        guard let url = AppResources.bundle.url(forResource: "themes-builtin", withExtension: "json"),
              let data = try? Data(contentsOf: url)
        else { fatalError("themes-builtin.json is missing from the app's resources") }
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        do {
            return try d.decode([String: ThemeView].self, from: data)
        } catch {
            fatalError("themes-builtin.json cannot be read: \(error)")
        }
    }()
}
