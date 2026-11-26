//! Build script.
//!
//! Steam: the game links against `steam_api64.dll`, but loads it lazily
//! (/DELAYLOAD), so it starts fine without the DLL or without Steam; Steam
//! features switch on only when the DLL sits beside the game (see
//! `src/steam.rs`). For development builds it also copies the DLL next to
//! the executable, from where the `steamworks-sys` build put it.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !(target.contains("windows") && target.contains("msvc")) {
        return;
    }
    println!("cargo:rustc-link-arg=/DELAYLOAD:steam_api64.dll");
    println!("cargo:rustc-link-lib=delayimp");

    // OUT_DIR is target/<profile>/build/<crate>-<hash>/out (older cargo) or
    // target/<profile>/build/<crate>/<hash>/out (newer).
    let Ok(out) = std::env::var("OUT_DIR") else { return };
    let Some(build) = Path::new(&out).ancestors().find(|p| p.file_name().is_some_and(|n| n == "build")) else {
        return;
    };
    let Some(profile) = build.parent() else { return };
    if profile.join("steam_api64.dll").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(build) else { return };
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with("steamworks-sys") {
            continue;
        }
        // Either <dir>/out/steam_api64.dll or <dir>/<hash>/out/steam_api64.dll.
        let mut places = vec![e.path().join("out").join("steam_api64.dll")];
        if let Ok(hashes) = std::fs::read_dir(e.path()) {
            places.extend(hashes.flatten().map(|h| h.path().join("out").join("steam_api64.dll")));
        }
        if let Some(dll) = places.into_iter().find(|p| p.exists()) {
            let _ = std::fs::copy(&dll, profile.join("steam_api64.dll"));
            return;
        }
    }
}
