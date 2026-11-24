// swift-tools-version:6.0
// The first line is required: it tells SwiftPM which manifest format to use.
import PackageDescription

// Swift Package manifest: describes what to build. `swift build` and
// `swift run` read it; no Xcode project is needed.
let package = Package(
    name: "NiceMailer",
    // Source language of the user-facing strings. Translations live in the
    // String Catalog `Resources/Localizable.xcstrings` (see CLAUDE.md,
    // "Languages": English and French, English as fallback).
    defaultLocalization: "en",
    // `@Observable` and `.onKeyPress` need macOS 14.
    platforms: [.macOS(.v14)],
    // Sparkle updates the app from the releases of NiceMailer
    // (`docs/updates.md`). An exact version: the framework is shipped
    // inside the app and signed with it, and a release must be rebuildable
    // with the same one. Its binary comes from Sparkle's GitHub release.
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.10.1"),
    ],
    targets: [
        .executableTarget(
            name: "NiceMailer",
            dependencies: [.product(name: "Sparkle", package: "Sparkle")],
            path: "Sources/NiceMailer",
            // Files bundled with the app, reachable through `Bundle.module`.
            // `.copy` keeps the Fonts folder as is (fonts are registered at
            // launch, see `Fonts.register`); `.process` compiles the String
            // Catalog into per-language `.strings` files.
            resources: [
                .copy("Resources/Fonts"),
                // Dark Reader, run in the HTML reader (`DarkMode.swift`).
                .copy("Resources/DarkReader"),
                // The editor of a mail's text (`apps/editor`, built by
                // `make editor`, never committed; `EditorWebView.swift`).
                .copy("Resources/Editor"),
                // The default keys, written by the core (`nicemailer keymap
                // show --defaults --json`; `Keymap.defaults`).
                .copy("Resources/keymap-defaults.json"),
                // The built-in themes resolved, written by the core
                // (`nicemailer theme show --builtins --json`, `docs/themes.md`).
                .copy("Resources/themes-builtin.json"),
                .process("Resources/Localizable.xcstrings"),
            ],
            // Swift 5 language mode: Swift 6's strict concurrency checking
            // is not enabled yet, to keep this first version simple.
            swiftSettings: [.swiftLanguageMode(.v5)],
            // Where the app looks for Sparkle.framework: in `Contents/
            // Frameworks` of the bundle (`scripts/make-bundle.sh`), and next
            // to the executable, where SwiftPM puts it (`swift run`, the UI
            // scenarios). SwiftPM adds neither by itself.
            linkerSettings: [.unsafeFlags([
                "-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks",
                "-Xlinker", "-rpath", "-Xlinker", "@executable_path",
            ])]
        ),
        // Unit tests (`swift test`): selection rules, keyboard handling of the
        // geek theme against a fake core, formatting and localization. The
        // scripted UI tests are in `UITests/` (`make ui-test`).
        .testTarget(
            name: "NiceMailerTests",
            dependencies: ["NiceMailer"],
            path: "Tests/NiceMailerTests",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
