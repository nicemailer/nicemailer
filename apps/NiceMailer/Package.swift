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
    targets: [
        .executableTarget(
            name: "NiceMailer",
            path: "Sources/NiceMailer",
            // Files bundled with the app, reachable through `Bundle.module`.
            // `.copy` keeps the Fonts folder as is (fonts are registered at
            // launch, see `Fonts.register`); `.process` compiles the String
            // Catalog into per-language `.strings` files.
            resources: [
                .copy("Resources/Fonts"),
                .process("Resources/Localizable.xcstrings"),
            ],
            // Swift 5 language mode: Swift 6's strict concurrency checking
            // is not enabled yet, to keep this first version simple.
            swiftSettings: [.swiftLanguageMode(.v5)]
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
