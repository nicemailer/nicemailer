import SwiftUI

/// The Preferences window (the app menu's "Settings…", ⌘,): one tab for
/// now, the accounts (`docs/accounts.md`, "In the app"). Other settings
/// will join it as tabs.
///
/// Its controls are the Mac's standard ones in either theme: a window of
/// settings is not a place for the geek theme's keys.
///
/// SwiftUI note: a `Settings` scene (`NiceMailerApp`) gives the menu item
/// and its shortcut for free; the view only says what the window holds.
struct PreferencesView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        TabView {
            Group {
                if let core = model.accountsCore {
                    // A new connection (the core restarted) makes a new
                    // pane: the old one would keep calling a closed socket.
                    AccountsPane(client: core)
                        .id(ObjectIdentifier(core))
                } else {
                    Text(L("Connecting to the core…"))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .tabItem { Label(L("Accounts"), systemImage: "at") }
        }
        .frame(width: 720, height: 540)
    }
}

/// What the right side of the accounts pane shows.
enum AccountsSelection: Hashable {
    case add
    case account(Int)
}

/// The accounts, in display order, on the left; the new account's form or
/// the selected account on the right.
struct AccountsPane: View {
    @Environment(AppModel.self) private var model
    /// Swift note: `@State` keeps these objects alive across redraws; they
    /// are made once, from the client the pane was created with.
    /// `@Bindable` is not needed for `$list.selection`: `@State` of an
    /// `@Observable` object gives bindings to its properties too.
    @State private var list: AccountsList
    @State private var setup: AccountSetup

    init(client: any CoreCalling) {
        _list = State(initialValue: AccountsList(client: client))
        _setup = State(initialValue: AccountSetup(client: client))
    }

    var body: some View {
        HStack(spacing: 0) {
            sidebar
                .frame(width: 230)
            Divider()
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .task {
            // Swift note: `[weak s]` keeps the closure from owning the
            // object that owns it (a cycle neither would ever leave).
            let (l, s) = (list, setup)
            s.onAdded = { [weak l, weak s] in
                guard let l, let s else { return }
                await l.load()
                s.proposeDisplayName(l.items)
            }
            await list.load()
            setup.proposeDisplayName(list.items)
            // No account yet: the form is what the user came for.
            if list.selection == nil { list.selection = list.items.first.map { .account($0.id) } ?? .add }
            UIScript.preferencesOpened(list: list, setup: setup)
        }
        // The core announced a change (another client added an account,
        // a server refused its password): read the list again.
        .onChange(of: model.accountsRevision) {
            Task {
                await list.load()
                setup.proposeDisplayName(list.items)
            }
        }
    }

    private var sidebar: some View {
        VStack(spacing: 0) {
            // Swift note: `List(selection:)` binds the highlighted row to
            // `selection`; each row's `.tag` says which value it stands for.
            List(selection: $list.selection) {
                Section(L("Accounts")) {
                    ForEach(list.items) { a in
                        AccountRow(account: a)
                            .tag(AccountsSelection.account(a.id))
                            .uiTarget("prefs-account-\(a.key)")
                    }
                    // A drag reorders; the buttons below do the same.
                    .onMove { offsets, destination in
                        Task { await list.move(fromOffsets: offsets, toOffset: destination) }
                    }
                }
            }
            .listStyle(.sidebar)
            Divider()
            HStack(spacing: 2) {
                Button {
                    list.selection = .add
                } label: {
                    Image(systemName: "plus").frame(width: 22, height: 18)
                }
                .help(L("Add an account"))
                .uiTarget("prefs-plus")
                Spacer()
                Button {
                    move(-1)
                } label: {
                    Image(systemName: "chevron.up").frame(width: 22, height: 18)
                }
                .help(L("Move the account up"))
                .disabled(!canMove(-1))
                .uiTarget("prefs-up")
                Button {
                    move(1)
                } label: {
                    Image(systemName: "chevron.down").frame(width: 22, height: 18)
                }
                .help(L("Move the account down"))
                .disabled(!canMove(1))
                .uiTarget("prefs-down")
            }
            .buttonStyle(.borderless)
            .padding(6)
            if let e = list.error {
                Text(e).font(.caption).foregroundStyle(.red).padding([.horizontal, .bottom], 6)
            }
        }
    }

    @ViewBuilder private var detail: some View {
        switch list.selection {
        case .account(let id):
            if let a = list.items.first(where: { $0.id == id }) {
                AccountDetail(account: a)
            } else {
                AddAccountForm(setup: setup)
            }
        case .add, nil:
            AddAccountForm(setup: setup)
        }
    }

    private func canMove(_ step: Int) -> Bool {
        list.selectedId.map { list.canMove($0, by: step) } ?? false
    }

    private func move(_ step: Int) {
        guard let id = list.selectedId else { return }
        Task { await list.move(id, by: step) }
    }
}

/// One account of the list: its name or address, and what is wrong.
private struct AccountRow: View {
    let account: AccountInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(account.email ?? account.key)
                .lineLimit(1)
            Text(AccountRow.subtitle(account))
                .font(.caption)
                .foregroundStyle(account.error != nil ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                .lineLimit(1)
        }
        .padding(.vertical, 2)
    }

