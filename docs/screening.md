# Screening at reception

Every new mail of the Inbox of an IMAP account is sorted as it arrives, by its sender and by whether the sender is authenticated. This is phase 6 of `specifications/plan-imap.md`; the rules are in `specifications/dossiers.md` and `specifications/authentification-expediteur.md`. The code is `crates/nm-store/src/auth.rs` (the verdict) and `crates/nm-store/src/reception.rs` (what an arrival does), called by the synchronization ([sync.md](sync.md)) in the transaction that inserts the mail: a mail is never visible in the Inbox before it is screened.

## The authentication verdict

The `From` header is free text. The receiving server checks SPF, DKIM and DMARC and writes what it found in an `Authentication-Results` header at the top of the message. Every received mail, in any folder, gets a verdict from it (`message.auth_result`, `auth_source = 'server'`):

| Verdict | When |
| --- | --- |
| `pass` | `dmarc=pass`; or no DMARC result (or `dmarc=none`) and a `dkim=pass` whose `header.d` is aligned on the domain of `From` |
| `fail` | `dmarc=fail` |
| `none` | no DMARC verdict and no aligned DKIM signature |
| `unknown` | `dmarc=temperror` or `permerror`; no trusted header; no `From`, or several addresses in it |

**Which header is trusted.** A sender can write a fake `Authentication-Results: … dmarc=pass` into its mail. Only the headers whose `authserv-id` (their first word) is the account's are read, and of those the first from the top, the one the server added last: a conforming server removes incoming headers that carry its own id (RFC 8601, section 5).

**The account's `authserv-id`.** The key `authserv_id` of the account in `accounts.toml` if present, else the one detected: the id that tops the first `Authentication-Results` of at least 90 % of the 50 most recent mails of the Inbox (highest UIDs). The detection runs when a pass brings mail to the Inbox, at most once an hour while none is found, and the result is kept in `account.authserv_id`. `nicemailer sync status` shows the id trusted ("trusted Authentication-Results"), or that none was found, in which case every verdict is `unknown`.

**Alignment.** DMARC's relaxed alignment: the same *organizational domain*, the registrable domain under a public suffix. `mail.exemple.fr` is aligned on `exemple.fr`; `a.co.uk` is not aligned on `b.co.uk`, since `co.uk` is a public suffix. The suffixes come from the Public Suffix List (https://publicsuffix.org, MPL 2.0), embedded in the binary from `data/public-suffix/public_suffix_list.dat`, both its ICANN and private sections, wildcard and exception rules included. Refresh it by downloading the file again; its header carries its version.

The verdict is computed once, at reception, from the headers alone: it does not wait for the body. `reindex` never changes it.

## Routing

Only the mails of the Inbox at or above `folder.uid_screening_start` are screened: those below were there before NiceMailer (`import-initial.md`, section 5). `add_account` sets it to the `UIDNEXT` read by the test connection ([accounts.md](accounts.md)); for an account declared by hand in `accounts.toml`, the synchronization sets it to the `UIDNEXT` of the Inbox at its first synchronization. After the Inbox's `UIDVALIDITY` changes, it is set again to the new `UIDNEXT`: what is there counts as imported. While it is NULL, the account is not screened.

For a mail from address `A`, of domain `D`:

1. `A` banned, or `A` without rule and `D` (or a parent domain) banned: **Banned**;
2. `A` has a favorable rule (Inbox, Newsletter, Paperwork): its target if `A.auth_policy` accepts the verdict (`auto`: `pass`, or `none` and `unknown` while `A` was never seen authenticated; `required`: `pass`; `not_required`: any), else **Quarantine**, marked suspect;
3. else `D` has a favorable rule: its target if the verdict is `pass`, else **Quarantine**, suspect;
4. else, a reply in a known conversation (it cites a mail of the account kept in Inbox, Todo, Waiting, Archive or Sent): **Inbox**, without any decision on `A`, unless the verdict is `fail` (**Quarantine**, suspect);
5. else **Quarantine**.

Then `A.auth_seen` becomes 1 if the verdict is `pass`, and never goes back to 0.

