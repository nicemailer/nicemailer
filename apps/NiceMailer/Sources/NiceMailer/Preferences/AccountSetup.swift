import Foundation

// Adding an IMAP account from the app (`docs/accounts.md`, "In the app").
//
// The screen asks for three things only: the address, the password and the
// name shown in "From". Everything else is found by the core
// (`discover_account`: the provider by the domain or by its MX, then the
// IMAP and SMTP servers) and asked only when the attempt shows it is
// needed:
//
// - the IMAP server, when the core finds none for the domain, or cannot
//   connect to the one it found;
// - the password of the SMTP server, when it refuses the IMAP password or
//   belongs to another organization (the IMAP password is never sent
//   there, `docs/sending.md`, "Password");
// - the SMTP server, when none answered;
// - the import choices (`import-initial.md`, section 3), when the Inbox is
//   above the thresholds and the core offers to archive its old mail.
//
// Once the servers work, one more step is always shown before anything is
// written: "On the server", what the account will change there. The folders
// of the workflow, those found and those to create, each one changeable
// (`dossiers.md`, "A la création du compte IMAP dans niceMailer"); or, the
// user's choice, nothing at all: the account is only looked at
// (`read_only`), and organized later from its page (`AccountActivation`,
// `docs/accounts.md`, "Looking first").
//
// This file holds the logic only, without any view, so that each step is
// tested against `FakeCore` (`Tests/NiceMailerTests/AccountSetupTests.swift`).
// The views are in `PreferencesView.swift`.

// MARK: - Mirrors of the contract (`crates/nm-api`)

/// A server of the contract (`ImapParams`, for IMAP and SMTP alike).
struct ServerParams: Decodable, Equatable {
    let host: String
    let port: Int
    let security: String
    let username: String?

    /// The JSON the core takes. Swift note: `[String: Any]` is a
    /// heterogeneous dictionary, what `CoreCalling.call` sends as JSON; an
    /// optional left `nil` is simply not put in it.
    var json: [String: Any] {
        var p: [String: Any] = ["host": host, "port": port, "security": security]
        if let username, !username.isEmpty { p["username"] = username }
        return p
    }
}

/// A server to use or to try, and where it comes from (`ServerCandidate`).
struct ServerCandidate: Decodable, Equatable {
    let server: ServerParams
    let origin: String

    var json: [String: Any] { ["server": server.json, "origin": origin] }
}

/// A provider of the core's database (`ProviderMatch`).
struct ProviderMatch: Decodable, Equatable {
    let name: String
    /// OAuth2 only: NiceMailer cannot log in there yet.
    let imapUnsupported: Bool
}

/// What `discover_account` found (`DiscoveryReport`).
struct DiscoveryReport: Decodable, Equatable {
    /// The account key proposed: never asked in the app.
    let key: String
    let domain: String
    let byDomain: ProviderMatch?
    let byMx: ProviderMatch?
    let mx: [String]
    let mxLookedUp: Bool
    let mxError: String?
    let imap: ServerCandidate?
    let smtp: [ServerCandidate]

    var provider: ProviderMatch? { byDomain ?? byMx }
}

/// A folder of the server (`ProbeFolder`).
struct ServerFolder: Decodable, Equatable {
    let name: String
    /// `false`: a node of the hierarchy that holds no mail.
    let selectable: Bool
}

/// The probe of the server (`ProbeReport`): only its folders are read here.
struct ServerProbe: Decodable, Equatable {
    let folders: [ServerFolder]
}

/// The folder a workflow role would get (`RolePlan`).
struct RolePlan: Decodable, Equatable {
    /// `inbox`, `todo`, `archive`…
    let role: String
    let path: String
    /// `false`: the server has no such folder, it would be created.
    let exists: Bool
}

/// The report of `test_account` (`TestAccountReport`), also the answer of
/// `prepare_activation`: the counts of the import screen, the folders of
/// the server and the plan of the roles.
struct AccountTestReport: Decodable, Equatable {
    let probe: ServerProbe
    /// In sidebar order, Inbox first.
    let roles: [RolePlan]
    let inboxMessages: Int
    let inboxUnseen: Int
    let inboxUidnext: Int
    let correspondents: Int
    let offerArchive: Bool
    let readBefore: String
    let unreadBefore: String
    let archiveRead: Int
    let archiveUnread: Int
}

