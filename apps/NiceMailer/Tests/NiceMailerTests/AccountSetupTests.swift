import Foundation
import Testing
@testable import NiceMailer

/// Adding an account from the Preferences (`AccountSetup`,
/// `docs/accounts.md`, "In the app"): three fields, then only what the
/// attempt shows is needed, against a fake core.
@MainActor
struct AccountSetupTests {
    // MARK: A small scripted core

    /// The answers of the creation methods, changed by each test.
    final class Script {
        var discovery: [String: Any] = AccountSetupTests.discovery()
        var test: Any = AccountSetupTests.testReport()
        /// The answer of `test_smtp`, from its parameters.
        var smtp: ([String: Any]) -> Any = { p in
            AccountSetupTests.smtpReport(p, outcomes: ["logged_in"])
        }
    }

    let script = Script()
    let core: FakeCore
    let setup: AccountSetup

    init() {
        let script = self.script
        core = FakeCore { method, params in
            switch method {
            case "discover_account":
                var d = script.discovery
                // A server given: used, with the guesses after it.
                if let given = params["imap"] as? [String: Any] {
                    d["imap"] = ["server": given, "origin": "given"]
                    d["smtp"] = [["server": ["host": "smtp.given.fr", "port": 465, "security": "tls"], "origin": "smtp_name"]]
                }
                return d
            case "test_account": return script.test
            case "test_smtp": return script.smtp(params)
            case "add_account": return ["account_id": 7, "created": [String](), "roles": [String: String](), "correspondents": 3, "archive_queued": false]
            default: return Fixture.answer(method, params)
            }
        }
        setup = AccountSetup(client: core)
    }

    nonisolated static func server(_ host: String, _ port: Int = 993, _ security: String = "tls") -> [String: Any] {
        ["host": host, "port": port, "security": security]
    }

    /// What `discover_account` says of a domain listed by a provider.
    nonisolated static func discovery(imap: Any = ["server": server("imap.toto.fr"), "origin": "provider"],
                          smtp: [[String: Any]] = [["server": server("smtp.toto.fr", 465), "origin": "provider"]],
                          mx: [String] = [], mxLookedUp: Bool = false, mxError: Any = NSNull(),
                          provider: Any = ["id": "toto", "name": "Toto", "source": "local", "matched": "toto.fr", "mx_host": NSNull(), "imap_unsupported": false]) -> [String: Any] {
        ["key": "toto", "key_source": "domain", "domain_key": "toto", "domain_account": NSNull(), "domain": "toto.fr",
         "by_domain": provider, "mx": mx, "mx_looked_up": mxLookedUp, "mx_error": mxError, "by_mx": NSNull(),
         "imap": imap, "smtp": smtp]
    }

    /// The folders of the server in `testReport`: Sent, Drafts and Archive;
    /// `Projets` has no role.
    nonisolated static let serverFolders = ["INBOX", "Archive", "Drafts", "Projets", "Sent"]

    /// The plan of `testReport`: the folders found, the others to create.
    nonisolated static func plan() -> [[String: Any]] {
        let found = ["inbox": "INBOX", "archive": "Archive", "drafts": "Drafts", "sent": "Sent"]
        return ["inbox", "todo", "waiting", "archive", "sent", "drafts", "newsletter", "paperwork", "quarantine", "deleted", "junk", "banned"].map { r in
            ["role": r, "path": found[r] ?? Role.name(r), "exists": found[r] != nil]
        }
    }

    nonisolated static func testReport(offerArchive: Bool = false) -> [String: Any] {
        ["probe": ["folders": serverFolders.map { ["name": $0, "selectable": true] }] as [String: Any], "roles": plan(),
         "inbox_messages": offerArchive ? 18052 : 40, "inbox_unseen": offerArchive ? 3921 : 2,
         "inbox_uidnext": 777, "correspondents": 284, "offer_archive": offerArchive,
         "read_before": "2026-11-17", "unread_before": "2026-10-18",
         "archive_read": offerArchive ? 14100 : 0, "archive_unread": offerArchive ? 3860 : 0]
    }

