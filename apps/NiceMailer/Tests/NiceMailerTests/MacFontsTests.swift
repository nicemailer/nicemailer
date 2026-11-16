import Testing
@testable import NiceMailer

/// The fonts of the formatting bar's menu (`MacFonts`, `edition.md`,
/// section 5, "Polices").
@MainActor
struct MacFontsTests {
    @Test func everyFamilyGetsAStackTheCoreKeeps() {
        let known = [ComposeFormats.Font(name: "Georgia", stack: "Georgia, 'Times New Roman', serif")]
        let menu = MacFonts.list(families: [("Avenir Next", "sans-serif"), ("georgia", "serif"), ("Menlo", "monospace")], known: known)
        #expect(menu.map(\.stack) == ["'Avenir Next', sans-serif", "Georgia, 'Times New Roman', serif", "'Menlo', monospace"])
    }

    @Test func onlySafeNamesAreOffered() {
        #expect(MacFonts.safe("Hiragino Kaku Gothic ProN"))
        #expect(MacFonts.safe("Noto Sans Myanmar"))
        #expect(MacFonts.safe("Big Caslon"))
        #expect(!MacFonts.safe(".SF NS"))
        #expect(!MacFonts.safe("Font's"))
        #expect(!MacFonts.safe("x;y"))
        #expect(!MacFonts.safe(""))
        #expect(!MacFonts.safe(String(repeating: "A", count: 81)))
    }

    @Test func genericFamilyComesFromTheFontsTraits() {
        #expect(MacFonts.generic(of: "Menlo") == "monospace")
        #expect(MacFonts.generic(of: "Georgia") == "serif")
        #expect(MacFonts.generic(of: "Helvetica") == "sans-serif")
        #expect(MacFonts.generic(of: "No Such Font") == "sans-serif")
    }

    @Test func theMacsFontsAreAllOffered() {
        let menu = MacFonts.fonts(known: [])
        #expect(menu.contains { $0.name == "Helvetica" })
        #expect(!menu.contains { $0.name.hasPrefix(".") })
    }

    @Test func aFontUsedComesFirstOnce() {
        #expect(MacFonts.adding("Arial", to: []) == ["Arial"])
        #expect(MacFonts.adding("Arial", to: ["Georgia", "Arial", "Menlo"]) == ["Arial", "Georgia", "Menlo"])
        #expect(MacFonts.adding("F", to: ["A", "B", "C", "D", "E"]) == ["F", "A", "B", "C", "D"])
    }
}
