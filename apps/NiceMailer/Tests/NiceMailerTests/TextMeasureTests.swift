import AppKit
import Foundation
import Testing
@testable import NiceMailer

/// Text measured in the font of the theme (`docs/themes.md`, "For
/// developers"): geek keeps its columns counted in characters, a
/// proportional font gets each column as wide as what it holds.
@MainActor
struct TextMeasureTests {
    let mono = TextMeasure.font(.monospacedSystemFont(ofSize: 12, weight: .regular),
                                bold: .monospacedSystemFont(ofSize: 12, weight: .bold))
    let sans = TextMeasure.font(.systemFont(ofSize: 12), bold: .boldSystemFont(ofSize: 12))

    @Test func aFontOfFixedPitchCountsCharacters() {
        #expect(mono.fixedPitch)
        #expect(mono.width("Inbox") == 5 * mono.char)
        #expect(mono.width("Inbox", bold: true) == 5 * mono.char)
        #expect(mono.average == mono.char)
        let fixed = TextMeasure.fixed(char: 7)
        #expect(fixed.width("abc") == 21)
    }

    @Test func aProportionalFontMeasuresEachText() {
        #expect(!sans.fixedPitch)
        #expect(sans.width("iii") < sans.width("MMM"))
        #expect(sans.width("Inbox", bold: true) >= sans.width("Inbox"))
        // Whole points: columns land on the pixel grid.
        #expect(sans.width("Inbox") == sans.width("Inbox").rounded(.up))
        #expect(sans.average > 0 && sans.average < sans.width("M"))
    }

    /// geek: the layout of a font of fixed pitch is the one counted in
    /// characters, exactly, whatever the width of the list.
    @Test func geekKeepsItsColumnsInCharacters() {
        let m = TextMeasure.fixed(char: 7.224609375)
        for width in [300.0, 450, 640, 900, 1400, 2000] {
            let l = RowLayout(width: width, scale: 1, measure: m)
            #expect(l.char == m.char)
            #expect(l.date == 8 * m.char)
            #expect(l.count == 3 * m.char)
            #expect(l.dot == m.char)
            #expect(l.sender == min(24, max(8, (width / m.char * 0.26).rounded(.down))) * m.char)
        }
    }

    /// A proportional font: the date column holds every date the list can
    /// show, and no more; the count holds three digits.
    @Test func aProportionalListIsAsNarrowAsItsTexts() {
        let l = RowLayout(width: 900, scale: 1, measure: sans)
        let f = Format.dates
        for d in Format.samples(calendar: f.geekDayMonth.calendar, timeZone: f.geekDayMonth.timeZone) {
            for text in [f.geekTime.string(from: d), f.geekDayMonth.string(from: d), f.geekShort.string(from: d)] {
                #expect(sans.width(text) <= l.date, "\(text)")
            }
        }
        // Narrower than eight digits: the space goes to the subject.
        #expect(l.date < 8 * sans.char)
        #expect(l.count == sans.width("999"))
        // The unread dot is wider than a digit in SF Pro: its own width,
        // or it is cut in half.
        #expect(l.dot == sans.width("●") && l.dot > sans.char)
        #expect(l.subjectWidth(folderWidth: 0) > RowLayout(width: 900, scale: 1, measure: .fixed(char: sans.char)).subjectWidth(folderWidth: 0))
    }

    /// The help: in characters for a font of fixed pitch (as before), each
    /// text measured for a proportional one, the titles in bold.
    @Test func theHelpIsMeasuredInTheFont() {
        let groups = Help.groups(.defaults, severalAccounts: true)
        let columns = Help.columns(groups, count: 2)
        #expect(Help.width(columns, .fixed(char: 7)) == CGFloat(Help.width(columns)) * 7)
        let inSans = Help.width(columns, sans)
        #expect(inSans > 0)
        // Enough room for the widest layout in SF Pro: three columns.
        let three = Help.width(Help.columns(groups, count: 3), sans)
        #expect(Help.columnCount(groups, available: three + 20, measure: sans, padding: 10) == 3)
        #expect(Help.columnCount(groups, available: 50, measure: sans, padding: 10) == 1)
        // The width in characters and the one of `.fixed` agree.
        #expect(Help.columnCount(groups, available: 900, charWidth: 7, padding: 10)
                == Help.columnCount(groups, available: 900, measure: .fixed(char: 7), padding: 10))
    }

    @Test func aFontIsFoundByItsName() {
        #expect(Fonts.named("system", size: 12).familyName == NSFont.systemFont(ofSize: 12).familyName)
        #expect(Fonts.named("system-mono", size: 12).isFixedPitch)
        #expect(Fonts.named("system-serif", size: 12).fontDescriptor.symbolicTraits.contains(.monoSpace) == false)
        #expect(Fonts.named("Helvetica", size: 12).familyName == "Helvetica")
        // A family not installed: the system font stands in, and the app
        // knows it is missing.
        #expect(Fonts.named("No Such Font", size: 12).familyName == NSFont.systemFont(ofSize: 12).familyName)
        #expect(!Fonts.isAvailable("No Such Font"))
        #expect(Fonts.isAvailable("system") && Fonts.isAvailable("Helvetica") && Fonts.isAvailable(Fonts.readingFamily))
    }

    /// A theme naming a font the Mac does not have: the status bar says so
    /// when the theme arrives.
    @Test func aMissingFontIsSaidOnce() async {
        var clean = ThemeTests.themeJSON("clean")
        var fonts = clean["fonts"] as! [String: Any]
        fonts["reading"] = "No Such Font"
        clean["fonts"] = fonts
        let core = FakeCore { method, params in
            method == "theme_get" ? clean : Fixture.answer(method, params)
        }
        let model = AppModel()
        model.theme = ThemeStore()
        await model.attach(core)
        #expect(model.flash == L("Font “%@” not installed: using the system font", "No Such Font"))
        model.flash = nil
        await model.loadTheme()
        #expect(model.flash == nil, "the same theme again: said once")
    }
}

/// The last column of a row (the flag) stays inside the rounded fill of
/// the selected row (`RowLayout.trailingPadding`, `layout.row_inset`).
@MainActor
struct RowTrailingPaddingTests {
    @Test func theFlagStaysInsideTheFillOfEveryTheme() {
        for name in ["geek", "clean", "regular"] {
            let theme = ThemeStore.builtins[name]!
            let inset = scaled(CGFloat(theme.layout.rowInset))
            // 6 pt between the flag's column and the fill's right edge.
            #expect(RowLayout.trailingPadding(theme) == inset + 6, "\(name)")
        }
        // clean and regular inset their fill by 8: 14 after the flag.
        #expect(RowLayout.trailingPadding(ThemeStore.builtins["clean"]!) == 6 + scaled(8))
        #expect(RowLayout.trailingPadding(ThemeStore.builtins["geek"]!) == 6)
    }

    @Test func theSubjectGivesUpTheRoomAfterTheFlag() {
        // The subject's width is what the other columns and the trailing
        // padding of the current theme leave.
        let l = RowLayout(width: 900, scale: 1, measure: .fixed(char: 7))
        let columns = 2 + l.dot + l.date + l.sender + l.icon + l.count + l.icon + 8 * l.char
        #expect(abs(l.subjectWidth(folderWidth: 0) - (900 - columns - RowLayout.trailingPadding)) < 0.001)
    }
}