    /// A `test_smtp` report: one outcome per candidate, in order.
    nonisolated static func smtpReport(_ p: [String: Any], outcomes: [String]) -> [String: Any] {
        let candidates = p["candidates"] as? [[String: Any]] ?? []
        var attempts: [[String: Any]] = []
        var chosen: Any = NSNull()
        for (c, o) in zip(candidates, outcomes) {
            let details: [String: Any] = ["auth_refused": "535 bad", "failed": "cannot connect"]
            let detail = details[o] ?? NSNull()
            attempts.append(["candidate": c, "outcome": o, "detail": detail])
            if o == "logged_in" || o == "reachable" { chosen = attempts.count - 1; break }
        }
        let refused: Any = chosen is NSNull ? (attempts.firstIndex { $0["outcome"] as? String == "auth_refused" } ?? NSNull()) as Any : NSNull()
        return ["attempts": attempts, "chosen": chosen, "refused": refused]
    }

    func fill(_ email: String = "pierre@toto.fr", _ password: String = "secret", name: String = "Pierre Dupont") {
        setup.email = email
        setup.password = password
        setup.displayName = name
    }

    func methods() -> [String] { core.calls.map(\.method) }

    /// The last step: "On the server" is shown, nothing added yet; then
    /// confirmed as it is.
    func confirmPlan() async {
        #expect(setup.plan != nil, "the plan is shown before anything is added")
        #expect(core.calls("add_account").isEmpty)
        await setup.submit()
    }

    // MARK: The name of "From"

    static func account(_ id: Int, _ position: Int, _ name: String?) -> AccountInfo {
        AccountInfo(id: id, key: "a\(id)", kind: "imap", email: "a\(id)@x.fr", displayName: name, enabled: true, error: nil, position: position)
    }

    @Test func theNameOfFromIsTakenFromTheFirstAccountThatHasOne() {
        let accounts = [Self.account(1, 2, "Second"), Self.account(2, 0, nil), Self.account(3, 1, "  Pierre  ")]
        #expect(AccountSetup.proposedDisplayName(accounts) == "Pierre")
        #expect(AccountSetup.proposedDisplayName([Self.account(1, 0, " ")]) == "")
        #expect(AccountSetup.proposedDisplayName([]) == "")
    }

    @Test func aNameTypedIsNeverReplacedByTheProposal() {
        setup.proposeDisplayName([Self.account(1, 0, "Pierre")])
        #expect(setup.displayName == "Pierre")
        // A later proposal (the list read again) follows the accounts…
        setup.proposeDisplayName([Self.account(1, 0, "Pierre D.")])
        #expect(setup.displayName == "Pierre D.")
        // …until the user types their own.
        setup.displayName = "Moi"
        setup.proposeDisplayName([Self.account(1, 0, "Pierre")])
        #expect(setup.displayName == "Moi")
    }

    // MARK: Three fields

    @Test func threeFieldsAreEnoughWhenTheServersAreFound() async {
        var addedCalled = false
        setup.onAdded = { addedCalled = true }
        fill()
        await setup.submit()
        #expect(methods() == ["discover_account", "test_account", "test_smtp"])
        // Only "On the server" is shown, without a field to fill.
        #expect(setup.sectionsShown == [.server])
        #expect(setup.errorField == nil)
        await confirmPlan()
        #expect(methods() == ["discover_account", "test_account", "test_smtp", "add_account"])
        let add = core.calls("add_account")[0]
        #expect(add["key"] as? String == "toto")
        #expect(add["email"] as? String == "pierre@toto.fr")
        #expect(add["display_name"] as? String == "Pierre Dupont")
        #expect(add["password"] as? String == "secret")
        #expect((add["imap"] as? [String: Any])?["host"] as? String == "imap.toto.fr")
        #expect((add["smtp"] as? [String: Any])?["host"] as? String == "smtp.toto.fr")
        #expect(add["smtp_password"] == nil)
        #expect(add["uidnext"] as? Int == 777)
        #expect(add["correspondents"] as? Bool == true)
        #expect(add["archive_read_before"] == nil && add["archive_unread_before"] == nil)
        #expect(add["read_only"] == nil)
        // The plan as proposed: every role, the folders to create under
        // their English names.
        let roles = add["roles"] as? [String: String]
        #expect(roles?.count == 11 && roles?["sent"] == "Sent" && roles?["quarantine"] == "Quarantine" && roles?["inbox"] == nil)
        // Nothing else was asked; the form is empty again, the name kept.
        #expect(setup.imapServer == nil && setup.smtpServer == nil && setup.smtpPassword == nil && setup.plan == nil)
        #expect(setup.added?.contains("pierre@toto.fr") == true)
        #expect(setup.email.isEmpty && setup.password.isEmpty && setup.displayName == "Pierre Dupont")
        #expect(addedCalled)
    }

