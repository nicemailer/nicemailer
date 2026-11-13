import AppKit
import ApplicationServices

/// Confirms a call for the user (`docs/geek-ui.md`, "Calling a number").
///
/// When the app opens a `tel:` URL, macOS (FaceTime, through the iPhone)
/// does not call at once: it shows a notification with a green button, a
/// phone icon and no text, which must be clicked. A notification never
/// takes the keyboard focus, so Return cannot press it. With
/// `ui.auto_confirm_calls = true` in `config.toml`, the app presses the
/// button itself through the accessibility API (the API VoiceOver uses to
/// read and drive the interface of any app).
///
/// How the button is recognized, without guessing its look:
/// - Its accessibility label is the one macOS gives the call action, read
///   from macOS's own translations (`TelephonyUtilities.loctable`,
///   `DIAL_CALL_*_OSX`: "Call", "Appeler", "Call Using iPhone"…, in every
///   language of the system), so it matches whatever the Mac's language.
///   The label must be the whole text, not a part of it: "Rappeler" (call
///   back, in a missed call notification) is not "Appeler".
/// - It must be **new**: the buttons already on screen before the URL was
///   opened are never pressed. Only the confirmation of this call can be.
/// - It is looked for during a few seconds only, in the processes that
///   show notifications and FaceTime's windows.
///
/// When nothing is found, nothing is pressed: the user clicks as before,
/// and the accessibility tree that was searched is written to
/// `call-confirmation.txt` in the data directory, to adapt the search to a
/// new version of macOS.
///
/// The matching is made of pure functions over `AXTreeNode`, tested on fake
/// trees (`CallConfirmationTests`); only `LiveNode` talks to the system.
enum CallConfirmation {
    /// What happened to a call the app was asked to confirm.
    enum Outcome: Equatable {
        /// The green button was pressed: the call starts.
        case confirmed
        /// The app may not use the accessibility API: the user must allow
        /// it in System Settings (macOS was asked to show its dialog).
        case notTrusted
        /// No confirmation button appeared in time: the user clicks it.
        case notFound
    }

    // MARK: Labels

    /// The labels of the call action, normalized (`normalize`): exact ones
    /// ("appeler") and patterns with a name or a number inside ("appeler
    /// le %@ depuis l'iphone", kept as a prefix and a suffix).
    struct Labels: Equatable {
        var exact: Set<String>
        var patterns: [Pattern]

        struct Pattern: Equatable {
            let prefix: String
            let suffix: String
        }

        /// The label is one of the call action's: equal to an exact one,
        /// or made of a pattern's prefix, something, then its suffix.
        func matches(_ label: String) -> Bool {
            let s = CallConfirmation.normalize(label)
            guard !s.isEmpty else { return false }
            if exact.contains(s) { return true }
            return patterns.contains { p in
                s.count > p.prefix.count + p.suffix.count && s.hasPrefix(p.prefix) && s.hasSuffix(p.suffix)
            }
        }
    }

    /// Lower case, trimmed, typographic apostrophes as plain ones: macOS's
    /// translations write "l’iPhone", an accessibility label may not.
    static func normalize(_ s: String) -> String {
        s.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\u{2019}", with: "'")
            .lowercased()
    }

    /// The labels of the call action in a `.loctable` of macOS: a
    /// dictionary of languages, each a dictionary of keys to texts. The
    /// keys `DIAL_CALL_…_OSX` are the texts macOS shows on the Mac for the
    /// button that calls ("Call", "Call %@ Using iPhone"…); every language
    /// is taken, since the button speaks the language of macOS, not the
    /// app's.
    static func labels(fromLoctable table: [String: Any]) -> Labels {
        var exact = Set<String>()
        var patterns: [Labels.Pattern] = []
        for case let (_, texts as [String: Any]) in table {
            for case let (key, text as String) in texts where key.hasPrefix("DIAL_CALL") && key.hasSuffix("_OSX") {
                let s = normalize(text)
                // Swift note: `range(of:)` returns an optional; `if let`
                // runs its block only when the text holds a `%@`.
                if let r = s.range(of: "%@") {
                    let p = Labels.Pattern(prefix: String(s[..<r.lowerBound]), suffix: String(s[r.upperBound...]))
                    if !patterns.contains(p) { patterns.append(p) }
                } else if !s.isEmpty {
                    exact.insert(s)
                }
            }
        }
        return Labels(exact: exact, patterns: patterns)
    }

