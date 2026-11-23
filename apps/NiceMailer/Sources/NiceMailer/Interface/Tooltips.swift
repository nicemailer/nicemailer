import Foundation

/// How soon the tooltips of the app show (`.help` in SwiftUI, which are
/// AppKit's tooltips). AppKit waits about a second and a half by default,
/// too long for the buttons of the reply, whose icons and keys the tooltip
/// explains: one looks at a button, and the sentence should be there.
///
/// The delay is AppKit's preference `NSInitialToolTipDelay`, in
/// milliseconds. It is registered as a default (`UserDefaults.register`),
/// not written: a value the user set with `defaults write` wins, and
/// nothing is stored in the app's preferences file.
enum Tooltips {
    /// The preference AppKit reads.
    static let delayKey = "NSInitialToolTipDelay"
    /// Milliseconds before a tooltip shows: soon enough to read a button
    /// at a glance, long enough not to flash under a pointer passing by.
    static let delay = 300

    /// Registers the delay in `defaults`, at launch, before any view asks
    /// for a tooltip.
    static func register(in defaults: UserDefaults = .standard) {
        defaults.register(defaults: [delayKey: delay])
    }
}
