import AppKit
import CoreText
import SwiftUI

// Shared pieces of the interface (`ui.md`): the preferences, the
// localization helper, the bundled font and the geek palette.

/// Where the app keeps its preferences (columns, status bar…).
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

    /// Width of "0" in the interface font of the theme at `size`: the unit
    /// of the gaps of the layout (`TextMeasure.char`).
    static func charWidth(_ size: CGFloat) -> CGFloat {
        ("0" as NSString).size(withAttributes: [.font: interface(size)]).width
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

/// Colors of the theme in effect (`ThemeStore`, `themes.md`, section 6.5),
/// by the names the views have always used.
///
/// Each one is a dynamic color whose provider reads the theme's colors when
/// AppKit draws: the same `Color` value for the life of the app (views and
/// tests can compare them), its value following the theme and the
/// appearance. A new theme rebuilds the window (`ThemeStore.revision`), so
/// that nothing keeps a color SwiftUI resolved before.
///
/// Swift note: `NSColor(name:dynamicProvider:)` creates a color that AppKit
/// re-evaluates when the appearance changes, so views redraw themselves in
/// the right variant without any code of ours.
enum Palette {
    private static func dynamic(_ key: KeyPath<ThemeColors, ThemeColor>) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            ThemeStore.colors[keyPath: key].resolved(for: appearance)
        })
    }

    /// `#rrggbb` of a color of the theme for one appearance: for the pages
    /// of the editor and of Dark Reader, which take CSS values (a dynamic
    /// color converted out of its view does not reliably follow an
    /// appearance asked for).
    static func hex(_ key: KeyPath<ThemeColors, ThemeColor>, dark: Bool) -> String {
        ThemeStore.colors[keyPath: key].hex(dark: dark)
    }

    static let background = dynamic(\.background)
    /// The folder column (`colors.sidebar`).
    static let sidebar = dynamic(\.sidebar)
    /// The list of the mails (`colors.list`).
    static let list = dynamic(\.list)
    static let text = dynamic(\.text)
    /// Secondary text: dates, counts, headers.
    static let muted = dynamic(\.muted)
    /// Accent, for what is not text (a frame).
    static let accent = dynamic(\.accent)
    /// The accent for text and small marks (keys of the menus, tags, the
    /// mode, the caret).
    static let accentText = dynamic(\.accentText)
    /// Row under the cursor (`colors.selection`).
    static let cursor = dynamic(\.selection)
    /// Text on that row (`colors.selection_text`).
    static let selectionText = dynamic(\.selectionText)
    /// Rows marked for a grouped action.
    static let marked = dynamic(\.marked)
    static let markedBar = dynamic(\.markedBar)
    /// The status bar (`colors.bar`).
    static let statusBar = dynamic(\.bar)
    static let popup = dynamic(\.popup)
    static let separator = dynamic(\.separator)
    static let unread = dynamic(\.unread)
    static let error = dynamic(\.error)
    /// A flagged mail: orange, as in Apple Mail, and apart from the unread
    /// colour, so that the flag stands out at the end of its row.
    static let flag = dynamic(\.flag)
}

/// The text sizes of the geek theme (`docs/geek-ui.md`, "Text size").
///
/// Two base sizes, from which every size of the theme derives: `size` for
/// the whole window (fonts, row heights, column widths, menus, the HTML
/// view) and `statusBarSize` for the status bar alone. Their values come
/// from `config.toml` (`ui.text_size`, 16 pt by default, and
/// `ui.status_bar_text_size`, 18 pt); ⌘+ and ⌘− move both by one point for the session,
/// ⌘0 comes back to the file's values.
///
/// Why the step is not remembered across launches: the file is the one
/// place that says the size. A size remembered by the app as well would
/// silently win over an edit of the file.
@MainActor
@Observable
final class TextSize {
    static let shared = TextSize()
    /// What `ui.text_size` and `ui.status_bar_text_size` are when the file
    /// does not say (the same defaults as the core's, `config.rs`).
    static let defaultSize: Double = 16
    static let defaultStatusBarSize: Double = 18
    /// The bounds of both sizes, as the core checks them in the file
    /// (`TEXT_SIZE_RANGE`): ⌘+ and ⌘− stop there too.
    static let range: ClosedRange<Double> = 9...32
    /// The size the lengths of the layout are written for (`scaled`,
    /// `Font.interface`): they were all measured at 12 pt, so this stays 12
    /// whatever the default size is.
    static let referenceSize: Double = 12

    /// The values of `config.toml`, before the step of ⌘+ / ⌘−.
    private(set) var configured: Double = TextSize.defaultSize
    private(set) var configuredStatusBar: Double = TextSize.defaultStatusBarSize
    /// Points added by ⌘+ (and removed by ⌘−) since the launch or the last
    /// ⌘0, to both sizes.
    private(set) var step: Double = 0

