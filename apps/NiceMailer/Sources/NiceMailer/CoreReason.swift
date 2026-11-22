import Foundation
import os

/// The sentence of an error of the core, in the user's language
/// (`docs/localization.md`, "Errors of the core"; `localisation.md`,
/// section 3.1).
///
/// The core does not know the user's language: an error says why by a
/// stable reason and its values (`data`, `nm_api::reason`), and its
/// `message` is the English sentence of the CLI and the logs. This is the
/// one place where the app turns a reason into words. The values whose key
/// ends with `_text` were written by someone else (a mail server, the
/// system, the AI provider): they are cited as they are.
///
/// Every reason of the contract is either written here or listed in
/// `generic`; `LocalizationTests` reads the reasons of `docs/contract.md`
/// and fails on one that is neither.
enum CoreReason {
    /// The reasons the app says with the generic sentence: mistakes of a
    /// client's program, which a user cannot correct, and refusals made to
    /// AI agents, which the app never meets. The English `message` goes
    /// with it, for whoever reports the problem.
    static let generic: Set<String> = [
        "parse_error",
        "invalid_request",
        "method_not_found",
        "invalid_params",
        "internal",
        "hello_expected",
        "protocol_version",
        "reserved_to_people",
        "reserved_to_app",
        "agent_cannot_send",
        "agent_cannot_file",
        "agent_cannot_unsubscribe",
        "agent_cannot_set_password",
        "agent_not_allowed",
        "draft_by_user",
        "reply_started_by_user",
    ]

    static let log = Logger(subsystem: "eu.nicemailer.app", category: "core-reason")

    /// What the app says of `error`: its reason's sentence, else the
    /// generic sentence with the core's English message. A reason neither
    /// written nor generic is a reason added to the core and forgotten
    /// here, which the tests should have caught: logged as an error, said
    /// generically rather than left blank.
    static func text(_ error: CoreClient.RPCError) -> String {
        if let reason = error.reason {
            if let s = sentence(reason, error.data) { return s }
            if !generic.contains(reason) {
                log.error("no sentence for the reason \(reason, privacy: .public): \(error.message, privacy: .public)")
            }
        }
        return L("NiceMailer could not do it: %@", error.message)
    }

    /// What the app says of a `why` of an event or a result (`AccountError`,
    /// `SendFailed`…): its reason's sentence, else the core's English
    /// `message`, else nil.
    static func text(why: [String: String]?, message: String?) -> String? {
        if let why, let reason = why["reason"], let s = sentence(reason, why) { return s }
        if let reason = why?["reason"], !generic.contains(reason) {
            log.error("no sentence for the reason \(reason, privacy: .public)")
        }
        return message
    }