    @Test func anEmptyNameIsNotSent() async {
        fill(name: "  ")
        await setup.submit()
        await confirmPlan()
        #expect(core.calls("add_account").first?["display_name"] == nil)
    }

    @Test func anAddressOrAPasswordMissingIsSaidWithoutAskingTheCore() async {
        fill("pierre", "secret")
        await setup.submit()
        #expect(setup.errorField == .email)
        fill("pierre@toto.fr", "")
        await setup.submit()
        #expect(setup.errorField == .password)
        #expect(core.calls.isEmpty)
    }

    // MARK: The IMAP server

    /// `hosted.fr` is not listed and its MX is not either: the IMAP server
    /// is asked, with why, and nothing is tried before it is given.
    @Test func theIMAPServerIsAskedWhenTheMXDoesNotTellIt() async {
        script.discovery = Self.discovery(imap: NSNull(), smtp: [], mx: ["mx.hosting.example"], mxLookedUp: true, provider: NSNull())
        fill("pierre@toto.fr")
        await setup.submit()
        #expect(methods() == ["discover_account"])
        #expect(setup.imapServer?.host == "imap.toto.fr")
        #expect(setup.imapServer?.port == "993")
        #expect(setup.imapReason?.contains("mx.hosting.example") == true)
        #expect(setup.errorField == .imapHost)

        // Given: looked for again with it, then tried; the SMTP guesses
        // made for it are tried.
        setup.imapServer?.host = "mail.hosting.example"
        await setup.submit()
        await confirmPlan()
        #expect(methods() == ["discover_account", "discover_account", "test_account", "test_smtp", "add_account"])
        #expect((core.calls("discover_account")[1]["imap"] as? [String: Any])?["host"] as? String == "mail.hosting.example")
        #expect((core.calls("add_account")[0]["imap"] as? [String: Any])?["host"] as? String == "mail.hosting.example")
        #expect((core.calls("add_account")[0]["smtp"] as? [String: Any])?["host"] as? String == "smtp.given.fr")
    }

    @Test func theReasonSaysWhyTheServerIsNotKnown() throws {
        func report(_ d: [String: Any]) throws -> DiscoveryReport {
            try CoreClient.decoder.decode(DiscoveryReport.self, from: JSONSerialization.data(withJSONObject: d))
        }
        let oauth = try report(Self.discovery(provider: ["id": "o", "name": "Outlook", "source": "thunderbird", "matched": "x", "mx_host": NSNull(), "imap_unsupported": true]))
        #expect(AccountSetup.whyNoIMAP(oauth).contains("OAuth2"))
        let dns = try report(Self.discovery(imap: NSNull(), mxLookedUp: true, mxError: "timed out", provider: NSNull()))
        #expect(AccountSetup.whyNoIMAP(dns).contains("timed out"))
        let none = try report(Self.discovery(imap: NSNull(), mxLookedUp: true, provider: NSNull()))
        #expect(AccountSetup.whyNoIMAP(none) == L("%@ has no mail server (no MX record). Give the IMAP server.", "toto.fr"))
    }

    /// A provider that takes only OAuth2: its server is not tried.
    @Test func anOAuthOnlyProviderAsksForAnotherServer() async {
        script.discovery = Self.discovery(provider: ["id": "o", "name": "Outlook", "source": "thunderbird", "matched": "x", "mx_host": NSNull(), "imap_unsupported": true])
        fill()
        await setup.submit()
        #expect(methods() == ["discover_account"])
        #expect(setup.imapServer != nil)
    }

