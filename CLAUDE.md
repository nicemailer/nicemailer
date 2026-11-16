## Languages
- Code comments (Rust, Swift and TypeScript, including doc comments `///`, `//!` and `/** */`) and the README are written in English.
- French remains the reference language for the specifications (`specifications/`).
- The CLI (`nicemailer`) is in English only: clap `///` comments on CLI arguments (clap turns them into `--help` text), output and error messages.
- The Swift app is localized and follows the user's language preferences (macOS system or per-app language). English and French are supported by default; English is the fallback. No user-facing string is hard-coded in a single language: every UI string goes through localization (String Catalog).
- When quoting a spec section in a comment, keep its French title, e.g. (`dossiers.md`, "Cibles proposées").

## Commit messages  
- Commit messages must be written in English (this overrides the global rule).
- Never add attribution trailers to commit messages: no "Co-Authored-By", no "Claude-Session".

## Comment style

This is the owner's first Rust, Swift and TypeScript project. Comments should be detailed enough to teach:
- explain why the code does what it does (design choice, spec rule, trade-off), not only what;
- briefly explain language idioms the first time they appear in a file (ownership and borrowing, `Result`/`?`, lifetimes, traits, `async`/tokio; in Swift: optionals, `@Observable`, `@MainActor`, continuations, SwiftUI bindings; in TypeScript: unions, `declare module`, `async`), prefixed with "Rust note:", "Swift note:" or "TypeScript note:" when it is a general language point;
- point to the relevant spec file when the code implements a rule from it.

## Engineering priorities
- Pay particular attention to speed and robustness everywhere: the data model, the processing in the core, the interface between the core and its clients, and inside each client.
- Every feature must be extensively tested and documented.

## Documentation
- The application documentation lives in `docs/`, is written in English, and is produced together with the code: a change that adds or modifies a feature updates `docs/` in the same change.

## Testing the app
Every UI feature of the Swift app is tested twice, in the same change as the feature:
- a unit test in `apps/NiceMailer/Tests/NiceMailerTests` (Swift Testing, `make swift-test`): logic as pure functions where possible, keys and actions through `GeekController` against `FakeCore`;
- a scripted UI scenario in `apps/NiceMailer/UITests/*.uiscript` (`make ui-test`), which plays real keys and clicks in the app: add `.uiTarget(...)` to new clickable views and expose new state in `Runner.state()` (`UIScript.swift`).
Run `make test` and `make ui-test` before saying a UI change works, and look at the snapshots of the scenarios when the change is visual. Never test the app by sending events to a window the user may be using: UI script mode keeps the app in the background, off screen and away from the keyboard.