    static func subtitle(_ a: AccountInfo) -> String {
        state(a) ?? a.key
    }

    /// What is special about the account, if anything.
    static func state(_ a: AccountInfo) -> String? {
        if a.kind == "archive" { return L("Archive (mbox import)") }
        if !a.enabled { return L("Disabled: no longer in accounts.toml") }
        if a.error != nil { return L("Cannot connect") }
        return nil
    }
}

/// An account of the list, selected: what the core says of it. Changing it
/// (key, servers, removal) is still done with the CLI (`docs/accounts.md`).
private struct AccountDetail: View {
    let account: AccountInfo

    var body: some View {
        Form {
            Section {
                LabeledContent(L("Address"), value: account.email ?? "—")
                LabeledContent(L("Name shown in “From”"), value: account.displayName ?? "—")
                LabeledContent(L("Key in accounts.toml"), value: account.key)
                LabeledContent(L("State"), value: AccountRow.state(account) ?? L("Active"))
                if let e = account.error {
                    Text(e).foregroundStyle(.red).textSelection(.enabled)
                }
            }
        }
        .formStyle(.grouped)
    }
}

/// The new account: three fields, then those the attempt shows are needed.
struct AddAccountForm: View {
    /// Swift note: `@Bindable` turns the properties of an `@Observable`
    /// object into bindings (`$setup.email`), for the fields to edit them.
    @Bindable var setup: AccountSetup
    /// Swift note: `@FocusState` is the field that has the keyboard; set
    /// from the flow, it moves the cursor to the field asked for.
    @FocusState private var focus: AccountSetup.Field?