- A mail from one of the user's addresses stays in the Inbox (rule `self`), whatever decision the address has, Banned included. They are the main address and the identities of **every** account of `accounts.toml`, not only of the account that receives, and the address of every archive account: a mail the user sends from one of their accounts to another is their own mail. The list is shared by the workers and read again at each pass, so an identity added to an account applies at once, without a new connection (`Syncer::apply`).
- A mail without `From`, or with several addresses in it, goes to Quarantine (rule `no_sender`).
- A suspect mail has `message.auth_suspect = 1`: the app warns of a probable spoofing.
- Each screened mail has a `screen` row in `audit_log` (source `core`): target folder, rule (`address`, `domain`, `conversation`, `self`, `no_sender`, `unknown_sender`), verdict, suspect.
- A mail screened out of the Inbox moves like any command, through the spool ([spool.md](spool.md)): the server moves it too, at once.
- A target folder the account does not have (an account declared by hand on a server without Quarantine or Newsletter; `add_account` creates them): the mail stays in the Inbox, and the `screen` row says `folder_missing`.

`MessageArrived` gives the folder the mail ends in.

## A conversation back from Todo or Waiting

A mail delivered to the Inbox (by any rule but `self`) in a conversation whose mails wait in Todo or Waiting brings them back to the Inbox: what is new is in the Inbox (`dossiers.md`, "Nouveau mail dans une conversation en Todo ou en Waiting"). The mails brought back get the keyword `$NmWasTodo` or `$NmWasWaiting`, on the server too (spool), so that the list shows "↩ Todo" and other Macs see it; the new mail gets none.

Nothing comes back for a mail filed elsewhere by the screening (Quarantine, Newsletter, Banned…), for the user's own mail, for a mail moved into the Inbox by another client, or for the mails already there at account creation.

The mark goes away with the next command on the conversation (`move_thread`: moving, deleting, junk), sent to the server too, and when the synchronization sees a marked mail leave the Inbox in another client.

## A mail found in Sent

A mail that appears in Sent after the folder's first synchronization was sent elsewhere (the phone, the webmail):

- each recipient (To, Cc, Bcc) without decision, other than the user's addresses (those of every account, as above), gets the decision `inbox` (`audit_log`: `screening`, source `imap`, reason `sent_to`), and their mails still in Quarantine go to the Inbox. An existing decision, or a decision the user removed, is kept;
- a reply (its `In-Reply-To` names a mail of the account outside Sent) files its conversation as the reply screen would have preselected: from the Inbox to Archive, or back to Waiting or Todo if it came back from there; from Todo to Archive; Waiting stays. The return marks go away.

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-store/src/auth.rs` (unit) | each line of the translation table; a fake header under the server's ignored; another `authserv-id` alone gives `unknown`; no `From`, several `From`, no id; comments, folding; organizational domains with the list's wildcard and exception rules and a private suffix; detection at 90 %, below, without any header |
| `crates/nm-store/tests/reception.rs` | the examples of `authentification-expediteur.md`, section 4; address and domain rules, a subdomain, Banned whatever the verdict; a reply in a known conversation, forged, from a banned sender, to a newsletter; the UID boundary and a suspended account; a conversation back from Waiting, marked, the mark removed by the next command; what brings nothing back; recipients of a mail found in Sent, the conversation of a reply filed; a mark removed when the mail leaves the Inbox elsewhere; the mail of another account of the user or of an archive account never screened, even banned, and no decision on those addresses from a mail in Sent |
| `crates/nm-engine/src/sync/mod.rs` (unit) | the user's addresses: those of every account and its identities, lowercase, once each |
| `crates/nm-daemon/tests/sync_rpc.rs`, `new_mail_is_screened_on_arrival` (`make imap-test`) | against Dovecot: the `authserv-id` detected, the mails already there untouched, an unknown sender to Quarantine on the server, a forged header beaten by the server's, a conversation back from Todo with its keyword on the server, a mail sent from the phone taking a sender out of Quarantine |

## Limits

- Internationalized domains are compared as written: an address in Unicode and a signature in punycode (`xn--…`) are not aligned.
- Microsoft 365 and Outlook.com reportedly write `Authentication-Results` without `authserv-id`: their mail would then be `unknown` (`authentification-expediteur.md`, open question 1).
- A server that does not remove incoming headers carrying its own id would let a forged header through, if the forger also knew the id: to check on the servers targeted (open question 2).