    @Test func aRefusedPasswordPointsAtThePasswordAndOnlyItIsTriedAgain() async {
        script.test = CoreClient.RPCError(code: 1009, message: "the server refused the login: bad",
                                          data: ["reason": "imap_auth_refused", "server_text": "bad"])
        fill()
        await setup.submit()
        #expect(setup.errorField == .password)
        // The server's words alone: Toto asks for no app password.
        #expect(setup.error == L("%@ refused the password: %@", "imap.toto.fr", "bad"))
        #expect(setup.imapServer == nil)

        script.test = Self.testReport()
        setup.password = "right"
        await setup.submit()
        await confirmPlan()
        // The servers are not looked for again.
        #expect(methods() == ["discover_account", "test_account", "test_account", "test_smtp", "add_account"])
    }

    @Test func anUnreachableServerShowsTheServerFound() async {
        script.test = CoreClient.RPCError(code: 1009, message: "the server could not be reached: cannot connect to imap.toto.fr:993",
                                          data: ["reason": "imap_unreachable", "server_text": "cannot connect to imap.toto.fr:993"])
        fill()
        await setup.submit()
        #expect(setup.imapServer == ServerFields(host: "imap.toto.fr", port: 993, security: "tls"))
        #expect(setup.imapReason?.contains("cannot connect") == true)
        #expect(setup.errorField == .imapHost)
    }

    @Test func aWrongPortIsRefusedBeforeAnything() async {
        script.discovery = Self.discovery(imap: NSNull(), provider: NSNull())
        fill()
        await setup.submit()
        setup.imapServer?.port = "99999"
        await setup.submit()
        #expect(setup.errorField == .imapPort)
        #expect(methods() == ["discover_account"])
    }

    // MARK: The SMTP server

    /// `smtp.toto.fr` refuses the IMAP password: its own is asked, then
    /// tried on it alone, and saved with the account.
    @Test func anSMTPServerRefusingTheIMAPPasswordGetsItsOwn() async {
        script.smtp = { p in
            Self.smtpReport(p, outcomes: [p["smtp_password"] as? String == "app-pw" ? "logged_in" : "auth_refused"])
        }
        fill()
        await setup.submit()
        #expect(setup.smtpPassword == "")
        #expect(setup.smtpPasswordReason?.contains("smtp.toto.fr") == true)
        #expect(setup.smtpPasswordReason?.contains("535 bad") == true)
        #expect(setup.errorField == .smtpPassword)
        #expect(core.calls("add_account").isEmpty)

        // Empty: said, nothing tried.
        await setup.submit()
        #expect(setup.errorField == .smtpPassword)
        #expect(core.calls("test_smtp").count == 1)

        // Wrong: refused again, cleared.
        setup.smtpPassword = "nope"
        await setup.submit()
        #expect(setup.smtpPasswordReason == L("%@ refused this password too (%@).", "smtp.toto.fr", "535 bad"))
        #expect(setup.smtpPassword == "")

        setup.smtpPassword = "app-pw"
        await setup.submit()
        await confirmPlan()
        let last = core.calls("test_smtp").last!
        #expect(last["smtp_password"] as? String == "app-pw")
        #expect((last["candidates"] as? [[String: Any]])?.count == 1)
        #expect(core.calls("test_account").count == 1)
        let add = core.calls("add_account")[0]
        #expect(add["smtp_password"] as? String == "app-pw")
        #expect((add["smtp"] as? [String: Any])?["host"] as? String == "smtp.toto.fr")
    }

    /// Another organization: the IMAP password is never sent there, its
    /// own is asked.
    @Test func anSMTPServerOfAnotherOrganizationGetsItsOwnPassword() async {
        script.smtp = { p in Self.smtpReport(p, outcomes: [p["smtp_password"] == nil ? "reachable" : "logged_in"]) }
        fill()
        await setup.submit()
        #expect(setup.smtpPassword == "")
        #expect(setup.smtpPasswordReason == AccountSetup.smtpPasswordMessage(.otherProvider, server: "smtp.toto.fr", imap: "imap.toto.fr"))
        setup.smtpPassword = "relay-pw"
        await setup.submit()
        await confirmPlan()
        #expect(core.calls("add_account").first?["smtp_password"] as? String == "relay-pw")
    }

