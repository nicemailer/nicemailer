import Foundation
import Testing
@testable import NiceMailer

/// The search of the confirmation macOS shows before a call
/// (`CallConfirmation`), on fake accessibility trees: which labels call,
/// which element is pressed, and what is never pressed.
struct CallConfirmationTests {
    /// A fake element: an id, to tell old buttons from new ones.
    struct Node: AXTreeNode {
        var id: Int
        var info: AXNodeInfo
        var children: [Node] = []

        static func group(_ id: Int, _ children: [Node]) -> Node {
            Node(id: id, info: AXNodeInfo(role: "AXGroup", labels: [], actions: []), children: children)
        }
        static func button(_ id: Int, _ label: String) -> Node {
            Node(id: id, info: AXNodeInfo(role: "AXButton", labels: [label], actions: ["AXPress"]))
        }
    }

    /// A piece of macOS's `TelephonyUtilities.loctable`, as read from disk.
    let labels = CallConfirmation.labels(fromLoctable: [
        "en": ["DIAL_CALL_CARRIER_OSX": "Call", "DIAL_CALL_RELAY_OSX": "Call Using iPhone",
               "DIAL_CALL_TO_NUMBER_%@_RELAY_OSX": "Call %@ Using iPhone",
               "DIAL_CALL_CARRIER_IOS": "Call on iOS", "INCOMING_CALL": "Answer"],
        "fr": ["DIAL_CALL_CARRIER_OSX": "Appeler", "DIAL_CALL_RELAY_OSX": "Appeler depuis l’iPhone",
               "DIAL_CALL_TO_NUMBER_%@_RELAY_OSX": "Appeler le %@ depuis l’iPhone"],
    ])

    @Test func theLabelsAreThoseOfTheMacInEveryLanguage() {
        #expect(labels.exact == ["call", "call using iphone", "appeler", "appeler depuis l'iphone"])
        #expect(labels.patterns.count == 2)
        // Case, spaces and the typographic apostrophe do not count.
        #expect(labels.matches("Appeler"))
        #expect(labels.matches("  CALL "))
        #expect(labels.matches("Appeler depuis l'iPhone"))
        #expect(labels.matches("Appeler le 01 23 45 67 89 depuis l’iPhone"))
        #expect(labels.matches("Call +33 1 23 45 67 89 Using iPhone"))
    }

    @Test func onlyAWholeCallLabelMatches() {
        // "Call back" of a missed call, a part of a label, the iOS texts
        // and other keys are not the call of the Mac.
        #expect(!labels.matches("Rappeler"))
        #expect(!labels.matches("Call back"))
        #expect(!labels.matches("Call on iOS"))
        #expect(!labels.matches("Answer"))
        #expect(!labels.matches(""))
        // A pattern needs something between its prefix and its suffix.
        #expect(!labels.matches("Appeler le  depuis l’iPhone".replacingOccurrences(of: "  ", with: " ")))
    }

    @Test func theLabelsOfThisMacAreRead() {
        // The running macOS has its translations: at least the plain
        // label, whatever the language.
        #expect(!CallConfirmation.systemLabels.exact.isEmpty)
    }

    @Test func customActionsAreNamedLikeTheirButton() {
        #expect(CallConfirmation.actionName("Name:Appeler\nTarget:0x0\nSelector:(null)") == "Appeler")
        #expect(CallConfirmation.actionName("AXPress") == "AXPress")
        let banner = AXNodeInfo(role: "AXGroup", labels: ["FaceTime, Appeler le 01 23 45 67 89 ?"],
                                actions: ["AXPress", "Name:Fermer\nTarget:0x0", "Name:Appeler\nTarget:0x0"])
        // The banner's own label is a sentence, not the button's: pressing
        // the banner opens FaceTime, its "Appeler" action calls.
        #expect(CallConfirmation.callAction(banner, labels: labels) == "Name:Appeler\nTarget:0x0")
    }

    @Test func aButtonWithoutTextIsFoundByItsAccessibilityLabel() {
        // The green button shows a phone icon and no text: VoiceOver's
        // label (description) is what names it.
        let green = AXNodeInfo(role: "AXButton", labels: ["Appeler"], actions: ["AXPress"])
        #expect(CallConfirmation.callAction(green, labels: labels) == "AXPress")
        // Not pressable: nothing to do with it.
        let text = AXNodeInfo(role: "AXStaticText", labels: ["Appeler"], actions: [])
        #expect(CallConfirmation.callAction(text, labels: labels) == nil)
        let close = AXNodeInfo(role: "AXButton", labels: ["Fermer"], actions: ["AXPress"])
        #expect(CallConfirmation.callAction(close, labels: labels) == nil)
    }

