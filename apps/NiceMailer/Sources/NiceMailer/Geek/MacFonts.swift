import AppKit

/// The fonts of the formatting bar's menu (`specifications/edition.md`,
/// section 5, "Polices"): every font family of the Mac, and the ones
/// chosen last.
///
/// The core cannot list them (it may run on another machine); it gives the
/// fonts found on every system with their fallbacks (`compose_formats`,
/// `nm_core::formats::FONTS`), and keeps any font with a safe name when the
/// mail leaves (`font_stack`). Here each family of the Mac gets its CSS
/// stack: the core's when it has one, else its name and its generic family
/// (`'Avenir Next', sans-serif`), so that a reader without the font still
/// reads a serif, a sans serif or a fixed width font.
@MainActor
enum MacFonts {
    /// The key of the recent fonts in the app's defaults.
    static let recentKey = "editor.recentFonts"
    /// How many recent fonts are remembered (the editor shows five).
    static let recentCount = 5

    /// The menu's fonts: the families of the Mac, in their order (by name),
    /// those of the core's list with its stack.
    ///
    /// Reading each family's traits takes about half a second for 350
    /// fonts: `prepare` does it once in the background, when the editor is
    /// created (after the first list). A draft opened before it is done
    /// gets the families with `sans-serif`, at once.
    static func fonts(known: [ComposeFormats.Font]) -> [ComposeFormats.Font] {
        let families = cached ?? NSFontManager.shared.availableFontFamilies.filter(safe).map { ($0, "sans-serif") }
        return list(families: families, known: known)
    }

    /// The families and their generic family, once read (`prepare`).
    private static var cached: [(name: String, generic: String)]?

    /// Reads the families and their generic family in the background.
    ///
    /// Swift note: `Task.detached` runs its closure off the main actor; the
    /// result comes back with `await MainActor.run`, where `cached` lives.
    /// `nonisolated` lets the background code call `read()` though the
    /// type is `@MainActor`.
    static func prepare() {
        guard cached == nil, !preparing else { return }
        preparing = true
        Task.detached(priority: .utility) {
            let families = read()
            await MainActor.run { cached = families }
        }
    }

    private static var preparing = false

    nonisolated private static func read() -> [(name: String, generic: String)] {
        NSFontManager.shared.availableFontFamilies.filter(safe).map { ($0, generic(of: $0)) }
    }

    /// The menu for these families (pure, for the tests).
    static func list(families: [(name: String, generic: String)], known: [ComposeFormats.Font]) -> [ComposeFormats.Font] {
        families.map { f in
            known.first { $0.name.caseInsensitiveCompare(f.name) == .orderedSame }
                ?? ComposeFormats.Font(name: f.name, stack: "'\(f.name)', \(f.generic)")
        }
    }

    /// Whether the core keeps this name (`nm_core::formats::font_stack`):
    /// letters, digits, spaces and `- . _ &`, starting with a letter or a
    /// digit, 80 characters at most. The Mac's hidden families (`.SF NS`)
    /// start with a dot.
    nonisolated static func safe(_ name: String) -> Bool {
        guard let first = name.unicodeScalars.first, CharacterSet.alphanumerics.contains(first), name.count <= 80 else { return false }
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: " -._&"))
        return name.unicodeScalars.allSatisfy { allowed.contains($0) }
    }

    /// The CSS generic family of a font family, from its first face's
    /// traits: `monospace`, `serif` (the serif classes of the font's
    /// family class), `cursive` (scripts), else `sans-serif`.
    nonisolated static func generic(of family: String) -> String {
        guard let face = NSFontManager.shared.availableMembers(ofFontFamily: family)?.first,
              let postScript = face.first as? String,
              let font = NSFont(name: postScript, size: 12) else { return "sans-serif" }
        let traits = font.fontDescriptor.symbolicTraits
        if traits.contains(.monoSpace) { return "monospace" }
        // The family class lives in the top four bits of the traits
        // (`NSFontDescriptor.SymbolicTraits.familyClassMask`, 0xF0000000).
        let familyClass = traits.rawValue >> 28
        switch familyClass {
        case 1...7: return "serif"  // old style, transitional, modern, clarendon, slab, (reserved), freeform
        case 10: return "cursive"  // scripts
        default: return "sans-serif"
        }
    }

    // MARK: Recent fonts

    /// The fonts chosen last, most recent first.
    static var recent: [String] {
        UserDefaults.standard.stringArray(forKey: recentKey) ?? []
    }

    /// A font chosen in the menu (the editor's `fontUsed`).
    static func used(_ name: String) {
        UserDefaults.standard.set(adding(name, to: recent), forKey: recentKey)
    }

    /// `recent` with `name` first, once, at most `recentCount` (pure).
    static func adding(_ name: String, to recent: [String]) -> [String] {
        Array(([name] + recent.filter { $0 != name }).prefix(recentCount))
    }
}