    var body: some View {
        VStack(spacing: 0) {
            // Swift note: `ScrollViewReader` gives a `proxy` that scrolls
            // the form to a view by its `.id`.
            ScrollViewReader { proxy in
                Form {
                    Section {
                        TextField(L("Email address"), text: $setup.email, prompt: Text(verbatim: "name@example.com"))
                            .textContentType(.username)
                            .focused($focus, equals: .email)
                            .uiTarget("prefs-email")
                        SecureField(L("Password"), text: $setup.password)
                            .focused($focus, equals: .password)
                            .uiTarget("prefs-password")
                        TextField(L("Name shown in “From”"), text: $setup.displayName, prompt: Text(L("Your name")))
                            .focused($focus, equals: .displayName)
                            .uiTarget("prefs-name")
                    } header: {
                        Text(L("Add an account"))
                    } footer: {
                        Text(L("NiceMailer finds the servers from the address. It asks for more only if it needs to."))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if setup.imapServer != nil { imapSection.id(AccountSetup.Section.imap) }
                    if setup.smtpPassword != nil { smtpPasswordSection.id(AccountSetup.Section.smtpPassword) }
                    if setup.smtpServer != nil { smtpServerSection.id(AccountSetup.Section.smtp) }
                    if setup.importShown, let t = setup.test { importSection(t).id(AccountSetup.Section.importScreen) }
                }
                .formStyle(.grouped)
                .onSubmit { Task { await setup.submit() } }
                // A section asked for appears at the bottom of the form, maybe
                // out of sight: brought into view. (A field that takes the
                // focus is scrolled to by AppKit; the import screen has none.)
                .onChange(of: setup.sectionsShown) { old, new in
                    guard let added = new.first(where: { !old.contains($0) }) else { return }
                    proxy.scrollTo(added, anchor: .bottom)
                }
            }
            Divider()
            footer
        }
        .onAppear { if focus == nil { focus = .email } }
        .onChange(of: setup.errorField) { _, f in if let f { focus = f } }
    }

    // MARK: Sections asked for

    private var imapSection: some View {
        Section {
            if let why = setup.imapReason { reason(why) }
            ServerFieldsView(fields: optional(\.imapServer), host: .imapHost, port: .imapPort, focus: $focus, target: "imap")
        } header: {
            Text(L("Incoming mail (IMAP)"))
        }
    }

    private var smtpPasswordSection: some View {
        Section {
            if let why = setup.smtpPasswordReason { reason(why) }
            // Written only while shown: a field that loses the focus as its
            // section goes away writes its text back, which would bring
            // the section back.
            SecureField(L("SMTP password"), text: Binding(get: { setup.smtpPassword ?? "" }, set: { if setup.smtpPassword != nil { setup.smtpPassword = $0 } }))
                .focused($focus, equals: .smtpPassword)
                .disabled(setup.sendLater)
                .uiTarget("prefs-smtp-password")
            // No password at hand (an app password to create first): the
            // account can be added now and its SMTP password saved later.
            // Offered once: here, unless the SMTP server is asked too.
            if setup.smtpServer == nil { sendLaterToggle }
        } header: {
            Text(L("Outgoing mail (SMTP)"))
        }
    }

    private var smtpServerSection: some View {
        Section {
            if let why = setup.smtpReason { reason(why) }
            ServerFieldsView(fields: optional(\.smtpServer), host: .smtpHost, port: .smtpPort, focus: $focus, target: "smtp")
                .disabled(setup.sendLater)
            sendLaterToggle
        } header: {
            Text(L("Outgoing mail (SMTP)"))
        }
    }

    private var sendLaterToggle: some View {
        Toggle(L("Add the account without sending for now"), isOn: $setup.sendLater)
            .uiTarget("prefs-send-later")
    }

    private func importSection(_ t: AccountTestReport) -> some View {
        Section {
            Text(L("Your Inbox holds %@ mails, %@ of them unread.", t.inboxMessages.formatted(), t.inboxUnseen.formatted()))
            Toggle(L("Archive the read mails received before %@", Self.day(t.readBefore)), isOn: $setup.importChoices.archiveRead)
                .uiTarget("prefs-archive-read")
            Toggle(L("Archive the unread mails received before %@", Self.day(t.unreadBefore)), isOn: $setup.importChoices.archiveUnread)
                .uiTarget("prefs-archive-unread")
            let archived = setup.importChoices.archived(t)
            Text(L("Flagged mails stay in the Inbox.")).font(.caption).foregroundStyle(.secondary)
            Text(L("→ %@ mails will be archived, %@ will stay in the Inbox.", archived.formatted(), max(0, t.inboxMessages - archived).formatted()))
                .bold()
            Toggle(L("The people you have written to (%@) arrive straight in the Inbox.", t.correspondents.formatted()), isOn: $setup.importChoices.acceptCorrespondents)
                .uiTarget("prefs-correspondents")
            Text(L("Other senders go through the Quarantine with their next mail.")).font(.caption).foregroundStyle(.secondary)
        } header: {
            Text(L("Mail already in the mailbox"))
        }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(alignment: .center, spacing: 10) {
            Group {
                if let busy = setup.busy {
                    ProgressView().controlSize(.small)
                    Text(busy).foregroundStyle(.secondary)
                } else if let e = setup.error {
                    Text(e).foregroundStyle(.red)
                } else if let done = setup.added {
                    Text(done).foregroundStyle(.green)
                }
            }
            .font(.callout)
            .lineLimit(3)
            .textSelection(.enabled)
            Spacer()
            Button(L("Add Account")) { Task { await setup.submit() } }
                .keyboardShortcut(.defaultAction)
                .disabled(setup.busy != nil)
                .uiTarget("prefs-add")
        }
        .padding(12)
    }

    private func reason(_ text: String) -> some View {
        Text(text).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
    }

    /// A binding to a server shown (never `nil` while its section is).
    /// It writes only while the section is shown: a field that loses the
    /// focus as its section goes away (the account added, the form emptied)
    /// writes its text back, which would bring the section back.
    /// Swift note: `ReferenceWritableKeyPath` names a property of a class
    /// that can be written, here `\.imapServer` or `\.smtpServer`.
    private func optional(_ path: ReferenceWritableKeyPath<AccountSetup, ServerFields?>) -> Binding<ServerFields> {
        Binding(get: { setup[keyPath: path] ?? ServerFields(host: "", port: 993, security: "tls") },
                set: { if setup[keyPath: path] != nil { setup[keyPath: path] = $0 } })
    }

    /// A day of the core (`YYYY-MM-DD`) in the user's words, in the
    /// language of the app (`appLocale`), not only the system's.
    static func day(_ iso: String) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = "yyyy-MM-dd"
        guard let d = f.date(from: iso) else { return iso }
        let out = DateFormatter()
        out.locale = appLocale
        out.timeZone = TimeZone(identifier: "UTC")
        out.dateStyle = .long
        out.timeStyle = .none
        return out.string(from: d)
    }
}

/// Host, port, security and user name of a server.
private struct ServerFieldsView: View {
    @Binding var fields: ServerFields
    let host: AccountSetup.Field
    let port: AccountSetup.Field
    var focus: FocusState<AccountSetup.Field?>.Binding
    /// `imap` or `smtp`, for the UI scripts.
    let target: String

    var body: some View {
        TextField(L("Server"), text: $fields.host, prompt: Text(verbatim: "\(target).example.com"))
            .focused(focus, equals: host)
            .uiTarget("prefs-\(target)-host")
        TextField(L("Port"), text: $fields.port)
            .focused(focus, equals: port)
            .uiTarget("prefs-\(target)-port")
        Picker(L("Security"), selection: $fields.security) {
            Text(verbatim: "SSL/TLS").tag("tls")
            Text(verbatim: "STARTTLS").tag("starttls")
        }
        TextField(L("User name"), text: $fields.username, prompt: Text(L("The address")))
            .uiTarget("prefs-\(target)-user")
    }
}
