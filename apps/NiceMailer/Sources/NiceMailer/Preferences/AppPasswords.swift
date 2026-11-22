import Foundation

// The providers that refuse the password of their website to a mail app
// and ask for an app password instead: Gmail, iCloud, Fastmail
// (`docs/accounts.md`, "App passwords"). The Preferences say so under the
// password, before it is tried, with a link to the step-by-step guide of
// the website, and again when the server refuses it. Without this, the
// user only sees "the server refused the password", which reads as a typo.

/// A provider that asks for an app password.
enum AppPasswordProvider: String, CaseIterable {
    case gmail, icloud, fastmail

    /// The name the provider gives itself: not translated.
    var name: String {
        // Swift note: since Swift 5.9 a `switch` is an expression; each case
        // is the value returned.
        switch self {
        case .gmail: "Gmail"
        case .icloud: "iCloud"
        case .fastmail: "Fastmail"
        }
    }

    /// The provider of an address, if it asks for an app password: by the
    /// domain of the address, known before any lookup; else by the provider
    /// the core found (`discover_account`), by the domain or by the MX,
    /// which also catches a domain of one's own hosted there (Google
    /// Workspace, Fastmail). The ids are those of the providers' database
    /// (`data/providers/thunderbird.json`).
    nonisolated static func of(email: String, provider: ProviderMatch?) -> AppPasswordProvider? {
        switch AccountSetup.domain(of: email) {
        case "gmail.com", "googlemail.com": return .gmail
        case "icloud.com", "me.com", "mac.com": return .icloud
        case "fastmail.com", "fastmail.fm": return .fastmail
        default: break
        }
        switch provider?.id {
        case "googlemail.com": return .gmail
        case "me.com": return .icloud
        case "messagingengine": return .fastmail
        default: return nil
        }
    }

    /// The guide of the website, at this provider's section. The site has
    /// English and French pages: French when the app shows French, else
    /// English. The anchor is the id Astro gives the section's title.
    nonisolated func guide(language: String = appLanguage) -> URL {
        let page = language == "fr" ? "https://nicemailer.org/fr/docs/app-passwords/" : "https://nicemailer.org/docs/app-passwords/"
        return URL(string: page + "#" + rawValue)!
    }

    /// What the Preferences say under the password.
    var hint: String {
        L("%@ asks for an app password, not the password of its website.", name)
    }
}
