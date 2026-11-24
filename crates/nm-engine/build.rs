//! Build script of nm-engine: embeds our own mail providers
//! (`data/providers/local/*.toml`, docs in `data/providers/README.md`) into
//! the core.
//!
//! Rust note: `build.rs` runs before the crate is compiled. Here it writes
//! `local_providers.rs` in `OUT_DIR` (a folder Cargo gives it): one
//! `include_str!` per file found, so that adding a provider needs no change
//! to the code. `include_str!` itself cannot list a folder, hence this
//! script. `cargo:rerun-if-changed` on the folder makes Cargo run it again
//! when a file is added, removed or changed.
//!
//! It also embeds the Entra application id ("client ID") of the OAuth2
//! sign-in (`specifications/oauth2.md`, section 4): the
//! `NICEMAILER_MICROSOFT_CLIENT_ID` environment variable becomes the
//! `NICEMAILER_COMPILED_CLIENT_ID` compile-time variable read by
//! `oauth2.rs`. The id is not a secret — a public client sends it in every
//! authorization URL, visible in the browser — so compiling it in is safe;
//! what must never ship is a client *secret*, and the flow uses none
//! (PKCE and the loopback redirect instead, section 5). The value is
//! checked here: a malformed one fails the build. `scripts/make-bundle.sh
//! --release` refuses to run without the variable, so a distributed binary
//! always embeds a real id and the first install needs no configuration.
//! Without it (a development build), the zero GUID is embedded: no id is
//! available, and `oauth.microsoft_client_id` in `config.toml` replaces it.

use std::path::PathBuf;

/// The zero GUID: "no application id" in a development build.
const ZERO_CLIENT_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Whether `s` is a non-zero GUID: 8-4-4-4-12 hexadecimal digits, hyphens
/// included, not all zeros — the same rule as `config::is_client_id` (the
/// crate is not compiled yet when this script runs, so the check is
/// duplicated here on purpose).
fn is_client_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let hyphens_ok = [8, 13, 18, 23].iter().all(|&i| bytes[i] == b'-');
    let digits_ok = bytes.iter().enumerate().all(|(i, &b)| [8, 13, 18, 23].contains(&i) || b.is_ascii_hexdigit());
    hyphens_ok && digits_ok && s != ZERO_CLIENT_ID
}

fn main() {
    println!("cargo:rerun-if-env-changed=NICEMAILER_MICROSOFT_CLIENT_ID");
    let client_id = match std::env::var("NICEMAILER_MICROSOFT_CLIENT_ID") {
        Ok(v) if !v.is_empty() => {
            assert!(
                is_client_id(&v),
                "NICEMAILER_MICROSOFT_CLIENT_ID must be a non-zero GUID (8-4-4-4-12 hexadecimal digits, hyphens included), got {v:?}"
            );
            v
        }
        _ => ZERO_CLIENT_ID.to_string(),
    };
    println!("cargo:rustc-env=NICEMAILER_COMPILED_CLIENT_ID={client_id}");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = manifest.join("../../data/providers/local").canonicalize().expect("data/providers/local is missing");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("cannot list data/providers/local")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    // Sorted: the same build gives the same order on every Mac.
    files.sort();
    let mut out = String::from("/// `(file stem, contents)` of every `data/providers/local/*.toml`.\n");
    out.push_str("pub const LOCAL_PROVIDERS: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let stem = f.file_stem().unwrap().to_str().expect("provider file names are UTF-8");
        out.push_str(&format!("    ({stem:?}, include_str!({:?})),\n", f.display().to_string()));
    }
    out.push_str("];\n");
    let target = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("local_providers.rs");
    std::fs::write(target, out).expect("cannot write local_providers.rs");
}
