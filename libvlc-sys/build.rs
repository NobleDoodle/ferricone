// Links the pinned libVLC 4 from vendor/vlc (scripts/build-libvlc.sh),
// or from the prefix in FERRICONE_LIBVLC_DIR.
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FERRICONE_LIBVLC_DIR");
    let prefix = match env::var_os("FERRICONE_LIBVLC_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../vendor/vlc"),
    };
    let lib = prefix
        .join("lib")
        .canonicalize()
        .expect("libVLC not found: run scripts/build-libvlc.sh or set FERRICONE_LIBVLC_DIR");

    println!("cargo:rustc-link-search=native={}", lib.display());
    // By file name, the pinned ABI: newer rustc searches its sysroot (/usr/lib
    // on Arch) first, where plain `-lvlc` would find a system libVLC 3.
    println!("cargo:rustc-link-lib=dylib:+verbatim=libvlc.so.12");
    // Reaches the app's build script as DEP_VLC_LIB.
    println!("cargo:lib={}", lib.display());
}