/// One SMTP server tried (`SmtpAttempt`).
struct SmtpAttempt: Decodable, Equatable {
    let candidate: ServerCandidate
    /// `logged_in`, `auth_refused`, `reachable`, `no_login`, `failed`.
    let outcome: String
    let detail: String?
}

/// What `test_smtp` did (`SmtpTestReport`).
struct SmtpTestReport: Decodable, Equatable {
    let attempts: [SmtpAttempt]
    /// The server that works.
    let chosen: Int?
    /// None works, but this one refused the password: it needs its own.
    let refused: Int?
}

/// What `add_account` did (`AddAccountResult`).
struct AddAccountResult: Decodable, Equatable {
    let accountId: Int
    let correspondents: Int
    let archiveQueued: Bool
}

/// An account of `list_accounts` (`AccountView`).
struct AccountInfo: Decodable, Identifiable, Equatable {
    let id: Int
    let key: String
    /// `imap` or `archive`.
    let kind: String
    let email: String?
    let displayName: String?
    let enabled: Bool
    let error: String?
    let position: Int
    /// Only looked at (`read_only = true`): nothing is changed on its
    /// server until it is organized. Swift note: an optional property of a
    /// `Decodable` is `nil` when the key is missing, so older answers still
    /// decode.
    var readOnly: Bool? = nil

    var isReadOnly: Bool { readOnly == true }
}

struct AccountInfoList: Decodable {
    let items: [AccountInfo]
}

// MARK: - The fields the user may be asked for

/// A server typed by the user. The port is kept as typed (a `String`):
/// a field bound to a number would fight the user while they type.
struct ServerFields: Equatable {
    var host: String
    var port: String
    /// `tls` or `starttls`: `none` is for the loopback only, never offered.
    var security: String
    /// Empty: the address.
    var username: String = ""

    init(host: String, port: Int, security: String, username: String = "") {
        self.host = host
        self.port = String(port)
        self.security = security
        self.username = username
    }

    init(_ p: ServerParams) {
        self.init(host: p.host, port: p.port, security: p.security, username: p.username ?? "")
    }

    /// The server, or `nil` when the host is empty or the port is not a
    /// port number.
    var params: ServerParams? {
        let host = host.trimmingCharacters(in: .whitespaces)
        guard !host.isEmpty, let port = Int(port.trimmingCharacters(in: .whitespaces)), (1...65535).contains(port) else { return nil }
        let user = username.trimmingCharacters(in: .whitespaces)
        return ServerParams(host: host, port: port, security: security, username: user.isEmpty ? nil : user)
    }

    /// The guess shown when the user must give a server: `imap.` or
    /// `smtp.` before the domain, on the port of TLS.
    static func guess(_ prefix: String, domain: String, port: Int) -> ServerFields {
        ServerFields(host: domain.isEmpty ? "" : "\(prefix).\(domain)", port: port, security: "tls")
    }
}

/// The choices of the import screen (`import-initial.md`, section 3): the
/// defaults of the core, every box checked.
struct ImportChoices: Equatable {
    var archiveRead = true
    var archiveUnread = true
    var acceptCorrespondents = true

    /// Mails that would be archived, from the exact counts of
    /// `test_account` for the default dates. The two sets do not overlap
    /// (read, unread), so the sum is exact without counting again.
    func archived(_ r: AccountTestReport) -> Int {
        (archiveRead ? r.archiveRead : 0) + (archiveUnread ? r.archiveUnread : 0)
    }
}

/// What the account will change on the server, shown before anything is
/// done (`dossiers.md`, "A la création du compte IMAP dans niceMailer";
/// `import-initial.md`, section 3): the folder of each workflow role, those
/// to create, the import choices. Or nothing at all: `lookOnly`.
///
/// The same plan for a new account (`AccountSetup`) and for one only looked
/// at until now (`AccountActivation`).
struct ServerPlan: Equatable {
    /// The folder chosen for a role.
    struct RoleChoice: Equatable, Identifiable {
        let role: String
        var path: String
        /// What the core proposed: an existing folder, or a new one.
        let proposed: String
        var id: String { role }
    }

    /// Change nothing on the server: `read_only` (`docs/sync.md`,
    /// "Read-only accounts").
    var lookOnly = false
    /// Every role but the Inbox (always `INBOX`), in sidebar order.
    var roles: [RoleChoice]
    var importChoices = ImportChoices()
    /// The folders of the server that can hold mail, the Inbox excepted.
    let existing: [String]

