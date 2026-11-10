//! Libraries the bundle carries for systems that lack them, in `lib/fallback`
//! next to `bin/`. winit loads them on X11 by soname; a bare Ubuntu, like the
//! AppImage catalog's test machine, has no libxkbcommon-x11, and winit panics.
//!
//! Each one is loaded by its full path only when the system can't load it by
//! its soname, so a system copy is never shadowed. Once loaded, the later
//! `dlopen("libxkbcommon-x11.so.0")` finds it by its soname. The packages
//! that declare dependencies (.deb, .rpm, Arch) never need them.

use std::ffi::CString;

/// In load order: each one's dependencies come before it.
const LIBS: &[&str] = &["libxcb-xkb.so.1", "libxkbcommon-x11.so.0"];

pub fn load() {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("lib/fallback")))
    else {
        return;
    };
    if !dir.is_dir() {
        return;
    }
    for name in LIBS {
        if open(name) {
            continue;
        }
        let path = dir.join(name);
        if path.is_file() && !open(&path.to_string_lossy()) {
            eprintln!("ferricone: couldn't load {}", path.display());
        }
    }
}

/// dlopen, kept open for the whole run: RTLD_GLOBAL so that what loads
/// after it can use its symbols.
fn open(name: &str) -> bool {
    let Ok(name) = CString::new(name) else { return false };
    // SAFETY: a valid C string; the handle is never closed.
    !unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) }.is_null()
}
