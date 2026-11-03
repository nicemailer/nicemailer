import AppKit
import CoreText
import SwiftUI

// Shared pieces of the three themes (`ui.md`): the theme choice, the
// localization helper, the bundled font and the geek palette.

/// The three interface themes of `ui.md`. Only `classic` (the first
/// version of the interface) and `geek` exist so far; `design` falls back to
/// `classic`.
///
/// Swift note: `String` raw values let the enum be stored in UserDefaults
/// (`rawValue`) and read back (`init(rawValue:)`); `CaseIterable` provides
/// `allCases`, used to build the Theme menu.
enum UITheme: String, CaseIterable {
    case classic, design, geek

    /// Name shown in the Theme menu.
    var title: String {
        switch self {
        case .classic: L("Classic")
        case .design: L("Design (not yet: same as Classic)")
        case .geek: L("Geek")
        }
    }

    /// UserDefaults key. During development the theme is a menu choice; it
    /// will become a configuration option (`ui.md`).
    static let defaultsKey = "theme"
}

/// Where the app keeps its preferences (theme, columns, status bar…).
///
/// In UI script mode (`UIScript`), a separate, emptied domain is used, so
/// automated tests always start from the defaults and never change the
/// user's own settings. Unit tests pass their own `UserDefaults` too.
enum AppDefaults {
    static let store: UserDefaults = {
        guard UIScript.isActive, let d = UserDefaults(suiteName: "NiceMailer.uitest") else { return .standard }
        d.removePersistentDomain(forName: "NiceMailer.uitest")
        return d
    }()
}

/// Localized string from the app's String Catalog.
///
/// Swift note: in a Swift Package, resources live in `Bundle.module`, not in
/// `Bundle.main`, so SwiftUI's default lookup would not find the
/// translations. Every user-facing string goes through this function (see
/// CLAUDE.md, "Languages"). The key is the English text.
func L(_ key: String) -> String {
    localizedBundle.localizedString(forKey: key, value: key, table: nil)
}

/// The `.lproj` folder of the language to use, chosen explicitly.
///
/// Why not let Foundation choose: launched by `swift run`, the app is a bare
/// executable whose main bundle declares no language, and Foundation then
/// restricts every bundle to the development language (English). So the
/// best match between the user's languages (system, or the per-app choice
/// of System Settings) and the bundled translations is computed here, once.
/// A missing translation falls back to the key, which is the English text.
private let localizedBundle: Bundle = {
    let available = Bundle.module.localizations
    let wanted = Bundle.preferredLocalizations(from: available, forPreferences: Locale.preferredLanguages)
    if let lang = wanted.first,
       let path = Bundle.module.path(forResource: lang, ofType: "lproj"),
       let bundle = Bundle(path: path) {
        return bundle
    }
    return Bundle.module
}()

/// Same, with values inserted: `L("%lld marked", n)`. The catalog stores the
/// format with its placeholders (`%@` for text, `%lld` for integers).
func L(_ key: String, _ args: CVarArg...) -> String {
    String(format: L(key), arguments: args)
}

/// The fonts shipped with the app.
enum Fonts {
    /// iA Writer Mono V (SIL Open Font License, `Resources/Fonts/LICENSE.md`),
    /// used for mail bodies in the geek theme.
    static let readingName = "iAWriterMonoV-Regular"

    /// Registers the bundled fonts for this process only (they are not
    /// installed on the system). Called once at launch.
    static func register() {
        guard let dir = Bundle.module.url(forResource: "Fonts", withExtension: nil),
              let files = try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
        else { return }
        for url in files where url.pathExtension == "ttf" {
            // An error here (font already installed by the user, for one)
            // is harmless: the font is available either way.
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }

    /// The geek font at `size` and `weight`, or the system monospaced font
    /// if the bundled one could not be loaded. The family has Regular,
    /// Semibold and Bold instances, chosen by PostScript name.
    static func reading(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        let name: String
        switch weight {
        case .bold, .heavy, .black: name = "iAWriterMonoV-Bold"
        case .semibold: name = "iAWriterMonoV-Semibold"
        default: name = readingName
        }
        return NSFont(name: name, size: size) ?? .monospacedSystemFont(ofSize: size, weight: weight)
    }

    /// Width of one character of the geek font at `size`. The font is
    /// monospaced, so a column of `n` characters is `n * charWidth` wide:
    /// list columns are sized this way, and stay aligned row after row.
    static func charWidth(_ size: CGFloat) -> CGFloat {
        ("0" as NSString).size(withAttributes: [.font: reading(size)]).width
    }

    /// `@font-face` rules embedding the bundled fonts as data URLs, for the
    /// HTML reader: the web view runs in another process, which does not see
    /// the fonts registered by the app, and does not reliably read local
    /// files. Built once (about 370 KB of text), then reused.
    static let cssFontFaces: String = {
        guard let dir = Bundle.module.url(forResource: "Fonts", withExtension: nil) else { return "" }
        func face(_ file: String, _ style: String) -> String {
            guard let data = try? Data(contentsOf: dir.appendingPathComponent(file)) else { return "" }
            return "@font-face { font-family: \"NM Mono\"; font-style: \(style); "
                + "src: url(data:font/ttf;base64,\(data.base64EncodedString())) format(\"truetype\"); }\n"
        }
        return face("iAWriterMonoV.ttf", "normal") + face("iAWriterMonoV-Italic.ttf", "italic")
    }()
}

/// Colors of the geek theme, inspired by Helix's default themes: a dark
/// and a light variant, chosen by macOS appearance.
///
/// Swift note: `NSColor(name:dynamicProvider:)` creates a color that AppKit
/// re-evaluates when the appearance changes, so views redraw themselves in
/// the right variant without any code of ours.
enum Palette {
    private static func dynamic(dark: UInt32, light: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(rgb: isDark ? dark : light)
        })
    }

    static let background = dynamic(dark: 0x1B1D23, light: 0xFAFAF8)
    static let text = dynamic(dark: 0xD5D8DE, light: 0x24262B)
    static let muted = dynamic(dark: 0x6E7380, light: 0x8A8D94)
    /// Accent, Helix's lilac.
    static let accent = dynamic(dark: 0xA4A0E8, light: 0x6B5FD6)
    /// Row under the cursor.
    static let cursor = dynamic(dark: 0x2E3140, light: 0xE7E5F7)
    /// Rows marked for a grouped action.
    static let marked = dynamic(dark: 0x3A2E45, light: 0xF3E3EE)
    static let markedBar = dynamic(dark: 0xE08AC0, light: 0xC2408F)
    static let statusBar = dynamic(dark: 0x24262E, light: 0xEDEDEA)
    static let popup = dynamic(dark: 0x262833, light: 0xFFFFFF)
    static let separator = dynamic(dark: 0x30333C, light: 0xDDDDD8)
    static let unread = dynamic(dark: 0xE6C07B, light: 0xB07D16)
    static let error = dynamic(dark: 0xE06C75, light: 0xC0392B)
}

extension NSColor {
    /// A color from `0xRRGGBB`.
    convenience init(rgb: UInt32) {
        self.init(
            srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255,
            green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255,
            alpha: 1
        )
    }
}

extension Font {
    /// Font of the whole geek theme: iA Writer Mono V (`ui.md`).
    static func geek(_ size: CGFloat = 12, weight: Font.Weight = .regular) -> Font {
        let nsWeight: NSFont.Weight
        switch weight {
        case .bold, .heavy, .black: nsWeight = .bold
        case .semibold: nsWeight = .semibold
        default: nsWeight = .regular
        }
        return Font(Fonts.reading(size, weight: nsWeight))
    }
}