    init(roles: [RoleChoice] = [], existing: [String] = []) {
        self.roles = roles
        self.existing = existing
    }

    init(_ r: AccountTestReport) {
        self.init(
            roles: r.roles.filter { $0.role != "inbox" }.map { RoleChoice(role: $0.role, path: $0.path, proposed: $0.path) },
            existing: r.probe.folders.filter { $0.selectable && $0.name.uppercased() != "INBOX" }.map(\.name))
    }

    /// The folders offered for a role: those of the server, and the new
    /// folder the core proposes when the server has none for it.
    func options(_ c: RoleChoice) -> [String] {
        isNew(c.proposed) ? existing + [c.proposed] : existing
    }

    func isNew(_ path: String) -> Bool { !existing.contains(path) }

    /// The folders that will be created on the server, in sidebar order:
    /// none when only looking.
    var created: [String] {
        guard !lookOnly else { return [] }
        var seen = Set<String>()
        // Swift note: `insert` returns whether the element was new, so this
        // keeps the first of each folder, in order.
        return roles.map(\.path).filter { isNew($0) && seen.insert($0).inserted }
    }

    /// A folder chosen for several roles, and those roles: the core keeps
    /// one role per folder, the last one would silently win.
    var sharedFolder: (path: String, roles: [String])? {
        for c in roles {
            let same = roles.filter { $0.path == c.path }
            if same.count > 1 { return (c.path, same.map(\.role)) }
        }
        return nil
    }

    /// The parameters of `add_account` or `activate_account` that come
    /// from the plan and the import screen. Looking only: none of them,
    /// but `read_only` (the people written to are still accepted: a
    /// decision of the database, which the dry screening follows).
    func params(_ t: AccountTestReport) -> [String: Any] {
        if lookOnly { return ["read_only": true, "uidnext": t.inboxUidnext, "correspondents": true] }
        var p: [String: Any] = [
            "roles": Dictionary(uniqueKeysWithValues: roles.map { ($0.role, $0.path) }),
            "uidnext": t.inboxUidnext,
            "correspondents": importChoices.acceptCorrespondents,
        ]
        if t.offerArchive && importChoices.archiveRead { p["archive_read_before"] = t.readBefore }
        if t.offerArchive && importChoices.archiveUnread { p["archive_unread_before"] = t.unreadBefore }
        return p
    }

    /// Mails the plan archives.
    func archived(_ t: AccountTestReport) -> Int {
        !lookOnly && t.offerArchive ? importChoices.archived(t) : 0
    }

    /// Why the plan cannot be applied, if it cannot.
    var problem: String? {
        guard !lookOnly, let shared = sharedFolder else { return nil }
        return L("“%@” is chosen for several roles (%@): choose one folder per role.", shared.path, shared.roles.map { Role.name($0) }.joined(separator: ", "))
    }
}

// MARK: - The flow

/// The account being added, and where its creation stands.
///
/// Each step's result is kept, so that pressing "Add Account" again after
/// the user filled the field asked for starts where it stopped: the servers
/// are not looked for again once found, the IMAP server not tried again
/// once it accepted the password. Changing a field forgets the results that
/// depend on it (`didSet` below).
///
/// Swift notes:
/// - `@Observable` makes SwiftUI redraw the views that read a property
///   when it changes; `didSet` runs after each assignment, as for any
///   stored property.
/// - `@MainActor`: every access happens on the main thread, the views'
///   thread; the `await` on the core suspends without blocking it.
@Observable
@MainActor
final class AccountSetup {
    /// A field the flow can point at (error, focus).
    enum Field: Hashable {
        case email, password, displayName, imapHost, imapPort, smtpHost, smtpPort, smtpPassword
    }

    var email = "" {
        didSet { if email != oldValue { forget(from: .discovery) } }
    }
    var password = "" {
        didSet { if password != oldValue { forget(from: .imapTest) } }
    }
    /// The name of "From": proposed from the accounts already set
    /// (`proposeDisplayName`), until the user types their own.
    var displayName = "" {
        didSet { if displayName != oldValue && !proposing { displayNameEdited = true } }
    }

    /// The IMAP server, shown only when it must be given (`imapReason`
    /// says why). `nil`: the one the core found.
    var imapServer: ServerFields? {
        didSet { if imapServer != oldValue { forget(from: .discovery, keepingIMAP: true) } }
    }
    private(set) var imapReason: String?

