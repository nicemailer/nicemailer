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

use std::path::PathBuf;

fn main() {
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
