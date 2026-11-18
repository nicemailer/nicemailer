import Foundation
import Testing
@testable import NiceMailer

/// Formatting, column layout and translations of the geek theme.
@MainActor
struct FormatTests {
    /// The interface's own formatters, in the Mac's locale (the other
    /// locales: `LocalizationTests`).
    @Test func listDatesFitTheirEightCharacterColumn() {
        let now = Int(Date().timeIntervalSince1970)
        for days in [0, 10, 40, 200, 3 * 365] {
            #expect(Format.geekListDate(now - days * 86_400).count <= 8)
        }
    }

    @Test func rowColumnsHaveFixedWidthsInCharacters() {
        let narrow = RowLayout(width: 300)
        let wide = RowLayout(width: 2000)
        #expect(narrow.char > 0)
        #expect(narrow.date == 8 * narrow.char)
        // The sender column stays between 8 and 24 characters.
        #expect(narrow.sender >= 8 * narrow.char - 0.001)
        #expect(wide.sender <= 24 * wide.char + 0.001)
        // Same width for every row: the layout depends on the list width only.
        #expect(RowLayout(width: 900) == RowLayout(width: 900))
    }

    @Test func theAccountFollowsTheFolderOnlyIfTheSubjectKeepsRoom() {
        // A narrow list (2 columns in a 1400 pt window) shows the folder
        // alone; a wide one (1 column) adds the account.
        let narrow = RowLayout(width: 450, scale: 1)
        let wide = RowLayout(width: 1400, scale: 1)
        #expect(!narrow.showsAccount)
        #expect(wide.showsAccount)
        #expect(wide.subjectWidth(folderWidth: wide.folder(withAccount: true)) >= 30 * wide.char)
        // Paperclip and flag: one fixed icon column each, so rows with and
        // without them line up; the subject gets what is left.
        #expect(wide.icon >= wide.char && wide.icon <= 2 * wide.char)
        let fixed = 2 + wide.char + wide.date + wide.sender + 2 * wide.icon + wide.count + 8 * wide.char + 6
        #expect(wide.subjectWidth(folderWidth: 0) == wide.width - fixed)
        // The folder column is 10 characters, 18 with the account.
        #expect(abs(narrow.folder(withAccount: false) - 10 * narrow.char) < 0.001)
        #expect(abs(narrow.folder(withAccount: true) - 18 * narrow.char) < 0.001)
    }

    @Test func aSubjectInAStatusMessageIsTidiedAndCut() {
        #expect(Format.flashSubject("Devis toiture") == "Devis toiture")
        // Folded header lines and repeated spaces become one space.
        #expect(Format.flashSubject("  Re: devis\r\n\t toiture ") == "Re: devis toiture")
        #expect(Format.flashSubject("   ") == L("(no subject)"))
        let long = String(repeating: "a", count: 80)
        let cut = Format.flashSubject(long)
        #expect(cut.count == 50 && cut.hasSuffix("…"))
        #expect(Format.flashSubject(String(repeating: "b", count: 50)).count == 50)  // fits: not cut
    }

    @Test func theBundledFontIsAvailable() {
        Fonts.register()
        #expect(Fonts.reading(12).fontName.hasPrefix("iAWriterMonoV"))
        #expect(!Fonts.cssFontFaces.isEmpty)
    }

    /// Every string of the source catalog has a translation, in every
    /// language of the catalog, in the tables shipped with the app:
    /// `Localizable.strings`, and `Localizable.stringsdict` for the keys
    /// with plural forms. (The catalog itself is compiled into
    /// per-language tables, so it is read from the repository.)
    @Test func everyStringIsTranslatedIntoEveryLanguage() throws {
        let catalog = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/NiceMailer/Resources/Localizable.xcstrings")
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: catalog)) as? [String: Any]
        let strings = json?["strings"] as? [String: [String: Any]] ?? [:]
        #expect(strings.count > 50)
        // The languages are those the catalog translates into, French and
        // the ones added after it alike. English is the source: its key is
        // its text, and its table holds only the plural forms.
        let languages = Set(strings.values.flatMap { ($0["localizations"] as? [String: Any] ?? [:]).keys }).subtracting(["en"])
        #expect(languages.isSuperset(of: ["fr", "de", "ja", "ru", "zh-Hans", "zh-Hant"]))
        for language in languages.sorted() {
            // SwiftPM writes the lproj folder name in lower case.
            let path = try #require(Bundle.module.path(forResource: language, ofType: "lproj")
                                    ?? Bundle.module.path(forResource: language.lowercased(), ofType: "lproj"), "\(language)")
            var table = try #require(NSDictionary(contentsOfFile: path + "/Localizable.strings") as? [String: String], "\(language)")
            let plurals = NSDictionary(contentsOfFile: path + "/Localizable.stringsdict") as? [String: Any] ?? [:]
            #expect(plurals.count >= 20, "\(language)")
            for (key, entry) in plurals where (entry as? [String: Any])?["NSStringLocalizedFormatKey"] != nil {
                table[key] = key
            }
            for key in strings.keys {
                #expect(table[key]?.isEmpty == false, "missing \(language) for \(key)")
            }
        }
    }

    /// Every `L("…")` of the sources is in the catalog: a string added to
    /// the UI without a translation fails the tests.
    @Test func everyUIStringOfTheSourcesIsInTheCatalog() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let catalog = root.appendingPathComponent("Sources/NiceMailer/Resources/Localizable.xcstrings")
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: catalog)) as? [String: Any]
        let known = Set((json?["strings"] as? [String: Any] ?? [:]).keys)
        let sources = root.appendingPathComponent("Sources/NiceMailer")
        let files = FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil)?
            .compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" } ?? []
        let pattern = try NSRegularExpression(pattern: #"\bL\("((?:[^"\\]|\\.)*)""#)
        var used = Set<String>()
        for file in files {
            let text = try String(contentsOf: file, encoding: .utf8)
            for m in pattern.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                if let r = Range(m.range(at: 1), in: text) { used.insert(String(text[r])) }
            }
        }
        #expect(used.count > 50)
        for key in used.sorted() where key != "%@" {
            #expect(known.contains(key), "not in the catalog: \(key)")
        }
    }
}