    /// Base size of the window, in points.
    var size: Double { Self.clamped(configured + step) }
    /// Base size of the status bar, in points.
    var statusBarSize: Double { Self.clamped(configuredStatusBar + step) }

    /// Factor applied to the sizes written for `referenceSize`.
    var scale: CGFloat { CGFloat(size / Self.referenceSize) }
    /// The same, for the status bar (`barScaled`, `Font.interfaceBar`).
    var statusBarScale: CGFloat { CGFloat(statusBarSize / Self.referenceSize) }

    private static func clamped(_ v: Double) -> Double {
        min(range.upperBound, max(range.lowerBound, v))
    }

    /// Applies the sizes of `config.toml`, read from `config_status` at
    /// launch and after each `ConfigChanged`. `nil` (an older core) leaves
    /// the default. A changed value also clears the step: the user who
    /// edits the file wants the size written there, not that size plus an
    /// old ⌘+. An unchanged file keeps the step (the core announces every
    /// change of `config.toml`, not only those of these two keys).
    func configure(size: Double?, statusBar: Double?) {
        let new = Self.clamped(size ?? Self.defaultSize)
        let newBar = Self.clamped(statusBar ?? Self.defaultStatusBarSize)
        guard new != configured || newBar != configuredStatusBar else { return }
        configured = new
        configuredStatusBar = newBar
        step = 0
    }

    /// One point bigger (⌘+), as long as one of the two sizes can grow:
    /// once both are at the top, the step stops, so that ⌘− answers at
    /// the first press.
    func bigger() {
        guard size < Self.range.upperBound || statusBarSize < Self.range.upperBound else { return }
        step += 1
    }
    /// One point smaller (⌘−), as long as one of the two sizes can shrink.
    func smaller() {
        guard size > Self.range.lowerBound || statusBarSize > Self.range.lowerBound else { return }
        step -= 1
    }
    /// Back to the sizes of the file (⌘0).
    func reset() { step = 0 }

    /// What the status bar says after a change: the size of the window,
    /// and that of the status bar when it differs. Swift note: `Int(...)`
    /// is needed because `%lld` reads a 64-bit integer; a `Double` passed
    /// there would print garbage.
    var flash: String {
        size == statusBarSize
            ? L("Text size: %lld pt", Int(size))
            : L("Text size: %lld pt, status bar %lld pt", Int(size), Int(statusBarSize))
    }
}

/// A change of the text size, by key or by the View menu.
enum TextSizeChange {
    case bigger, smaller, reset
}

/// A length of the geek layout, written for the reference text size (12
/// pt), scaled to the current one (row heights, column widths, popup sizes).
@MainActor
func scaled(_ value: CGFloat) -> CGFloat { value * TextSize.shared.scale }

/// The same for the status bar, which has a text size of its own
/// (`ui.status_bar_text_size`).
@MainActor
func barScaled(_ value: CGFloat) -> CGFloat { value * TextSize.shared.statusBarScale }

extension Fonts {
    /// The interface font of the theme (`fonts.interface`) at a size
    /// written for the reference text size, scaled to the current one.
    @MainActor
    static func scaled(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        interface(size * TextSize.shared.scale, weight: weight)
    }

    /// The interface font of the theme in effect (`fonts.interface`,
    /// `docs/themes.md`) at `size` points: iA Writer Mono V in geek, SF
    /// Pro in clean and regular.
    static func interface(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        named(ThemeStore.active.fonts.interface, size: size, weight: weight)
    }

    /// The font a theme names (`themes.md`, section 5.2): a font of the
    /// system, the bundled iA Writer Mono V, or a family installed on the
    /// Mac. A family that is not installed gives the system font
    /// (`isAvailable` says so, for the message of the app).
    static func named(_ name: String, size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        switch name {
        case "system": return .systemFont(ofSize: size, weight: weight)
        case "system-mono": return .monospacedSystemFont(ofSize: size, weight: weight)
        case "system-serif", "system-rounded":
            // Swift note: a font descriptor describes a font without
            // loading it; `withDesign` asks for the serif (New York) or
            // rounded variant of the system font.
            let base = NSFont.systemFont(ofSize: size, weight: weight)
            let design: NSFontDescriptor.SystemDesign = name == "system-serif" ? .serif : .rounded
            guard let d = base.fontDescriptor.withDesign(design) else { return base }
            return NSFont(descriptor: d, size: size) ?? base
        case readingFamily: return reading(size, weight: weight)
        default:
            // NSFontManager's weights go from 0 to 15: 5 is regular, 8
            // semibold, 9 bold.
            let w: Int
            switch weight {
            case .bold, .heavy, .black: w = 9
            case .semibold: w = 8
            default: w = 5
            }
            return NSFontManager.shared.font(withFamily: name, traits: [], weight: w, size: size)
                ?? .systemFont(ofSize: size, weight: weight)
        }
    }

    /// Whether the font a theme names is there: a font of the system, the
    /// bundled one, or a family installed on the Mac.
    static func isAvailable(_ name: String) -> Bool {
        name.hasPrefix("system") || name == readingFamily
            || NSFontManager.shared.availableFontFamilies.contains(name)
    }

