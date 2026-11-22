import Foundation
import Testing
@testable import NiceMailer

/// The providers that ask for an app password (`AppPasswordProvider`,
/// `docs/accounts.md`, "App passwords"), and a new password for an account
/// already set (`AccountPassword`, "A new password").
@MainActor
struct AppPasswordTests {
    @Test func theProviderIsKnownByTheDomainOfTheAddress() {
        func of(_ email: String) -> AppPasswordProvider? { AppPasswordProvider.of(email: email, provider: nil) }
        #expect(of("pierre@gmail.com") == .gmail)
        #expect(of("Pierre@GoogleMail.com") == .gmail)
        #expect(of("pierre@icloud.com") == .icloud && of("pierre@me.com") == .icloud && of("pierre@mac.com") == .icloud)
        #expect(of("pierre@fastmail.com") == .fastmail && of("pierre@fastmail.fm") == .fastmail)
        #expect(of("pierre@toto.fr") == nil)
        #expect(of("") == nil)
    }

    @Test func aDomainOfOnesOwnIsKnownByTheProviderFound() {
        func of(_ id: String?) -> AppPasswordProvider? {
            AppPasswordProvider.of(email: "pierre@acme.fr", provider: ProviderMatch(id: id, name: "Some", imapUnsupported: false))
        }
        // Google Workspace and Fastmail are found by the MX of the domain.
        #expect(of("googlemail.com") == .gmail)
        #expect(of("messagingengine") == .fastmail)
        #expect(of("me.com") == .icloud)
        #expect(of("infomaniak") == nil)
        #expect(of(nil) == nil)
    }

    @Test func theGuideIsTheSectionOfTheSiteInTheAppsLanguage() {
        #expect(AppPasswordProvider.gmail.guide(language: "en").absoluteString == "https://nicemailer.org/docs/app-passwords/#gmail")
        #expect(AppPasswordProvider.icloud.guide(language: "fr").absoluteString == "https://nicemailer.org/fr/docs/app-passwords/#icloud")
        // The site has English and French only.
        #expect(AppPasswordProvider.fastmail.guide(language: "de").absoluteString == "https://nicemailer.org/docs/app-passwords/#fastmail")
    }

    @Test func theFormSaysItAsSoonAsTheAddressIsTyped() {
        let setup = AccountSetup(client: FakeCore { method, params in Fixture.answer(method, params) })
        #expect(setup.appPassword == nil)
        setup.email = "pierre@toto.fr"
        #expect(setup.appPassword == nil)
        setup.email = " pierre@gmail.com "
        #expect(setup.appPassword == .gmail)
    }

    @Test func aRefusalByAProviderThatAsksForAnAppPasswordSaysSo() async {
        let workspace: [String: Any] = ["id": "googlemail.com", "name": "Google Mail", "source": "thunderbird", "matched": "google.com",
                                        "mx_host": "aspmx.l.google.com", "imap_unsupported": false]
        let core = FakeCore { method, params in
            switch method {
            case "discover_account": return AccountSetupTests.discovery(provider: workspace)
            case "test_account": return CoreClient.RPCError(code: 1009, message: "the server refused the login: bad",
                                                            data: ["reason": "imap_auth_refused", "server_text": "bad"])
            default: return Fixture.answer(method, params)
            }
        }
        let setup = AccountSetup(client: core)
        setup.email = "pierre@acme.fr"
        setup.password = "the password of the website"
        setup.displayName = "Pierre"
        // A domain of one's own: unknown until the servers are found.
        #expect(setup.appPassword == nil)
        await setup.submit()
        #expect(setup.appPassword == .gmail)
        #expect(setup.errorField == .password)
        // The likely cause first, then the server's words.
        #expect(setup.error == L("%@ asks for an app password: see the link under the password.", "Gmail") + " "
                + L("%@ refused the password: %@", "imap.toto.fr", "bad"))
    }

    @Test func aNewPasswordIsSavedThenTheFieldEmptied() async {
        let core = FakeCore { method, params in method == "set_password" ? ["changed": 0] : Fixture.answer(method, params) }
        let edit = AccountPassword(client: core, key: "perso", email: "pierre@gmail.com")
        final class Box { var reloaded = false }
        let box = Box()
        edit.onSaved = { box.reloaded = true }
        #expect(edit.appPassword == .gmail)

        // Nothing typed: said, nothing sent.
        await edit.save()
        #expect(edit.error == L("Enter the new password."))
        #expect(core.calls("set_password").isEmpty)

        edit.password = "abcd efgh ijkl mnop"
        #expect(edit.error == nil)
        await edit.save()
        let calls = core.calls("set_password")
        #expect(calls.count == 1)
        #expect(calls.first?["account"] as? String == "perso")
        #expect(calls.first?["password"] as? String == "abcd efgh ijkl mnop")
        // The IMAP password: the SMTP server takes it too, unless it has its own.
        #expect(calls.first?["smtp"] == nil)
        #expect(edit.password.isEmpty)
        #expect(edit.saved == L("Password saved: NiceMailer connects with it now."))
        #expect(box.reloaded)

        // Typing again forgets what was said.
        edit.password = "x"
        #expect(edit.saved == nil)
    }

    @Test func aRefusalOfTheCoreIsSaidAndThePasswordKept() async {
        let refusal = CoreClient.RPCError(code: 1010, message: "the Keychain refused: locked", data: ["reason": "keychain", "system_text": "locked"])
        let core = FakeCore { method, params in method == "set_password" ? refusal : Fixture.answer(method, params) }
        let edit = AccountPassword(client: core, key: "perso", email: "pierre@toto.fr")
        #expect(edit.appPassword == nil)
        edit.password = "secret"
        await edit.save()
        #expect(edit.error == AccountSetup.message(refusal))
        #expect(edit.password == "secret" && edit.saved == nil)
    }
}
