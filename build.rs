fn main() {
    slint_build::compile("ui/app.slint").expect("compiling ui/app.slint");

    // Development runs load the vendored libVLC. DT_RPATH (not RUNPATH) also
    // covers the private libraries VLC's plugins load from lib/vlc/.
    // Packages set FERRICONE_RPATH ($ORIGIN-relative, packaging/bundle.sh).
    println!("cargo:rerun-if-env-changed=FERRICONE_RPATH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        if let Ok(rpath) = std::env::var("FERRICONE_RPATH") {
            println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags,-rpath,{rpath}");
        } else if let Ok(lib) = std::env::var("DEP_VLC_LIB") {
            println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags,-rpath,{lib},-rpath,{lib}/vlc");
        }
    }
}