    /// The sentence of `reason` with its values, nil for a generic or an
    /// unknown reason.
    ///
    /// Swift note: a `switch` whose cases are expressions returns their
    /// value directly (Swift 5.9), without a `return` in each case.
    static func sentence(_ reason: String, _ values: [String: String]) -> String? {
        func v(_ key: String) -> String { values[key] ?? "" }
        return switch reason {
        case "not_found": L("It no longer exists: it may have been deleted meanwhile")
        case "account_unknown": L("No account %@ in accounts.toml", v("account"))
        case "account_not_in_database": L("The account %@ is not ready yet: try again in a moment", v("account"))
        case "account_not_synchronized": L("The account %@ is not synchronized", v("account"))
        case "no_imap_account": L("No account to synchronize")
        case "account_exists": L("An account %@ exists already", v("account"))
        case "account_key_taken": L("The name %@ is taken by another account: choose another", v("account"))
        case "account_set_up": L("The account %@ is organized already", v("account"))
        case "account_read_only": L("This account is only looked at: NiceMailer changes nothing in it")
        case "account_read_only_send": L("This account is only looked at: it cannot send")
        case "theme_unknown": L("No theme %@: neither built in nor a file of the folder themes", v("theme"))
        case "config_not_written": L("config.toml was not changed: %@", v("detail"))
        case "password_empty": L("Give the password")
        case "smtp_password_empty": L("Give the SMTP password")
        case "smtp_password_without_server": L("An SMTP password needs an SMTP server")
        case "no_smtp_server": L("The account %@ has no SMTP server: it cannot send", v("account"))
        case "date_invalid": L("%@ is not a date (YYYY-MM-DD)", v("date"))
        case "read_only_archives_nothing": L("An account only looked at archives nothing on its server")
        case "server_folder_missing": L("The server has no folder %@", v("folder"))
        case "keychain": L("The Keychain refused: %@", v("system_text"))
        case "password_missing": L("No password saved for the account %@: give it in the Preferences", v("account"))
        case "oauth2_unsupported": L("OAuth2 sign-in is not supported yet")
        case "password_server_changed": L("The password of %@ was saved for another server (%@ on %@): give it again to confirm %@ on %@", v("account"), v("saved_user"), v("saved_host"), v("user"), v("host"))
        case "smtp_password_server_changed": L("The SMTP password of %@ was saved for another server (%@ on %@): give it again to confirm %@ on %@", v("account"), v("saved_user"), v("saved_host"), v("user"), v("host"))
        case "smtp_password_needed": L("The SMTP server %@ is not of the same provider as %@: give its own password", v("smtp_host"), v("host"))
        case "imap_auth_refused": L("The server refused the login: %@", v("server_text"))
        case "imap_unreachable": L("The server could not be reached: %@", v("server_text"))
        case "imap_failed": L("The server failed: %@", v("server_text"))
        case "mail_not_in_account": L("This mail is in an archive: it cannot be answered from here")
        case "no_account_can_send": L("No account can send: add an SMTP server to one")
        case "archive_cannot_send": L("This mail is in an archive account, which cannot send")
        case "no_sender_address": L("The account has no address to send from")
        case "not_own_address": L("%@ is not an address of this account", v("address"))
        case "draft_no_recipient": L("This mail has no recipient")
        case "draft_cannot_be_answered": L("A draft cannot be answered")
        case "draft_cannot_be_forwarded": L("A draft cannot be forwarded")
        case "formatted_text_too_big": L("The formatted text is too big")
        case "revision_conflict": L("This draft was changed elsewhere: reload it")
        case "draft_sending": L("This mail is being sent: cancel the sending first")
        case "send_started": L("Too late to cancel: the mail is leaving, or has left")
        case "draft_copying": L("This draft is being copied to the server: try again in a moment")
        case "body_pending": L("The text of this mail is still downloading: try again in a moment")
        case "raw_damaged": L("The file of this mail is damaged: it is downloaded again from the server if possible")
        case "server_refused_change": L("The server refused the change: %@", v("server_text"))
        case "attempts_failed": L("The server could not be reached after %@ attempts: %@", v("attempts"), v("server_text"))
        case "smtp_refused": L("The mail server refused the mail: %@", v("server_text"))
        case "move_refused": L("This move is not offered from this folder")
        case "confirmation_required": L("The mail of %@ may be forged: confirm first", v("addr"))
        case "tag_name_invalid": L("A tag name has 1 to 64 characters")
        case "tag_exists": L("The tag %@ exists already", v("tag"))
        case "group_name_invalid": L("A group name has 1 to 64 characters")
        case "group_exists": L("The group %@ exists already", v("group"))
        case "address_invalid": L("%@ is not an address", v("address"))
        case "domain_invalid": L("%@ is not a domain", v("domain"))
        case "import_running": L("An import is already running")
        case "path_not_absolute": L("%@ is not a full path", v("path"))
        case "not_a_file": L("%@ is not a file", v("path"))
        case "folder_missing": L("The folder %@ does not exist", v("path"))
        case "file_unreadable": L("%@ cannot be read: %@", v("path"), v("system_text"))
        case "database_not_empty": L("This mailbox already holds mail")
        case "ai_disabled": L("The AI is off: set ai.enabled = true in config.toml")
        case "ai_provider_missing": L("Claude Code is not installed: install it, or set ai.claude_path in config.toml")
        case "ai_not_logged_in": L("Claude Code is not signed in: run claude in a terminal to sign in")
        case "ai_provider_unsafe": L("Claude Code announced tools: nothing was sent to it")
        case "ai_provider_unsupported": L("The AI provider %@ is not available yet", v("provider"))
        case "ai_rate_limited": L("The usage limit of the Claude subscription is reached")
        case "ai_timeout": L("The AI did not answer in time")
        case "ai_invalid_output": L("The AI’s answer could not be read")
        case "ai_failed": L("The AI failed: %@", v("provider_text"))
        case "ai_io": L("Claude Code could not be run: %@", v("system_text"))
        case "ai_prompt_unavailable": L("This prompt is not available in this version yet")
        case "ai_prompt_not_here": L("This prompt is not offered in this folder")
        case "ai_selection_needed": L("Select the text to work on first")
        case "ai_selection_unwanted": L("This prompt works on the whole text: select nothing")
        case "ai_selection_empty": L("The selection is empty")
        case "ai_text_empty": L("There is no text of yours to send: write some first")
        case "ai_segment_tag": L("The text contains a <segment> tag, which the AI uses: remove it first")
        case "ai_mail_downloading": L("This mail is still downloading: try again in a moment")
        case "ai_folder_without_role": L("This folder has no role: nothing to sort it into")
        case "ai_folder_empty": L("This folder is empty: nothing to sort")
        case "ai_mark_to_sort": L("Mark the conversations to sort, or put the cursor on one")
        case "ai_draft_unsaved": L("The reply changed since: run it again")
        case "ai_cancelled": L("Cancelled")
        case "ai_draft_changed": L("The reply changed since the AI read it: run it again")
        case "file_unsupported": L("%@: this kind of file has no text the AI can read", v("file"))
        case "file_too_large": L("%@ is too large to read (%@ MB, 50 MB at most)", v("file"), v("size_mb"))
        case "file_encrypted": L("%@ is protected by a password", v("file"))
        case "file_without_text": L("%@ has no text: a scan or an image?", v("file"))
        case "file_damaged": L("%@ cannot be read", v("file"))
        case "update": L("NiceMailer is being updated: it opens again in a moment")
        case "migration": L("The mailbox is being upgraded: NiceMailer opens again when it ends")
        case "newer_schema": L("This mailbox was written by a newer version of NiceMailer: update NiceMailer to open it")
        case "migration_failed": L("The upgrade of the mailbox failed: %@", v("message"))
        default: nil
        }
    }
}