    /// The labels of the running macOS, read once from its translations;
    /// English and French ones if the file cannot be read (a macOS that
    /// moved it).
    static let systemLabels: Labels = {
        let path = "/System/Library/PrivateFrameworks/TelephonyUtilities.framework/Resources/TelephonyUtilities.loctable"
        if let data = FileManager.default.contents(atPath: path),
           let table = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any] {
            let labels = labels(fromLoctable: table)
            if !labels.exact.isEmpty { return labels }
        }
        return labels(fromLoctable: [
            "en": ["DIAL_CALL_CARRIER_OSX": "Call", "DIAL_CALL_RELAY_OSX": "Call Using iPhone"],
            "fr": ["DIAL_CALL_CARRIER_OSX": "Appeler", "DIAL_CALL_RELAY_OSX": "Appeler depuis l’iPhone"],
        ])
    }()

    // MARK: Matching (pure)

    /// The name of an accessibility action. A plain action is its name
    /// (`AXPress`); a custom one, such as the buttons of a notification,
    /// is written `Name:Appeler\nTarget:0x0\nSelector:(null)`.
    static func actionName(_ raw: String) -> String {
        guard raw.hasPrefix("Name:") else { return raw }
        let rest = raw.dropFirst("Name:".count)
        return String(rest.prefix { $0 != "\n" })
    }

    /// The action that calls, on one element: `AXPress` on a button whose
    /// label is a call label, or a custom action named with one (how a
    /// notification exposes its buttons). Nil for anything else.
    static func callAction(_ info: AXNodeInfo, labels: Labels) -> String? {
        if info.actions.contains("AXPress"), info.labels.contains(where: labels.matches) {
            return "AXPress"
        }
        return info.actions.first { labels.matches(actionName($0)) }
    }

    /// Every element of the trees with a call action, breadth first, with
    /// that action. Bounded (`maxDepth`, `maxNodes`): an accessibility tree
    /// may be deep or even cyclic, and the search runs every 100 ms.
    static func candidates<N: AXTreeNode>(in roots: [N], labels: Labels,
                                          maxDepth: Int = 14, maxNodes: Int = 4000) -> [(node: N, action: String)] {
        var found: [(node: N, action: String)] = []
        var level = roots
        var seen = 0
        var depth = 0
        while !level.isEmpty, depth <= maxDepth, seen < maxNodes {
            var next: [N] = []
            for node in level {
                seen += 1
                if seen > maxNodes { break }
                if let action = callAction(node.info, labels: labels) {
                    found.append((node, action))
                }
                next += node.children
            }
            level = next
            depth += 1
        }
        return found
    }

    /// The call button to press: the first candidate that was not on
    /// screen before the URL was opened (`isOld`).
    static func newButton<N: AXTreeNode>(in roots: [N], labels: Labels,
                                         isOld: (N) -> Bool) -> (node: N, action: String)? {
        candidates(in: roots, labels: labels).first { !isOld($0.node) }
    }

    /// The trees as text, one element per line, indented by depth: role,
    /// labels, actions. Written to `call-confirmation.txt` when no button
    /// was found, to see what macOS showed.
    static func describe<N: AXTreeNode>(_ roots: [N], maxDepth: Int = 14, maxNodes: Int = 4000) -> String {
        var lines: [String] = []
        // Swift note: a nested function may call itself; `inout` lets it
        // change the counter of its caller.
        func walk(_ node: N, _ depth: Int, _ count: inout Int) {
            guard depth <= maxDepth, count < maxNodes else { return }
            count += 1
            let i = node.info
            let labels = i.labels.map { "\"\($0)\"" }.joined(separator: " ")
            let actions = i.actions.map { actionName($0) }.joined(separator: ", ")
            lines.append(String(repeating: "  ", count: depth) + "\(i.role) \(labels) [\(actions)]")
            for c in node.children { walk(c, depth + 1, &count) }
        }
        var count = 0
        for r in roots { walk(r, 0, &count) }
        return lines.joined(separator: "\n")
    }

    // MARK: Live

    /// The apps whose windows may hold the confirmation: the notifications
    /// (where macOS shows it), FaceTime, and the older alert process.
    static let bundleIds = ["com.apple.notificationcenterui", "com.apple.FaceTime", "com.apple.UserNotificationCenter"]

    /// The app may use the accessibility API. The first time it may not in
    /// this run, macOS is asked to show its dialog, which leads to System
    /// Settings › Privacy & Security › Accessibility.
    @MainActor static func isTrusted() -> Bool {
        if AXIsProcessTrusted() { return true }
        if !promptShown {
            promptShown = true
            // Swift note: the key is a C global (`Unmanaged<CFString>`);
            // `takeUnretainedValue()` reads it without taking ownership.
            let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
            _ = AXIsProcessTrustedWithOptions([key: true] as CFDictionary)
        }
        return false
    }
    @MainActor private static var promptShown = false

    /// Opens the `tel:` URL with `open`, then presses the confirmation that
    /// appears, within `timeout` seconds. The buttons on screen are noted
    /// before `open` runs, so that only the new one can be pressed.
    @MainActor static func confirm(_ url: URL, timeout: TimeInterval = 5, open: () -> Void) async -> Outcome {
        guard isTrusted() else {
            open()
            return .notTrusted
        }
        let labels = systemLabels
        // The accessibility calls wait for the other process to answer:
        // off the main thread, so the window never freezes.
        // Swift note: `Task.detached` runs its closure on a background
        // thread; `await … .value` waits for its result without blocking.
        let before = await Task.detached { LiveSearch.oldButtons(labels: labels) }.value
        open()
        let pressed = await Task.detached { LiveSearch.pressNew(labels: labels, old: before, timeout: timeout) }.value
        if pressed { return .confirmed }
        let tree = await Task.detached { describe(LiveSearch.roots()) }.value
        let file = DataDirectory.current.appendingPathComponent("call-confirmation.txt")
        try? "\(Date()) \(url.absoluteString)\n\(tree)\n".write(to: file, atomically: true, encoding: .utf8)
        return .notFound
    }
}

