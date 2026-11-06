//! The authentication verdict of a received mail
//! (`authentification-expediteur.md`, section 3).
//!
//! The `From` header is free text: anyone can write `support@bank.example`
//! there. The receiving server checks SPF, DKIM and DMARC and writes what it
//! found in an `Authentication-Results` header (RFC 8601), at the top of the
//! message. NiceMailer trusts only the header of the account's own server,
//! recognized by its `authserv-id`, and turns it into a verdict: `pass`,
//! `fail`, `none` or `unknown`.
//!
//! Everything here is pure: header values in, a verdict out. No database.
//!
//! # Which header
//!
//! A sender can write a fake `Authentication-Results: dmarc=pass` into its
//! mail. Only the headers whose `authserv-id` is the account's are kept, and
//! of those the first from the top, the last one added: a conforming server
//! removes the incoming headers that carry its own id (RFC 8601, section
//! 5), so the first one with its id is its own.
//!
//! # From results to a verdict
//!
//! 1. `dmarc=pass` → `pass`; `dmarc=fail` → `fail`;
//! 2. `dmarc=temperror` or `permerror` → `unknown`;
//! 3. no DMARC result, or `dmarc=none`: `dkim=pass` with a `header.d`
//!    aligned on the domain of `From` → `pass`, else `none`.
//!
//! "Aligned" is DMARC's relaxed alignment: the same organizational domain,
//! the registrable domain under a public suffix (`mail.example.fr` and
//! `example.fr`; but `a.co.uk` and `b.co.uk` are not), found with the Public
//! Suffix List embedded in the binary (`data/public-suffix/`).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use nm_core::AuthResult;

/// The Public Suffix List (https://publicsuffix.org, MPL 2.0), as published
/// on the date in its header; refreshed by hand, like the list of public
/// mail domains (`domaines-publics.md`).
///
/// Rust note: `include_str!` copies the file into the binary at compile
/// time: nothing is read from disk at run time.
const PUBLIC_SUFFIX_LIST: &str = include_str!("../../../data/public-suffix/public_suffix_list.dat");

/// The rules of the list: plain (`co.uk`), wildcard (`*.ck`: every label
/// under `ck` is a suffix) and exceptions (`!www.ck`: not a suffix).
struct Suffixes {
    plain: HashSet<&'static str>,
    wildcard: HashSet<&'static str>,
    exception: HashSet<&'static str>,
}

/// Parsed once, at first use (about 10,000 rules).
///
/// Rust note: `OnceLock` holds a value computed the first time it is asked
/// for, by whichever thread comes first; the others wait and share it.
fn suffixes() -> &'static Suffixes {
    static S: OnceLock<Suffixes> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = Suffixes { plain: HashSet::new(), wildcard: HashSet::new(), exception: HashSet::new() };
        for line in PUBLIC_SUFFIX_LIST.lines() {
            // A rule is the first word of its line; comments start with //.
            let Some(rule) = line.split_whitespace().next() else { continue };
            if rule.starts_with("//") {
                continue;
            }
            if let Some(r) = rule.strip_prefix("*.") {
                s.wildcard.insert(r);
            } else if let Some(r) = rule.strip_prefix('!') {
                s.exception.insert(r);
            } else {
                s.plain.insert(rule);
            }
        }
        s
    })
}

/// The organizational domain of `domain` (lowercase): its public suffix and
/// one label more. A domain that is itself a public suffix is its own.
///
/// The list's algorithm: the longest matching rule wins, an exception wins
/// over everything, and a domain no rule matches has the suffix of its
/// last label (`*`, the implicit rule).
pub fn organizational_domain(domain: &str) -> String {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return domain;
    }
    let s = suffixes();
    // Length, in labels, of the public suffix.
    let mut suffix = 1;
    for start in 0..labels.len() {
        let candidate = labels[start..].join(".");
        if s.exception.contains(candidate.as_str()) {
            // `!www.ck`: the suffix is `ck`, one label shorter.
            suffix = labels.len() - start - 1;
            break;
        }
        let n = labels.len() - start;
        let parent = labels[start + 1..].join(".");
        if s.plain.contains(candidate.as_str()) || (start + 1 < labels.len() && s.wildcard.contains(parent.as_str())) {
            suffix = suffix.max(n);
        }
    }
    if suffix >= labels.len() {
        return domain;
    }
    labels[labels.len() - suffix - 1..].join(".")
}

/// Relaxed alignment: the two domains have the same organizational domain.
pub fn aligned(a: &str, b: &str) -> bool {
    organizational_domain(a) == organizational_domain(b)
}

/// One `Authentication-Results` header, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthHeader {
    /// Lowercase.
    pub authserv_id: String,
    pub results: Vec<MethodResult>,
}

/// One result of a header: `dkim=pass header.d=example.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodResult {
    /// Lowercase: `dmarc`, `dkim`, `spf`…
    pub method: String,
    /// Lowercase: `pass`, `fail`, `none`…
    pub result: String,
    /// `ptype.property` → value, keys lowercase (`header.d`, `header.from`).
    pub props: HashMap<String, String>,
}

