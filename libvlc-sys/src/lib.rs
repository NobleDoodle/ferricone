//! Raw FFI for libVLC 4, generated from the pinned headers by
//! `scripts/gen-libvlc-bindings.sh`. Only `src/vlc.rs` in the app uses it.

#![allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    clippy::all
)]

include!("bindings.rs");