    /// The body of a mail in the reader: the reading font of the theme
    /// (`fonts.reading`), at a size written for the reference text size,
    /// times the theme's `reading_scale`, scaled to the current text size.
    @MainActor
    static func mailBody(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        let fonts = ThemeStore.active.fonts
        return named(fonts.reading, size: size * CGFloat(fonts.readingScale) * TextSize.shared.scale, weight: weight)
    }

    /// Points added between two lines of the body of a mail
    /// (`fonts.reading_line_spacing`), whatever the text size.
    static var mailLineSpacing: CGFloat { CGFloat(ThemeStore.active.fonts.readingLineSpacing) }

    /// A font of the theme as a CSS `font-family` list, for the web views
    /// (the HTML reader, the editor): the bundled font is "NM Mono", given
    /// to them as data (`cssFontFaces`); the fonts of the system have names
    /// of their own in WebKit.
    static func css(_ name: String) -> String {
        switch name {
        case readingFamily: return "\"NM Mono\", ui-monospace, monospace"
        case "system": return "-apple-system, system-ui, sans-serif"
        case "system-mono": return "ui-monospace, monospace"
        case "system-serif": return "ui-serif, serif"
        case "system-rounded": return "ui-rounded, -apple-system, sans-serif"
        default:
            let quoted = name.replacingOccurrences(of: "\\", with: "").replacingOccurrences(of: "\"", with: "")
            return "\"\(quoted)\", -apple-system, sans-serif"
        }
    }

    /// The family name of the bundled font, as a theme writes it.
    static let readingFamily = "iA Writer Mono V"
}

/// Measures text in a font, to size the columns of the list and the key
/// column of the menus and the help (`themes.md`, section 7).
///
/// With a font of fixed pitch (geek), a text of `n` characters is `n`
/// times the width of one: columns counted in characters line up row after
/// row, as they always did. With a proportional font (SF Pro), each text is
/// measured as it will be drawn.
struct TextMeasure {
    /// Width of "0": the unit of the gaps, and of a text in a font of
    /// fixed pitch. Digits have one width in SF Pro too (tabular).
    let char: CGFloat
    /// Whether every character has the width of `char`.
    let fixedPitch: Bool
    /// Width of a typical character of running text: the unit for a text
    /// not known in advance (the name of an attachment).
    let average: CGFloat
    private let measure: (String, Bool) -> CGFloat

    /// The width of `s`, regular or bold, rounded up to a whole point.
    func width(_ s: String, bold: Bool = false) -> CGFloat {
        fixedPitch ? CGFloat(s.count) * char : measure(s, bold)
    }

    /// Characters of a fixed width `char`: the tests, and geek.
    static func fixed(char: CGFloat) -> TextMeasure {
        TextMeasure(char: char, fixedPitch: true, average: char) { s, _ in CGFloat(s.count) * char }
    }

    /// Text in `font` (and its bold variant).
    static func font(_ font: NSFont, bold: NSFont) -> TextMeasure {
        func w(_ s: String, _ f: NSFont) -> CGFloat {
            (s as NSString).size(withAttributes: [.font: f]).width.rounded(.up)
        }
        let char = ("0" as NSString).size(withAttributes: [.font: font]).width
        let sample = "Thanks again for the workshop, see you next week"
        let average = (sample as NSString).size(withAttributes: [.font: font]).width / CGFloat(sample.count)
        // In a font of fixed pitch every character is `char` wide: the same
        // value, not a division that could round differently.
        return TextMeasure(char: char, fixedPitch: font.isFixedPitch, average: font.isFixedPitch ? char : average) { s, isBold in
            w(s, isBold ? bold : font)
        }
    }

    /// The interface font of the theme at `size` points.
    static func interface(_ size: CGFloat) -> TextMeasure {
        font(Fonts.interface(size), bold: Fonts.interface(size, weight: .bold))
    }
}

extension Font {
    /// Font of the whole geek theme: iA Writer Mono V (`ui.md`), at a size
    /// written for the default text size and scaled to the current one.
    @MainActor
    static func interface(_ size: CGFloat = 12, weight: Font.Weight = .regular) -> Font {
        Font(Fonts.scaled(size, weight: nsWeight(weight)))
    }

    /// The geek font of the status bar, scaled to its own text size.
    @MainActor
    static func interfaceBar(_ size: CGFloat = 12, weight: Font.Weight = .regular) -> Font {
        Font(Fonts.interface(size * TextSize.shared.statusBarScale, weight: nsWeight(weight)))
    }

    /// The AppKit weight of a SwiftUI one, among those the font has.
    private static func nsWeight(_ weight: Font.Weight) -> NSFont.Weight {
        switch weight {
        case .bold, .heavy, .black: .bold
        case .semibold: .semibold
        default: .regular
        }
    }
}