/// A reason and its values as a result carries them (`AccountView.why`),
/// decoded as text.
///
/// Swift note: the app's `JSONDecoder` converts snake_case keys to
/// camelCase (`keyDecodingStrategy`), the keys of a dictionary included:
/// `server_text` arrives as `serverText`. The keys of a reason's values
/// are lowercase words joined by `_`, so the conversion is undone here,
/// and `CoreReason` reads the names of the contract.
struct CoreWhy: Decodable, Equatable, Sendable {
    let values: [String: String]

    init(values: [String: String]) { self.values = values }

    init(from decoder: Decoder) throws {
        struct Key: CodingKey {
            var stringValue: String
            var intValue: Int? { nil }
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { nil }
        }
        let c = try decoder.container(keyedBy: Key.self)
        var out: [String: String] = [:]
        for key in c.allKeys {
            let name = CoreWhy.snake(key.stringValue)
            if let s = try? c.decode(String.self, forKey: key) {
                out[name] = s
            } else if let n = try? c.decode(Int.self, forKey: key) {
                out[name] = String(n)
            } else if let list = try? c.decode([String].self, forKey: key) {
                out[name] = list.joined(separator: ", ")
            }
        }
        values = out
    }

    /// `serverText` back to `server_text`.
    static func snake(_ key: String) -> String {
        key.reduce(into: "") { out, c in
            if c.isUppercase { out += "_" + c.lowercased() } else { out.append(c) }
        }
    }
}
