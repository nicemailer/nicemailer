# Mail providers

Where `nicemailer account add` finds the IMAP and SMTP servers of an address when `--host` is not given ([docs/accounts.md](../../docs/accounts.md), "Finding the servers"). Both sources are built into the core when it is compiled.

| File | What | Written by |
| --- | --- | --- |
| `thunderbird.json` | Thunderbird's ISP database ([thunderbird/autoconfig](https://github.com/thunderbird/autoconfig), MPL 2.0), converted: IMAP and SMTP servers only, encrypted only | `make autoconfig` (`scripts/update-autoconfig.py`); never by hand |
| `local/*.toml` | Our own providers, one file per provider | by hand |

A provider of `local/` wins over Thunderbird's for the domains it names.

## A local provider

```toml
name = "Infomaniak"                  # shown to the user
domains = ["ik.me", "ikmail.com"]    # domains of the addresses it hosts
mx = ["infomaniak.ch"]               # optional: domains of its MX servers

[imap]
host = "mail.infomaniak.com"         # %EMAILDOMAIN% is replaced by the address's domain
port = 993
security = "tls"                     # tls or starttls
username = "%EMAILADDRESS%"          # or %EMAILLOCALPART%, or a mix

[smtp]                               # optional
host = "mail.infomaniak.com"
port = 587
security = "starttls"
username = "%EMAILADDRESS%"
```

`mx` lets a domain of the user's own (`me@my-company.fr`) be recognized: if the MX of `my-company.fr` is under one of these domains (`mta-gw.infomaniak.ch`), the provider's servers are used. Thunderbird's file has no such field: like Thunderbird, the core then looks up the MX's domain among the providers' domains (Google lists `google.com` for this).

Several servers of a kind can be given with `[[imap]]` / `[[smtp]]`; the first one is used. After a change, rebuild the core (`make core`); the unit tests of `nm-engine` (`providers.rs`) read every file.