/// What the matching reads of an element of an accessibility tree.
struct AXNodeInfo: Equatable {
    /// `AXButton`, `AXGroup`…
    var role: String
    /// Its title, description, help, identifier and string value: where a
    /// button without text keeps the label VoiceOver reads.
    var labels: [String]
    /// The names of its actions (`AXPress`, `Name:…` for custom ones).
    var actions: [String]
}

/// An element of an accessibility tree: the system's (`LiveNode`) or a
/// fake one, in the tests.
///
/// Swift note: a protocol lists what a type must provide; `Self` is the
/// type that conforms, so the children of a fake node are fake nodes.
protocol AXTreeNode {
    var info: AXNodeInfo { get }
    var children: [Self] { get }
}

/// An element of another app, read through the accessibility API.
struct LiveNode: AXTreeNode {
    let element: AXUIElement

    var info: AXNodeInfo {
        let role = string(kAXRoleAttribute) ?? "?"
        let labels = [kAXTitleAttribute, kAXDescriptionAttribute, kAXHelpAttribute, kAXIdentifierAttribute, kAXValueAttribute]
            .compactMap { string($0) }.filter { !$0.isEmpty }
        var names: CFArray?
        let actions = AXUIElementCopyActionNames(element, &names) == .success ? (names as? [String] ?? []) : []
        return AXNodeInfo(role: role, labels: labels, actions: actions)
    }

    var children: [LiveNode] {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value) == .success,
              let list = value as? [AXUIElement] else { return [] }
        return list.map(LiveNode.init)
    }

    /// A text attribute, or nil (absent, or not a text: a number value).
    private func string(_ attribute: String) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else { return nil }
        return value as? String
    }
}

/// The search in the live system, off the main thread.
enum LiveSearch {
    /// The windows of the apps that may show the confirmation.
    static func roots() -> [LiveNode] {
        CallConfirmation.bundleIds.flatMap { id in
            NSRunningApplication.runningApplications(withBundleIdentifier: id).flatMap { app -> [LiveNode] in
                let element = AXUIElementCreateApplication(app.processIdentifier)
                // An app that hangs must not hold the search: half a second
                // per call at most.
                AXUIElementSetMessagingTimeout(element, 0.5)
                var value: CFTypeRef?
                if AXUIElementCopyAttributeValue(element, kAXWindowsAttribute as CFString, &value) == .success,
                   let windows = value as? [AXUIElement], !windows.isEmpty {
                    return windows.map(LiveNode.init)
                }
                return LiveNode(element: element).children
            }
        }
    }

    /// The call buttons on screen now: never pressed.
    static func oldButtons(labels: CallConfirmation.Labels) -> [AXUIElement] {
        CallConfirmation.candidates(in: roots(), labels: labels).map(\.node.element)
    }

    /// Looks for a new call button every 100 ms until `timeout`, and
    /// presses the first one found. True when the press succeeded.
    static func pressNew(labels: CallConfirmation.Labels, old: [AXUIElement], timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            // `CFEqual`: two references to the same element of the other
            // app are equal, though they are different objects here.
            if let (node, action) = CallConfirmation.newButton(in: roots(), labels: labels, isOld: { n in
                old.contains { CFEqual($0, n.element) }
            }) {
                return AXUIElementPerformAction(node.element, action as CFString) == .success
            }
            Thread.sleep(forTimeInterval: 0.1)
        } while Date() < deadline
        return false
    }
}