    @Test func onlyANewButtonIsPressed() {
        // A call notification already on screen (1), and the one of this
        // call (3), deeper in another window.
        let roots = [
            Node.group(0, [.button(1, "Appeler"), .button(2, "Fermer")]),
            Node.group(10, [.group(11, [.button(3, "Appeler")])]),
        ]
        let old: Set<Int> = Set(CallConfirmation.candidates(in: roots, labels: labels).map(\.node.id))
        #expect(old == [1, 3])
        // Before the URL was opened, only 1 was there.
        let found = CallConfirmation.newButton(in: roots, labels: labels) { $0.id == 1 }
        #expect(found?.node.id == 3)
        #expect(found?.action == "AXPress")
        // Nothing new: nothing pressed.
        #expect(CallConfirmation.newButton(in: roots, labels: labels) { _ in true } == nil)
    }

    @Test func theSearchIsBounded() {
        // A deep chain: the button below the depth limit is not reached.
        var node = Node.button(99, "Call")
        for i in 0..<20 { node = .group(i, [node]) }
        #expect(CallConfirmation.candidates(in: [node], labels: labels, maxDepth: 10).isEmpty)
        #expect(CallConfirmation.candidates(in: [node], labels: labels, maxDepth: 25).count == 1)
        // A wide tree: no more than `maxNodes` elements are read.
        let wide = Node.group(0, (1...50).map { .button($0, "Call") })
        #expect(CallConfirmation.candidates(in: [wide], labels: labels, maxNodes: 11).count == 10)
    }

    @Test func theTreeIsDescribedForADiagnosis() {
        let roots = [Node.group(0, [.button(1, "Appeler")])]
        #expect(CallConfirmation.describe(roots) == "AXGroup  []\n  AXButton \"Appeler\" [AXPress]")
    }
}

/// `AppModel.openLink`: a `tel:` URL is confirmed only with
/// `ui.auto_confirm_calls`; other links are only opened.
@MainActor
struct OpenLinkTests {
    let model = AppModel()

    @Test func aCallIsOnlyOpenedByDefault() async {
        var opened: [String] = []
        var confirmed: [String] = []
        model.openFile = { opened.append($0.absoluteString) }
        model.confirmCall = { url, open in confirmed.append(url.absoluteString); open(); return .confirmed }
        model.openLink(URL(string: "tel:0612345678")!)
        #expect(opened == ["tel:0612345678"])
        #expect(confirmed.isEmpty)
    }

    @Test func aCallIsConfirmedWhenAsked() async {
        var opened: [String] = []
        var confirmed: [String] = []
        model.autoConfirmCalls = true
        model.openFile = { opened.append($0.absoluteString) }
        model.confirmCall = { url, open in confirmed.append(url.absoluteString); open(); return .confirmed }
        model.openLink(URL(string: "tel:0612345678")!)
        #expect(await eventually { model.flash == L("Calling…") })
        // Opened once, by the confirmation (which notes the buttons first).
        #expect(opened == ["tel:0612345678"])
        #expect(confirmed == ["tel:0612345678"])
        // A web link or a mail address is never "confirmed".
        model.openLink(URL(string: "https://example.org")!)
        model.openLink(URL(string: "mailto:a@example.org")!)
        #expect(opened == ["tel:0612345678", "https://example.org", "mailto:a@example.org"])
        #expect(confirmed == ["tel:0612345678"])
    }

    @Test func theStatusBarSaysWhatToDoWhenNotConfirmed() async {
        model.autoConfirmCalls = true
        model.openFile = { _ in }
        model.confirmCall = { _, open in open(); return .notTrusted }
        model.openLink(URL(string: "tel:0612345678")!)
        #expect(await eventually { model.flash == AppModel.callMessage(.notTrusted) })
        #expect(AppModel.callMessage(.notTrusted) == L("To confirm calls, allow NiceMailer in System Settings › Privacy & Security › Accessibility"))
        model.confirmCall = { _, open in open(); return .notFound }
        model.openLink(URL(string: "tel:0612345678")!)
        #expect(await eventually { model.flash == L("Call not confirmed: click the green button") })
    }

    @Test func theSettingComesFromTheCore() async {
        let core = FakeCore { method, params in
            method == "config_status" ? ["ui": ["title_bar": "hidden", "auto_confirm_calls": true]] : Fixture.answer(method, params)
        }
        await model.attach(core)
        await model.loadUISettings()
        #expect(model.autoConfirmCalls)
        // An older core, without the key: off.
        let older = FakeCore { method, params in
            method == "config_status" ? ["ui": ["title_bar": "hidden"]] : Fixture.answer(method, params)
        }
        let m2 = AppModel()
        m2.autoConfirmCalls = true
        await m2.attach(older)
        await m2.loadUISettings()
        #expect(!m2.autoConfirmCalls)
    }
}