    /// No SMTP password at hand: the account is added without a server
    /// to send from, its password to save later.
    @Test func theSMTPPasswordCanBeLeftForLater() async {
        script.smtp = { p in Self.smtpReport(p, outcomes: ["auth_refused"]) }
        fill()
        await setup.submit()
        #expect(setup.smtpPassword == "")
        setup.sendLater = true
        await setup.submit()
        await confirmPlan()
        #expect(core.calls("test_smtp").count == 1)
        let add = core.calls("add_account")[0]
        #expect(add["smtp"] == nil && add["smtp_password"] == nil)
    }

    @Test func noSMTPServerAsksForOneOrToAddWithoutSending() async {
        script.discovery = Self.discovery(smtp: [])
        fill()
        await setup.submit()
        #expect(setup.smtpServer == ServerFields(host: "smtp.toto.fr", port: 465, security: "tls"))
        #expect(setup.errorField == .smtpHost)
        #expect(core.calls("test_smtp").isEmpty)

        setup.sendLater = true
        await setup.submit()
        await confirmPlan()
        let add = core.calls("add_account")[0]
        #expect(add["smtp"] == nil && add["smtp_password"] == nil)
    }

    @Test func anSMTPServerThatDoesNotAnswerAsksForAnother() async {
        script.smtp = { p in
            let host = ((p["candidates"] as? [[String: Any]])?.first?["server"] as? [String: Any])?["host"] as? String
            return Self.smtpReport(p, outcomes: [host == "smtp.other.fr" ? "logged_in" : "failed"])
        }
        fill()
        await setup.submit()
        #expect(setup.smtpServer?.host == "smtp.toto.fr")
        #expect(setup.smtpReason?.contains("cannot connect") == true)
        setup.smtpServer?.host = "smtp.other.fr"
        await setup.submit()
        await confirmPlan()
        let tried = core.calls("test_smtp").last!["candidates"] as? [[String: Any]]
        #expect(tried?.first?["origin"] as? String == "given")
        #expect((core.calls("add_account")[0]["smtp"] as? [String: Any])?["host"] as? String == "smtp.other.fr")
    }

    @Test func eachSMTPReportIsReadAsTheFlowNeeds() {
        let c = ServerCandidate(server: ServerParams(host: "s", port: 465, security: "tls", username: nil), origin: "provider")
        func r(_ outcome: String, chosen: Int?, refused: Int?) -> SmtpTestReport {
            SmtpTestReport(attempts: [SmtpAttempt(candidate: c, outcome: outcome, detail: "d")], chosen: chosen, refused: refused)
        }
        #expect(AccountSetup.smtpVerdict(r("logged_in", chosen: 0, refused: nil), smtpPasswordGiven: false) == .use(c))
        #expect(AccountSetup.smtpVerdict(r("reachable", chosen: 0, refused: nil), smtpPasswordGiven: false) == .needsPassword(c, .otherProvider))
        #expect(AccountSetup.smtpVerdict(r("auth_refused", chosen: nil, refused: 0), smtpPasswordGiven: false) == .needsPassword(c, .refused("d")))
        #expect(AccountSetup.smtpVerdict(r("auth_refused", chosen: nil, refused: 0), smtpPasswordGiven: true) == .needsPassword(c, .refusedAgain("d")))
        #expect(AccountSetup.smtpVerdict(r("failed", chosen: nil, refused: nil), smtpPasswordGiven: false) == .noServer("s:465: d"))
    }

    // MARK: The import screen

