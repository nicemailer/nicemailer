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

/// The bundle of the app's resources (fonts, translations).
///
/// SwiftPM puts them in `NiceMailer_NiceMailer.bundle`, and its generated
/// `Bundle.module` looks for it beside the executable (`swift run`) or at
/// the root of an `.app`, where macOS's code signature allows nothing.
/// `make bundle` puts it in `Contents/Resources`, the regular place, which
/// is looked at first; `Bundle.module` covers `swift run` and the tests.
enum AppResources {
    static let bundle: Bundle = {
        if let url = Bundle.main.resourceURL?.appendingPathComponent("NiceMailer_NiceMailer.bundle"),
           let bundle = Bundle(url: url) {
            return bundle
        }
        return Bundle.module
    }()
}

/// The app's icon drawn small, for the status bar (`Resources/Icon`, made
/// by `scripts/make-icon.sh` from the icon's small-size drawing). Loaded
/// once; nil if the file is missing, the bar then writes the mode.
enum AppIcon {
    static let statusImage: NSImage? = AppResources.bundle
        .url(forResource: "StatusIcon", withExtension: "png", subdirectory: "Icon")
        .flatMap(NSImage.init(contentsOf:))
}

/// Localized string from the app's String Catalog.
///
/// Swift note: in a Swift Package, resources live in their own bundle
/// (`AppResources.bundle`), not in `Bundle.main`, so SwiftUI's default lookup would not find the
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
    if let path = AppResources.bundle.path(forResource: appLanguage, ofType: "lproj"),
       let bundle = Bundle(path: path) {
        return bundle
    }
    return AppResources.bundle
}()

/// The language the app shows (`fr`, `en`): the best match between the
/// user's languages and the translations bundled, English when none
/// matches.
let appLanguage: String = {
    let available = AppResources.bundle.localizations
    return Bundle.preferredLocalizations(from: available, forPreferences: Locale.preferredLanguages).first ?? "en"
}()

/// The locale of the language the app shows, alone, without the Mac's
/// region: the language asked of the AI for its answers (`AI.language`).
/// Everything the app writes, interface and mail, uses `uiLocale`.
let appLocale = Locale(identifier: appLanguage)

/// The locale of the dates and numbers the app writes, in its interface
/// and in the mails (the line introducing a quotation; `docs/localization.md`).
/// The words come from the app's language (month and day names), the rest
/// from the Mac's settings: the order of day and month, the 12- or 24-hour
/// clock. An English interface on a Mac set to France writes "14/11/2023"
/// and "23:13", not "11/14/23" and "11:13 PM".
let uiLocale = interfaceLocale(language: appLanguage, settings: .current)

/// `uiLocale` made of its two sources, apart so that the tests can give
/// any pair.
///
/// Swift note: `Locale.Components` is the editable form of a locale. Built
/// from a locale, it does not keep its region nor the hour cycle chosen in
/// System Settings, so both are copied explicitly.
func interfaceLocale(language: String, settings: Locale) -> Locale {
    var c = Locale.Components(locale: settings)
    c.languageComponents = Locale.Language.Components(identifier: language)
    c.region = settings.region
    c.hourCycle = settings.hourCycle
    return Locale(components: c)
}

/// Same, with values inserted: `L("%lld marked", n)`. The catalog stores the
/// format with its placeholders (`%@` for text, `%lld` for integers).
///
/// The locale matters: a key whose text depends on a number (`%lld mails`)
/// has plural variations in the catalog, and `String(format:)` picks the
/// form by the rules of the locale it is given. Without it, the English
/// rules apply to every language: French would write "0 mails", where 0
/// takes the singular.
func L(_ key: String, _ args: CVarArg...) -> String {
    String(format: L(key), locale: uiLocale, arguments: args)
}

/// What `L` gives when the app shows `language` (`fr`), whatever the
/// language of the Mac: for the tests, which check every translation.
func L(in language: String, _ key: String, _ args: CVarArg...) -> String {
    guard let path = AppResources.bundle.path(forResource: language, ofType: "lproj"),
          let bundle = Bundle(path: path) else { return key }
    let format = bundle.localizedString(forKey: key, value: key, table: nil)
    return String(format: format, locale: Locale(identifier: language), arguments: args)
}

/// The fonts shipped with the app.
enum Fonts {
    /// iA Writer Mono V (SIL Open Font License, `Resources/Fonts/LICENSE.md`),
    /// used for mail bodies in the geek theme.
    static let readingName = "iAWriterMonoV-Regular"

