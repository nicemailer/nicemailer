//! DNS: the MX servers of a domain, to recognize the provider hosting an
//! address when an account is added (`docs/accounts.md`, "Finding the
//! servers"). `me@my-company.fr` whose MX is `aspmx.l.google.com` is a
//! Google mailbox, whatever the domain.
//!
//! The resolvers are the system's (`/etc/resolv.conf`, which macOS keeps
//! up to date with its primary resolvers). A VPN's "split DNS" resolvers,
//! known to macOS only for some domains, are not read: the public MX of a
//! mail domain is what is looked for, and it is the same through any
//! resolver.

use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

/// One MX record: lower `preference` first (RFC 5321, 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mx {
    pub preference: u16,
    /// The server's host name, lower case, without the final dot.
    pub host: String,
}

/// Why no MX could be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DnsError {
    /// The system's resolvers could not be read.
    #[error("cannot read the system's DNS settings: {0}")]
    Config(String),
    /// The domain does not exist.
    #[error("the domain {0} does not exist")]
    NoDomain(String),
    /// The resolvers did not answer, or failed.
    #[error("DNS lookup of {domain} failed: {reason}")]
    Lookup { domain: String, reason: String },
}

/// Time for each resolver to answer, and attempts: an account being added
/// waits for this, so a broken resolver must not hold it for long.
const TIMEOUT: Duration = Duration::from_secs(3);
const ATTEMPTS: usize = 2;

/// The MX records of `domain`, sorted by preference then name. A domain
/// that exists without MX gives an empty list (not an error): the caller
/// says so.
pub async fn mx(domain: &str) -> Result<Vec<Mx>, DnsError> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let mut builder = TokioResolver::builder_tokio().map_err(|e| DnsError::Config(e.to_string()))?;
    builder.options_mut().timeout = TIMEOUT;
    builder.options_mut().attempts = ATTEMPTS;
    let resolver = builder.build().map_err(|e| DnsError::Config(e.to_string()))?;
    // The final dot makes the name complete: without it, the search
    // domains of the network (`search corp.example`) would be tried too,
    // and `gmail.com.corp.example` could answer.
    match resolver.mx_lookup(format!("{domain}.")).await {
        Ok(lookup) => Ok(sorted(lookup.answers().iter().filter_map(|r| match &r.data {
            RData::MX(mx) => Some(Mx { preference: mx.preference, host: host_name(&mx.exchange.to_ascii()) }),
            _ => None,
        }))),
        Err(e) if e.is_nx_domain() => Err(DnsError::NoDomain(domain)),
        Err(e) if e.is_no_records_found() => Ok(Vec::new()),
        Err(e) => Err(DnsError::Lookup { domain, reason: e.to_string() }),
    }
}

/// A host name as DNS gives it (`ASPMX.L.Google.com.`): lower case,
/// without the final dot.
fn host_name(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// By preference, then by name (two servers of the same preference are
/// equivalent; the name only makes the order stable), without duplicates
/// and without the "null MX" (`.`, RFC 7505: the domain receives no mail).
fn sorted(records: impl Iterator<Item = Mx>) -> Vec<Mx> {
    let mut v: Vec<Mx> = records.filter(|m| !m.host.is_empty()).collect();
    v.sort_by(|a, b| a.preference.cmp(&b.preference).then_with(|| a.host.cmp(&b.host)));
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mx(preference: u16, host: &str) -> Mx {
        Mx { preference, host: host.into() }
    }

    #[test]
    fn records_are_sorted_by_preference_then_name() {
        let v = sorted(vec![mx(10, "b.example"), mx(5, "z.example"), mx(10, "a.example"), mx(5, "z.example")].into_iter());
        assert_eq!(v, vec![mx(5, "z.example"), mx(10, "a.example"), mx(10, "b.example")]);
    }

    /// Needs the network: `cargo test -p nm-proto dns -- --ignored`.
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn gmail_has_google_mx_and_a_made_up_domain_none() {
        let v = super::mx("gmail.com").await.unwrap();
        assert!(v.iter().all(|m| m.host.ends_with(".google.com")), "{v:?}");
        assert!(matches!(super::mx("no-such-domain-nicemailer.invalid").await, Err(DnsError::NoDomain(_))));
    }

    #[test]
    fn the_null_mx_is_dropped_and_names_are_normalized() {
        assert_eq!(host_name("ASPMX.L.Google.com."), "aspmx.l.google.com");
        assert_eq!(sorted(vec![mx(0, &host_name("."))].into_iter()), vec![]);
    }
}