    @Test func aBigInboxShowsTheImportScreenOnce() async {
        script.test = Self.testReport(offerArchive: true)
        fill()
        await setup.submit()
        #expect(setup.importShown)
        #expect(setup.sectionsShown == [.server, .importScreen])
        #expect(core.calls("add_account").isEmpty)
        #expect(setup.plan?.importChoices.archived(setup.test!) == 17960)

        // The unread kept, the correspondents not accepted.
        setup.plan?.importChoices.archiveUnread = false
        setup.plan?.importChoices.acceptCorrespondents = false
        #expect(setup.plan?.importChoices.archived(setup.test!) == 14100)
        await setup.submit()
        let add = core.calls("add_account")[0]
        #expect(add["archive_read_before"] as? String == "2026-11-17")
        #expect(add["archive_unread_before"] == nil)
        #expect(add["correspondents"] as? Bool == false)
        #expect(setup.added?.contains(String(format: "%lld", locale: uiLocale, 14100)) == true)
        // Nothing tried twice.
        #expect(methods() == ["discover_account", "test_account", "test_smtp", "add_account"])
    }

    // MARK: On the server

    func report(_ offerArchive: Bool = false) throws -> AccountTestReport {
        try CoreClient.decoder.decode(AccountTestReport.self, from: JSONSerialization.data(withJSONObject: Self.testReport(offerArchive: offerArchive)))
    }

    @Test func thePlanSaysWhichFoldersAreFoundAndWhichAreCreated() throws {
        var plan = ServerPlan(try report())
        #expect(plan.roles.map(\.role) == ["todo", "waiting", "archive", "sent", "drafts", "newsletter", "paperwork", "quarantine", "deleted", "junk", "banned"])
        #expect(plan.created == ["Todo", "Waiting", "Newsletter", "Paperwork", "Quarantine", "Deleted", "Junk", "Banned"])
        // A role found offers the folders of the server, the Inbox
        // excepted; a role to create offers its new folder too.
        let sent = plan.roles.first { $0.role == "sent" }!
        #expect(plan.options(sent) == ["Archive", "Drafts", "Projets", "Sent"])
        let todo = plan.roles.first { $0.role == "todo" }!
        #expect(plan.options(todo) == ["Archive", "Drafts", "Projets", "Sent", "Todo"])
        // An existing folder chosen: one fewer created.
        plan.roles[0].path = "Projets"
        #expect(plan.created.first == "Waiting")
        #expect((plan.params(try report())["roles"] as? [String: String])?["todo"] == "Projets")
        #expect(plan.problem == nil)
    }

    @Test func aFolderForTwoRolesIsRefusedBeforeAnything() async throws {
        var plan = ServerPlan(try report())
        plan.roles[0].path = "Archive"
        #expect(plan.sharedFolder?.path == "Archive")
        #expect(plan.sharedFolder?.roles == ["todo", "archive"])
        #expect(plan.problem == L("“%@” is chosen for several roles (%@): choose one folder per role.", "Archive", "Todo, Archive"))
        // Looking only, the folders do not matter.
        plan.lookOnly = true
        #expect(plan.problem == nil)

        fill()
        await setup.submit()
        setup.plan?.roles[0].path = "Archive"
        await setup.submit()
        #expect(setup.error?.contains("Archive") == true)
        #expect(core.calls("add_account").isEmpty)
    }

    /// Only looking: `read_only`, no folder, no archiving, even for a big
    /// Inbox; the import screen is not shown.
    @Test func onlyLookingChangesNothingOnTheServer() async {
        script.test = Self.testReport(offerArchive: true)
        fill()
        await setup.submit()
        #expect(setup.importShown)
        setup.plan?.lookOnly = true
        #expect(!setup.importShown)
        #expect(setup.plan?.created == [])
        #expect(setup.sectionsShown == [.server])
        await setup.submit()
        let add = core.calls("add_account")[0]
        #expect(add["read_only"] as? Bool == true)
        #expect(add["roles"] == nil && add["archive_read_before"] == nil && add["archive_unread_before"] == nil)
        #expect(add["uidnext"] as? Int == 777)
        #expect(setup.added == L("%@ added: NiceMailer shows its mail and changes nothing on the server. Organize it from its page when you are ready.", "pierre@toto.fr"))
    }