    /// The SMTP server, shown only when none was found or none answered.
    var smtpServer: ServerFields? {
        didSet { if smtpServer != oldValue { pendingSMTP = nil; forget(from: .smtp) } }
    }
    private(set) var smtpReason: String?
    /// Add the account without a server to send from, for now: offered
    /// with `smtpServer` (`docs/accounts.md`: its mails are then answered
    /// from another account).
    var sendLater = false {
        didSet { if sendLater != oldValue { forget(from: .smtp) } }
    }

    /// The password of the SMTP server, shown only when it needs its own
    /// (`smtpPasswordReason` says why). `nil`: the IMAP password.
    var smtpPassword: String? {
        didSet { if smtpPassword != oldValue { forget(from: .smtp) } }
    }
    private(set) var smtpPasswordReason: String?

    /// A section of the form shown only when needed.
    enum Section: Hashable {
        case imap, smtpPassword, smtp, server, importScreen
    }

    /// The sections shown, in the order of the form.
    var sectionsShown: [Section] {
        [imapServer != nil ? .imap : nil, smtpPassword != nil ? .smtpPassword : nil,
         smtpServer != nil ? .smtp : nil, plan != nil ? .server : nil, importShown ? .importScreen : nil].compactMap { $0 }
    }

    /// "On the server": shown once the servers work, before anything is
    /// written. `nil` until then.
    var plan: ServerPlan?

    /// The import screen, shown with the plan when the core offers to
    /// archive and the account is organized now.
    var importShown: Bool { plan.map { !$0.lookOnly } == true && test?.offerArchive == true }

    /// The step running, said under the button; `nil` when idle.
    private(set) var busy: String?
    private(set) var error: String?
    /// The field the error is about, or the one now asked for.
    private(set) var errorField: Field?
    /// The account just added, said until the next attempt.
    private(set) var added: String?

    private(set) var discovery: DiscoveryReport?
    private(set) var test: AccountTestReport?
    /// The SMTP server to write, once known.
    private var smtpChoice: ServerParams?
    /// The account is added without SMTP (`sendLater`), once decided.
    private var smtpSkipped = false
    /// The server that needs its own password, tried again with it.
    private var pendingSMTP: ServerCandidate?

    private var proposing = false
    private var displayNameEdited = false

    private let client: any CoreCalling
    /// Called once an account was added: the list reloads.
    var onAdded: (() async -> Void)?

    init(client: any CoreCalling) {
        self.client = client
    }

    /// The steps whose results are kept, in order.
    private enum Step: Int, Comparable {
        case discovery, imapTest, smtp
        static func < (a: Step, b: Step) -> Bool { a.rawValue < b.rawValue }
    }

    /// Forgets the results of `step` and of every step after it. A new
    /// address starts again from scratch, the extra fields hidden.
    private func forget(from step: Step, keepingIMAP: Bool = false) {
        if step <= .discovery {
            discovery = nil
            if !keepingIMAP {
                imapServer = nil
                imapReason = nil
                smtpServer = nil
                smtpReason = nil
                smtpPassword = nil
                smtpPasswordReason = nil
                pendingSMTP = nil
                sendLater = false
            }
        }
        // The plan is made from the test: a new test, a new plan.
        if step <= .imapTest {
            test = nil
            plan = nil
        }
        if step <= .smtp {
            smtpChoice = nil
            smtpSkipped = false
        }
        added = nil
    }

    /// Proposes the name of "From" of the accounts already set: the first
    /// one, in display order, that has a name (the user writes under one
    /// name, whatever the address). Never replaces a name the user typed.
    func proposeDisplayName(_ accounts: [AccountInfo]) {
        guard !displayNameEdited else { return }
        let name = Self.proposedDisplayName(accounts)
        proposing = true
        displayName = name
        proposing = false
    }

    nonisolated static func proposedDisplayName(_ accounts: [AccountInfo]) -> String {
        accounts.sorted { ($0.position, $0.id) < ($1.position, $1.id) }
            .lazy.compactMap { $0.displayName?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty } ?? ""
    }

    // MARK: Running

