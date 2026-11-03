// swift-tools-version:6.0
// The first line is required: it tells SwiftPM which manifest format to use.
import PackageDescription

// Swift Package manifest: describes what to build. `swift build` and
// `swift run` read it; no Xcode project is needed.
let package = Package(
    name: "NiceMailer",
    // `@Observable` and `.onKeyPress` need macOS 14.
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "NiceMailer",
            path: "Sources/NiceMailer",
            // Swift 5 language mode: Swift 6's strict concurrency checking
            // is not enabled yet, to keep this first version simple.
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