    /// A new password is a new test, so a new plan: shown again.
    @Test func changingThePasswordShowsThePlanAgain() async {
        fill()
        await setup.submit()
        #expect(setup.plan != nil)
        setup.password = "other"
        #expect(setup.plan == nil && setup.test == nil)
        await setup.submit()
        #expect(setup.plan != nil && core.calls("add_account").isEmpty)
        #expect(core.calls("test_account").count == 2)
    }

    // MARK: Starting again

    @Test func anotherAddressStartsAgainFromScratch() async {
        script.discovery = Self.discovery(imap: NSNull(), provider: NSNull())
        fill()
        await setup.submit()
        #expect(setup.imapServer != nil)
        setup.email = "pierre@other.fr"
        #expect(setup.imapServer == nil && setup.imapReason == nil && setup.discovery == nil)
        script.discovery = Self.discovery()
        await setup.submit()
        #expect(core.calls("discover_account").count == 2)
        #expect(core.calls("discover_account")[1]["imap"] == nil)
    }

    @Test func aRefusedCreationKeepsTheForm() async {
        core.handler = { [script] method, params in
            if method == "add_account" { return CoreClient.RPCError(code: -32602, message: "accounts.toml already has an account toto") }
            if method == "discover_account" { return script.discovery }
            if method == "test_account" { return script.test }
            if method == "test_smtp" { return script.smtp(params) }
            return Fixture.answer(method, params)
        }
        fill()
        await setup.submit()
        await confirmPlan()
        #expect(setup.error?.contains("already has an account") == true)
        #expect(setup.plan != nil, "the plan kept, to try again")
        #expect(setup.email == "pierre@toto.fr" && setup.added == nil)
    }
}

/// The accounts of the Preferences and their order.
@MainActor
struct AccountsListTests {
    nonisolated static func row(_ id: Int, _ key: String, _ position: Int) -> [String: Any] {
        ["id": id, "key": key, "kind": "imap", "email": "\(key)@x.fr", "display_name": NSNull(), "enabled": true, "error": NSNull(), "position": position]
    }

    @Test func aDragMovesTheRowsAsSwiftUIDoes() {
        let ids = [1, 2, 3, 4]
        // `onMove`'s destination is an index before the move.
        #expect(AccountsList.reordered(ids, fromOffsets: [0], toOffset: 4) == [2, 3, 4, 1])
        #expect(AccountsList.reordered(ids, fromOffsets: [3], toOffset: 0) == [4, 1, 2, 3])
        #expect(AccountsList.reordered(ids, fromOffsets: [1], toOffset: 3) == [1, 3, 2, 4])
        #expect(AccountsList.reordered(ids, fromOffsets: [0, 2], toOffset: 4) == [2, 4, 1, 3])
        #expect(AccountsList.reordered(ids, fromOffsets: [1], toOffset: 1) == ids)
    }

    @Test func aMoveSendsTheWholeOrderAndShowsItAtOnce() async {
        let core = FakeCore { method, params in
            method == "list_accounts" ? ["items": [Self.row(1, "perso", 0), Self.row(2, "work", 1), Self.row(3, "old", 2)], "as_of": Fixture.position]
                : ["changed": 0]
        }
        let list = AccountsList(client: core)
        await list.load()
        #expect(list.items.map(\.key) == ["perso", "work", "old"])
        #expect(!list.canMove(1, by: -1) && list.canMove(1, by: 1) && !list.canMove(3, by: 1))

        await list.move(3, by: -1)
        #expect(list.items.map(\.key) == ["perso", "old", "work"])
        #expect(core.calls("set_account_order").last?["account_ids"] as? [Int] == [1, 3, 2])

        await list.move(fromOffsets: [2], toOffset: 0)
        #expect(core.calls("set_account_order").last?["account_ids"] as? [Int] == [2, 1, 3])

        // At the top: nothing sent.
        await list.move(2, by: -1)
        #expect(core.calls("set_account_order").count == 2)
    }