    /// Registers the bundled fonts for this process only (they are not
    /// installed on the system). Called once at launch.
    static func register() {
        guard let dir = AppResources.bundle.url(forResource: "Fonts", withExtension: nil),
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
        guard let dir = AppResources.bundle.url(forResource: "Fonts", withExtension: nil) else { return "" }
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
    /// A colour of the theme, dark and light (`#rrggbb`).
    struct Pair {
        let dark: UInt32
        let light: UInt32
        func hex(dark isDark: Bool) -> String { String(format: "#%06x", isDark ? dark : light) }
    }

    private static func dynamic(_ p: Pair) -> Color { dynamic(dark: p.dark, light: p.light) }

    private static func dynamic(dark: UInt32, light: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(rgb: isDark ? dark : light)
        })
    }

    /// The dark background and text, also given to Dark Reader so that a
    /// mail it darkens takes the theme's own colours (`DarkMode`).
    static let darkBackground: UInt32 = 0x1B1D23
    /// A soft white, a little dimmed and warmed towards the green of the
    /// accent: the bright #D5D8DE glared on the dark background. Still
    /// 9.8:1 on it.
    static let darkText: UInt32 = 0xC3C6C1

    static let backgroundPair = Pair(dark: darkBackground, light: 0xFAFAF8)
    static let background = dynamic(backgroundPair)
    static let textPair = Pair(dark: darkText, light: 0x24262B)
    static let text = dynamic(textPair)
    static let mutedPair = Pair(dark: 0x6E7380, light: 0x8A8D94)
    static let muted = dynamic(mutedPair)
    /// Accent: the green of the app's icon, rgb(138 165 129)
    /// (`design/icon/README.md`), for what is not text (a frame).
    static let accent = dynamic(dark: 0x8AA581, light: 0x8AA581)
    /// The accent for text and small marks (keys of the menus, tags, the
    /// mode, the caret): the icon's green in the dark theme (6.2:1 on its
    /// background), a darker green of the same hue in the light one, where
    /// the icon's would read at 2.6:1 only; this one reads at 5.3:1.
    static let accentTextPair = Pair(dark: 0x8AA581, light: 0x56704E)
    static let accentText = dynamic(accentTextPair)
    /// Row under the cursor: a green of the accent's hue, a little more
    /// apart from the background than a plain tint (text on it 11:1 light,
    /// 7.4:1 dark).
    static let cursorPair = Pair(dark: 0x33432F, light: 0xD3E0CE)
    static let cursor = dynamic(cursorPair)
    /// Rows marked for a grouped action.
    static let marked = dynamic(dark: 0x3A2E45, light: 0xF3E3EE)
    static let markedBar = dynamic(dark: 0xE08AC0, light: 0xC2408F)
    static let statusBarPair = Pair(dark: 0x24262E, light: 0xEDEDEA)
    static let statusBar = dynamic(statusBarPair)
    static let popup = dynamic(dark: 0x262833, light: 0xFFFFFF)
    static let separatorPair = Pair(dark: 0x30333C, light: 0xDDDDD8)
    static let separator = dynamic(separatorPair)
    static let unread = dynamic(dark: 0xE6C07B, light: 0xB07D16)
    static let error = dynamic(dark: 0xE06C75, light: 0xC0392B)
    /// A flagged mail: orange, as in Apple Mail, and apart from the unread
    /// colour, so that the flag stands out at the end of its row.
    static let flag = dynamic(dark: 0xF0883E, light: 0xD9661C)
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

/// The text size of the geek theme, like Zed's ⌘+ / ⌘− (`ui.md`).
///
/// One base size (12 pt by default) from which every size of the theme
/// derives: fonts, row heights, column widths, menus, the HTML view. It is
/// remembered in the preferences.
@MainActor
@Observable
final class TextSize {
    static let shared = TextSize(defaults: AppDefaults.store)
    static let defaultSize: Double = 12
    static let range: ClosedRange<Double> = 9...24
    static let key = "geek.fontSize"

    @ObservationIgnored private let defaults: UserDefaults

    /// Base size in points.
    private(set) var size: Double {
        didSet { defaults.set(size, forKey: Self.key) }
    }

    /// Factor applied to the sizes written for the default base size.
    var scale: CGFloat { CGFloat(size / Self.defaultSize) }

    init(defaults: UserDefaults) {
        self.defaults = defaults
        let stored = defaults.object(forKey: Self.key) as? Double ?? Self.defaultSize
        size = min(Self.range.upperBound, max(Self.range.lowerBound, stored))
    }

    /// One point bigger (⌘+), within `range`.
    func bigger() { size = min(Self.range.upperBound, size + 1) }
    /// One point smaller (⌘−), within `range`.
    func smaller() { size = max(Self.range.lowerBound, size - 1) }
    /// Back to the default size (⌘0).
    func reset() { size = Self.defaultSize }
}

/// A length of the geek layout, written for the default text size, scaled
/// to the current one (row heights, column widths, popup sizes).
@MainActor
func scaled(_ value: CGFloat) -> CGFloat { value * TextSize.shared.scale }

extension Fonts {
    /// The geek font at a size written for the default text size, scaled
    /// to the current one.
    @MainActor
    static func scaled(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        reading(size * TextSize.shared.scale, weight: weight)
    }
}

extension Font {
    /// Font of the whole geek theme: iA Writer Mono V (`ui.md`), at a size
    /// written for the default text size and scaled to the current one.
    @MainActor
    static func geek(_ size: CGFloat = 12, weight: Font.Weight = .regular) -> Font {
        let nsWeight: NSFont.Weight
        switch weight {
        case .bold, .heavy, .black: nsWeight = .bold
        case .semibold: nsWeight = .semibold
        default: nsWeight = .regular
        }
        return Font(Fonts.scaled(size, weight: nsWeight))
    }
}