    /// Runs the steps from the first whose result is missing, until the
    /// account is added or a step needs the user: then the field to fill
    /// is shown, with why, and the flow stops there.
    func submit() async {
        guard busy == nil else { return }
        error = nil
        errorField = nil
        added = nil
        defer { busy = nil }

        let email = email.trimmingCharacters(in: .whitespacesAndNewlines)
        guard Self.looksLikeAddress(email) else {
            return stop(.email, L("Enter the whole address, like name@example.com."))
        }
        guard !password.isEmpty else { return stop(.password, L("Enter the password of the mailbox.")) }
        if let f = imapServer, f.params == nil {
            return stop(f.host.trimmingCharacters(in: .whitespaces).isEmpty ? .imapHost : .imapPort, L("Enter the IMAP server and its port."))
        }

        // 1. The servers of the address.
        if discovery == nil {
            busy = L("Looking for the servers of %@…", Self.domain(of: email))
            var p: [String: Any] = ["email": email]
            if let given = imapServer?.params { p["imap"] = given.json }
            do {
                discovery = try await client.call("discover_account", p) as DiscoveryReport
            } catch {
                return stop(.email, Self.message(error))
            }
        }
        guard let d = discovery else { return }

        // 2. The IMAP server: the one given, else the one found.
        let imap: ServerParams
        if let given = imapServer?.params {
            imap = given
        } else if let found = d.imap?.server, d.provider?.imapUnsupported != true {
            imap = found
        } else {
            showIMAP(ServerFields.guess("imap", domain: d.domain, port: 993), Self.whyNoIMAP(d))
            return
        }

        // 3. The IMAP server tried with the password.
        if test == nil {
            busy = L("Connecting to %@…", imap.host)
            do {
                test = try await client.call("test_account", ["email": email, "imap": imap.json, "password": password]) as AccountTestReport
            } catch let e as CoreClient.RPCError where e.reason == "imap_auth_refused" {
                // The server's own words, cited (`CoreReason`).
                return stop(.password, L("%@ refused the password: %@", imap.host, e.data["server_text"] ?? ""))
            } catch let e as CoreClient.RPCError where e.reason == "imap_unreachable" {
                showIMAP(imapServer ?? ServerFields(imap), L("NiceMailer could not connect to %@: %@. Check the IMAP server.", imap.host, e.data["server_text"] ?? ""))
                return
            } catch {
                return stop(.email, Self.message(error))
            }
        }
        guard let t = test else { return }

        // 4. The server to send from.
        if smtpChoice == nil && !smtpSkipped {
            if sendLater {
                smtpSkipped = true
            } else {
                guard await findSMTP(email: email, imap: imap, discovery: d) else { return }
            }
        }

        // 5. "On the server": what the account will change there, always
        // shown once before anything is written; the user adds the
        // account from it.
        guard let plan else {
            plan = ServerPlan(t)
            errorField = nil
            return
        }
        if let problem = plan.problem { return stop(nil, problem) }

        // 6. The account.
        busy = L("Adding %@…", email)
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        // Swift note: `merging(_:uniquingKeysWith:)` joins two
        // dictionaries; the closure says which value wins on a shared key.
        var p: [String: Any] = ["key": d.key, "email": email, "imap": imap.json, "password": password]
            .merging(plan.params(t)) { _, new in new }
        if !name.isEmpty { p["display_name"] = name }
        if let s = smtpChoice {
            p["smtp"] = s.json
            if let pw = smtpPassword, !pw.isEmpty { p["smtp_password"] = pw }
        }
        do {
            let _: AddAccountResult = try await client.call("add_account", p)
            let archived = plan.archived(t)
            reset()
            added = plan.lookOnly
                ? L("%@ added: NiceMailer shows its mail and changes nothing on the server. Organize it from its page when you are ready.", email)
                : archived > 0
                ? L("%@ added: %@ old mails are being archived on the server.", email, archived.formatted())
                : L("%@ added: its mail is being downloaded.", email)
            await onAdded?()
        } catch {
            stop(nil, Self.message(error))
        }
    }