    @Test func aRefusedMoveShowsTheCoresOrderAgain() async {
        let core = FakeCore { method, _ -> Any in
            if method == "list_accounts" { return ["items": [Self.row(1, "perso", 0), Self.row(2, "work", 1)], "as_of": Fixture.position] }
            return CoreClient.RPCError(code: 1001, message: "not found: account 9", data: ["reason": "not_found", "detail": "account 9"])
        }
        let list = AccountsList(client: core)
        await list.load()
        await list.move(2, by: -1)
        #expect(list.items.map(\.key) == ["perso", "work"])
        #expect(list.error == L("It no longer exists: it may have been deleted meanwhile"))
    }
}

/// Organizing an account only looked at (`AccountActivation`,
/// `docs/accounts.md`, "Looking first").
@MainActor
struct AccountActivationTests {
    @Test func thePlanIsReadFromTheCoreThenApplied() async {
        var reloaded = false
        let core = FakeCore { method, params in
            switch method {
            case "prepare_activation": return AccountSetupTests.testReport(offerArchive: true)
            case "activate_account": return ["created": ["Todo"], "roles": [String: String](), "correspondents": 0, "archive_queued": true]
            default: return Fixture.answer(method, params)
            }
        }
        let a = AccountActivation(client: core, key: "look", email: "pierre@toto.fr")
        a.onDone = { reloaded = true }
        await a.prepare()
        #expect(core.calls("prepare_activation").first?["account"] as? String == "look")
        #expect(a.plan?.created.count == 8 && a.importShown)
        // The plan's choices: a folder changed, the unread mail kept.
        a.plan?.roles[0].path = "Projets"
        a.plan?.importChoices.archiveUnread = false
        await a.activate()
        let p = core.calls("activate_account")[0]
        #expect(p["account"] as? String == "look")
        #expect((p["roles"] as? [String: String])?["todo"] == "Projets")
        #expect(p["archive_read_before"] as? String == "2026-11-17" && p["archive_unread_before"] == nil)
        #expect(p["read_only"] == nil && p["uidnext"] as? Int == 777)
        #expect(a.plan == nil && a.error == nil)
        #expect(a.done == L("%@ is organized: %lld old mails are being archived on the server.", "pierre@toto.fr", 14100))
        #expect(reloaded)
    }

    @Test func aRefusalKeepsThePlanAndSaysWhy() async {
        let core = FakeCore { method, params in
            switch method {
            case "prepare_activation": return AccountSetupTests.testReport()
            case "activate_account":
                return CoreClient.RPCError(code: 1009, message: "the server failed: refused CREATE",
                                           data: ["reason": "imap_failed", "server_text": "refused CREATE"])
            default: return Fixture.answer(method, params)
            }
        }
        let a = AccountActivation(client: core, key: "look", email: "pierre@toto.fr")
        await a.prepare()
        #expect(!a.importShown)
        await a.activate()
        #expect(a.error == L("The server failed: %@", "refused CREATE") && a.plan != nil && a.done == nil)
        // Cancelled: back to the page, nothing kept.
        a.cancel()
        #expect(a.plan == nil && a.error == nil)
    }

    @Test func aServerThatCannotBeReachedIsSaid() async {
        let core = FakeCore { method, params in
            method == "prepare_activation"
                ? CoreClient.RPCError(code: 1009, message: "the server could not be reached: cannot connect",
                                      data: ["reason": "imap_unreachable", "server_text": "cannot connect"])
                : Fixture.answer(method, params)
        }
        let a = AccountActivation(client: core, key: "look", email: "pierre@toto.fr")
        await a.prepare()
        #expect(a.error == L("The server could not be reached: %@", "cannot connect") && a.plan == nil)
    }

    @Test func anAccountOnlyLookedAtSaysSo() throws {
        let row: [String: Any] = ["id": 1, "key": "look", "kind": "imap", "email": "x@y.fr", "display_name": NSNull(), "enabled": true,
                                  "error": NSNull(), "position": 0, "read_only": true]
        let a = try CoreClient.decoder.decode(AccountInfo.self, from: JSONSerialization.data(withJSONObject: row))
        #expect(a.isReadOnly)
        var old = row
        old["read_only"] = nil
        #expect(try !CoreClient.decoder.decode(AccountInfo.self, from: JSONSerialization.data(withJSONObject: old)).isReadOnly)
    }
}