/// Removes the comments of a header value (`(…)`, nested, RFC 5322) and
/// unfolds it, leaving quoted strings as they are.
fn strip_comments(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for c in v.chars() {
        if escaped {
            if depth == 0 {
                out.push(c);
            }
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                escaped = true;
                if depth == 0 {
                    out.push(c);
                }
            }
            '"' if depth == 0 => {
                quoted = !quoted;
                out.push(c);
            }
            '(' if !quoted => depth += 1,
            ')' if !quoted && depth > 0 => depth -= 1,
            '\r' | '\n' => out.push(' '),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Reads one header value (RFC 8601, section 2.2):
/// `authserv-id [version] *( ";" method[/version] "=" result [reason] *(ptype "." prop "=" value) )`,
/// or `authserv-id; none`. `None` when it has no `authserv-id`.
pub fn parse_header(value: &str) -> Option<AuthHeader> {
    let clean = strip_comments(value);
    let mut parts = clean.split(';');
    let id = parts.next()?.split_whitespace().next()?.trim_matches('"').to_ascii_lowercase();
    if id.is_empty() {
        return None;
    }
    let mut results = Vec::new();
    for part in parts {
        let mut words = part.split_whitespace();
        let Some(first) = words.next() else { continue };
        let Some((method, result)) = first.split_once('=') else { continue };
        let method = method.split('/').next().unwrap_or(method).to_ascii_lowercase();
        let mut props = HashMap::new();
        for w in words {
            if let Some((k, v)) = w.split_once('=')
                && k.contains('.')
            {
                props.insert(k.to_ascii_lowercase(), v.trim_matches('"').to_string());
            }
        }
        results.push(MethodResult { method, result: result.trim_matches('"').to_ascii_lowercase(), props });
    }
    Some(AuthHeader { authserv_id: id, results })
}

/// The `authserv-id` of the first header (the top one, added last), for
/// the detection of the account's id.
pub fn first_authserv_id(headers: &[String]) -> Option<String> {
    headers.first().and_then(|h| parse_header(h)).map(|h| h.authserv_id)
}

/// The verdict of a mail, and its source (`server`, or `None` without any
/// trusted header).
///
/// `headers`: the values of its `Authentication-Results` headers, top first;
/// `authserv_id`: the account's (the file's, else the detected one); `from`:
/// the address of `From`, and `from_count` how many `From` holds. A mail
/// without `From`, or with several, is `unknown`: no address rule can apply.
pub fn verdict(headers: &[String], authserv_id: Option<&str>, from: Option<&str>, from_count: usize) -> (AuthResult, Option<&'static str>) {
    let (Some(id), Some(from)) = (authserv_id, from) else { return (AuthResult::Unknown, None) };
    if from_count != 1 {
        return (AuthResult::Unknown, None);
    }
    let Some(from_domain) = from.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()) else { return (AuthResult::Unknown, None) };
    let id = id.to_ascii_lowercase();
    let Some(h) = headers.iter().filter_map(|v| parse_header(v)).find(|h| h.authserv_id == id) else {
        return (AuthResult::Unknown, None);
    };
    let dmarc = h.results.iter().find(|r| r.method == "dmarc").map(|r| r.result.as_str());
    let verdict = match dmarc {
        Some("pass") => AuthResult::Pass,
        Some("fail") => AuthResult::Fail,
        Some("temperror") | Some("permerror") => AuthResult::Unknown,
        _ => {
            let dkim_aligned = h.results.iter().any(|r| {
                r.method == "dkim"
                    && r.result == "pass"
                    && r.props.get("header.d").or_else(|| r.props.get("header.i")).is_some_and(|d| {
                        aligned(d.rsplit_once('@').map_or(d.as_str(), |(_, d)| d), &from_domain)
                    })
            });
            if dkim_aligned { AuthResult::Pass } else { AuthResult::None }
        }
    };
    (verdict, Some("server"))
}

/// The account's `authserv-id`, detected from the first header of its
/// recent Inbox mails (`authentification-expediteur.md`, "Détection"): the
/// id that tops at least 90 % of them. `firsts` holds the first id of each
/// mail looked at, `None` for a mail without any header.
pub fn detect_authserv_id(firsts: &[Option<String>]) -> Option<String> {
    if firsts.is_empty() {
        return None;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for id in firsts.iter().flatten() {
        *counts.entry(id.as_str()).or_default() += 1;
    }
    let (best, n) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    // At least 90 %: n / total >= 0.9, in integers.
    (n * 10 >= firsts.len() * 9).then(|| best.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organizational_domains_follow_the_public_suffix_list() {
        assert_eq!(organizational_domain("mail.exemple.fr"), "exemple.fr");
        assert_eq!(organizational_domain("exemple.fr"), "exemple.fr");
        assert_eq!(organizational_domain("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(organizational_domain("co.uk"), "co.uk");
        // A wildcard rule (`*.ck`) and its exception (`!www.ck`).
        assert_eq!(organizational_domain("a.b.foo.ck"), "b.foo.ck");
        assert_eq!(organizational_domain("mail.www.ck"), "www.ck");
        // A private suffix: two blogs are two owners.
        assert_eq!(organizational_domain("alice.blogspot.com"), "alice.blogspot.com");
        // No rule: the last label is the suffix.
        assert_eq!(organizational_domain("x.y.unknowntld"), "y.unknowntld");
        assert!(aligned("mail.exemple.fr", "EXEMPLE.fr"));
        assert!(!aligned("a.co.uk", "b.co.uk"));
    }

    #[test]
    fn a_header_with_comments_and_folding_is_read() {
        let h = parse_header(
            "mx.infomaniak.ch (version 1);\r\n\tdkim=pass (2048-bit key) header.d=exemple.fr header.i=@exemple.fr;\r\n\
             spf=pass (sender ok) smtp.mailfrom=exemple.fr;\r\n\tdmarc=pass (p=reject) header.from=exemple.fr",
        )
        .unwrap();
        assert_eq!(h.authserv_id, "mx.infomaniak.ch");
        assert_eq!(h.results.len(), 3);
        assert_eq!(h.results[0].method, "dkim");
        assert_eq!(h.results[0].props["header.d"], "exemple.fr");
        assert_eq!(h.results[2].result, "pass");
        assert_eq!(parse_header("mx.example.org; none").unwrap().results.len(), 0);
        assert!(parse_header("   ").is_none());
    }

    fn v(headers: &[&str], from: &str) -> AuthResult {
        let headers: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
        verdict(&headers, Some("mx.ours.example"), Some(from), 1).0
    }

    #[test]
    fn each_line_of_the_translation_table() {
        let from = "jean@exemple.fr";
        assert_eq!(v(&["mx.ours.example; dmarc=pass header.from=exemple.fr"], from), AuthResult::Pass);
        assert_eq!(v(&["mx.ours.example; dmarc=fail header.from=exemple.fr"], from), AuthResult::Fail);
        assert_eq!(v(&["mx.ours.example; dmarc=temperror"], from), AuthResult::Unknown);
        assert_eq!(v(&["mx.ours.example; dmarc=permerror"], from), AuthResult::Unknown);
        // Without DMARC: an aligned DKIM signature, a subdomain included.
        assert_eq!(v(&["mx.ours.example; dkim=pass header.d=mail.exemple.fr"], from), AuthResult::Pass);
        assert_eq!(v(&["mx.ours.example; dmarc=none; dkim=pass header.d=exemple.fr"], from), AuthResult::Pass);
        // A signature of another domain proves nothing about this one.
        assert_eq!(v(&["mx.ours.example; dkim=pass header.d=mailer.example.com"], from), AuthResult::None);
        assert_eq!(v(&["mx.ours.example; dkim=fail header.d=exemple.fr"], from), AuthResult::None);
        assert_eq!(v(&["mx.ours.example; spf=pass smtp.mailfrom=exemple.fr"], from), AuthResult::None);
        // A public suffix is not an organization: example.co.uk is not
        // aligned on other.co.uk.
        assert_eq!(v(&["mx.ours.example; dkim=pass header.d=other.co.uk"], "a@example.co.uk"), AuthResult::None);
    }

    #[test]
    fn a_header_written_by_the_sender_is_ignored() {
        // The sender added a fake `dmarc=pass` under the server's header.
        let headers = [
            "mx.ours.example; dmarc=fail header.from=bank.example",
            "mx.ours.example; dmarc=pass header.from=bank.example",
        ];
        assert_eq!(v(&headers, "support@bank.example"), AuthResult::Fail);
        // Another authserv-id only: no trusted header.
        let (r, source) = verdict(&["evil.example; dmarc=pass".to_string()], Some("mx.ours.example"), Some("a@b.example"), 1);
        assert_eq!((r, source), (AuthResult::Unknown, None));
    }

    #[test]
    fn no_id_no_from_or_several_from_is_unknown() {
        let h = vec!["mx.ours.example; dmarc=pass".to_string()];
        assert_eq!(verdict(&h, None, Some("a@b.example"), 1).0, AuthResult::Unknown);
        assert_eq!(verdict(&h, Some("mx.ours.example"), None, 0).0, AuthResult::Unknown);
        assert_eq!(verdict(&h, Some("mx.ours.example"), Some("a@b.example"), 2).0, AuthResult::Unknown);
        // The id is compared without case.
        assert_eq!(verdict(&h, Some("MX.Ours.Example"), Some("a@b.example"), 1), (AuthResult::Pass, Some("server")));
    }

    #[test]
    fn the_id_of_the_account_is_detected_at_ninety_percent() {
        let id = |s: &str| Some(s.to_string());
        let mut firsts: Vec<Option<String>> = vec![id("mx.ours.example"); 45];
        firsts.extend(vec![id("other.example"); 5]);
        assert_eq!(detect_authserv_id(&firsts), Some("mx.ours.example".into()), "45 of 50");
        firsts[0] = None;
        assert_eq!(detect_authserv_id(&firsts), None, "44 of 50: below 90 %");
        assert_eq!(detect_authserv_id(&[None, None]), None, "no header at all");
        assert_eq!(detect_authserv_id(&[]), None);
    }
}