    /// Tries the SMTP servers; `true` once one is chosen. Otherwise shows
    /// what is needed (its password, or a server) and returns `false`.
    private func findSMTP(email: String, imap: ServerParams, discovery d: DiscoveryReport) async -> Bool {
        let candidates: [ServerCandidate]
        if let given = smtpServer {
            guard let s = given.params else {
                stop(given.host.trimmingCharacters(in: .whitespaces).isEmpty ? .smtpHost : .smtpPort, L("Enter the SMTP server and its port."))
                return false
            }
            candidates = [ServerCandidate(server: s, origin: "given")]
        } else if let p = pendingSMTP {
            candidates = [p]
        } else {
            candidates = d.smtp
        }
        if candidates.isEmpty {
            showSMTP(ServerFields.guess("smtp", domain: d.domain, port: 465),
                     L("NiceMailer found no server to send mail from %@. Give the SMTP server, or add the account without sending for now.", email))
            return false
        }
        if let pw = smtpPassword, pw.isEmpty {
            stop(.smtpPassword, L("Enter the password of the SMTP server."))
            return false
        }

        busy = L("Trying the server to send mail…")
        var p: [String: Any] = ["email": email, "imap": imap.json, "password": password, "candidates": candidates.map(\.json)]
        if let pw = smtpPassword { p["smtp_password"] = pw }
        let r: SmtpTestReport
        do {
            r = try await client.call("test_smtp", p)
        } catch {
            stop(nil, Self.message(error))
            return false
        }
        switch Self.smtpVerdict(r, smtpPasswordGiven: smtpPassword != nil) {
        case .use(let c):
            smtpChoice = c.server
            return true
        case .needsPassword(let c, let why):
            pendingSMTP = c
            askSMTPPassword(Self.smtpPasswordMessage(why, server: c.server.host, imap: imap.host))
            return false
        case .noServer(let detail):
            showSMTP(smtpServer ?? candidates.first.map { ServerFields($0.server) } ?? ServerFields.guess("smtp", domain: d.domain, port: 465),
                     L("No server to send mail worked (%@). Check the SMTP server, or add the account without sending for now.", detail))
            return false
        }
    }

    /// Why an SMTP server needs a password of its own.
    enum SMTPPasswordReason: Equatable {
        /// It refused the IMAP password.
        case refused(String)
        /// It refused the SMTP password typed.
        case refusedAgain(String)
        /// Another organization than the IMAP server: never sent the IMAP
        /// password.
        case otherProvider
    }

    enum SMTPVerdict: Equatable {
        case use(ServerCandidate)
        case needsPassword(ServerCandidate, SMTPPasswordReason)
        case noServer(String)
    }

    /// What a `test_smtp` report means for the flow. Pure: tested apart.
    nonisolated static func smtpVerdict(_ r: SmtpTestReport, smtpPasswordGiven: Bool) -> SMTPVerdict {
        if let i = r.chosen, r.attempts.indices.contains(i) {
            let a = r.attempts[i]
            // `reachable`: the password was not tried. It is the IMAP one
            // that was not sent; with the SMTP password given, the server
            // would have answered `logged_in` or `auth_refused`.
            if a.outcome == "reachable" && !smtpPasswordGiven { return .needsPassword(a.candidate, .otherProvider) }
            return .use(a.candidate)
        }
        if let i = r.refused, r.attempts.indices.contains(i) {
            let a = r.attempts[i]
            let detail = a.detail ?? ""
            return .needsPassword(a.candidate, smtpPasswordGiven ? .refusedAgain(detail) : .refused(detail))
        }
        let details = r.attempts.map { "\($0.candidate.server.host):\($0.candidate.server.port): \($0.detail ?? $0.outcome)" }
        return .noServer(details.isEmpty ? "?" : details.joined(separator: "; "))
    }

    static func smtpPasswordMessage(_ why: SMTPPasswordReason, server: String, imap: String) -> String {
        switch why {
        case .refused(let d):
            return L("%@ refused your mailbox password (%@): it needs a password of its own, such as an app password.", server, d)
        case .refusedAgain(let d):
            return L("%@ refused this password too (%@).", server, d)
        case .otherProvider:
            return L("%@ is not run by the provider of %@: NiceMailer does not send it your mailbox password. Enter the password of this SMTP server.", server, imap)
        }
    }

    /// Why the IMAP server must be given.
    static func whyNoIMAP(_ d: DiscoveryReport) -> String {
        if let p = d.provider, p.imapUnsupported {
            return L("%@ only accepts a sign-in NiceMailer does not support yet (OAuth2). Give another IMAP server if your provider has one.", p.name)
        }
        if let e = d.mxError {
            return L("NiceMailer could not look up the mail servers of %@ (%@). Give the IMAP server.", d.domain, e)
        }
        if d.mxLookedUp && d.mx.isEmpty {
            return L("%@ has no mail server (no MX record). Give the IMAP server.", d.domain)
        }
        if let first = d.mx.first {
            return L("NiceMailer does not know the provider of %@ (its mail goes to %@). Give the IMAP server.", d.domain, first)
        }
        return L("NiceMailer does not know the provider of %@. Give the IMAP server.", d.domain)
    }

