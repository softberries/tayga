//! With `embed-ui` on, warn (never fail) when `ui/dist/index.html` is missing.
//!
//! The derive in `src/spa.rs` uses `allow_missing`, so a missing `ui/dist`
//! compiles to an empty asset set and the server serves a placeholder page.
//! A hard failure would break `cargo test` on machines without Node. Docker
//! builds run `npm run build` first, so release images always embed the app.

use std::path::Path;

// Keep in sync with the `#[folder]` path in src/spa.rs.
fn main() {
    println!("cargo:rerun-if-changed=../../ui/dist");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_EMBED_UI");
    if std::env::var_os("CARGO_FEATURE_EMBED_UI").is_none() {
        return;
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let index = Path::new(&manifest).join("../../ui/dist/index.html");
    if !index.is_file() {
        println!(
            "cargo:warning=ui/dist/index.html not found: the web app is NOT embedded and tayga-api will serve a placeholder page. Run `npm --prefix ui run build` and rebuild."
        );
    }
}