    // MARK: Helpers

    private func stop(_ field: Field?, _ message: String) {
        error = message
        errorField = field
    }

    private func showIMAP(_ fields: ServerFields, _ why: String) {
        if imapServer == nil { imapServer = fields }
        imapReason = why
        errorField = .imapHost
    }

    private func showSMTP(_ fields: ServerFields, _ why: String) {
        if smtpServer == nil { smtpServer = fields }
        smtpReason = why
        errorField = .smtpHost
    }

    private func askSMTPPassword(_ why: String) {
        // Shown empty: a password typed and refused is cleared, so that
        // the next one is not appended to it.
        smtpPassword = ""
        smtpPasswordReason = why
        errorField = .smtpPassword
    }

    /// Back to an empty form, after an account was added; the name of
    /// "From" stays, the next account is likely to use it too.
    private func reset() {
        let name = displayName
        let edited = displayNameEdited
        email = ""
        password = ""
        forget(from: .discovery)
        proposing = true
        displayName = name
        proposing = false
        displayNameEdited = edited
    }

    nonisolated static func looksLikeAddress(_ s: String) -> Bool {
        let parts = s.split(separator: "@", omittingEmptySubsequences: false)
        return parts.count == 2 && !parts[0].isEmpty && parts[1].contains(".") && !s.contains(" ")
    }

    nonisolated static func domain(of email: String) -> String {
        email.split(separator: "@").last.map(String.init)?.lowercased() ?? email
    }

    /// What to say of an error: for the core's, the sentence of its reason
    /// in the user's language (`CoreReason`).
    nonisolated static func message(_ error: Error) -> String {
        error.localizedDescription
    }
}

// MARK: - Organizing an account only looked at

/// An account added "only to look" (`read_only`), organized later: the same
/// plan as for a new account, read with the password of the Keychain
/// (`prepare_activation`), then applied (`activate_account`,
/// `docs/accounts.md`, "Looking first"). Its mail is not downloaded again;
/// the mails that came meanwhile are not screened, as those already there
/// at a creation (`import-initial.md`, section 1).
@Observable
@MainActor
final class AccountActivation {
    let key: String
    let email: String
    /// The report the plan was made from; `nil` until read.
    private(set) var report: AccountTestReport?
    var plan: ServerPlan?
    private(set) var busy: String?
    private(set) var error: String?
    /// Said once organized.
    private(set) var done: String?

    private let client: any CoreCalling
    /// Called once organized: the list reloads.
    var onDone: (() async -> Void)?

    init(client: any CoreCalling, key: String, email: String) {
        self.client = client
        self.key = key
        self.email = email
    }

    /// The import screen, when the core offers to archive.
    var importShown: Bool { plan != nil && report?.offerArchive == true }

    /// Reads what organizing would do: nothing changes yet.
    func prepare() async {
        guard busy == nil else { return }
        error = nil
        done = nil
        busy = L("Connecting to the server of %@…", email)
        defer { busy = nil }
        do {
            let r: AccountTestReport = try await client.call("prepare_activation", ["account": key])
            report = r
            plan = ServerPlan(r)
        } catch {
            self.error = AccountSetup.message(error)
        }
    }

    /// Back to the account's page, nothing done.
    func cancel() {
        report = nil
        plan = nil
        error = nil
    }

    /// Applies the plan.
    func activate() async {
        guard busy == nil, let r = report, let plan else { return }
        if let problem = plan.problem {
            error = problem
            return
        }
        error = nil
        busy = L("Organizing %@…", email)
        defer { busy = nil }
        var p = plan.params(r)
        p["account"] = key
        do {
            let _: IgnoredResult = try await client.call("activate_account", p)
            let archived = plan.archived(r)
            report = nil
            self.plan = nil
            done = archived > 0
                ? L("%@ is organized: %@ old mails are being archived on the server.", email, archived.formatted())
                : L("%@ is organized: new mail is screened from now on.", email)
            await onDone?()
        } catch {
            self.error = AccountSetup.message(error)
        }
    }
}
