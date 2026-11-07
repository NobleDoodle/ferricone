# winit 0.30.13, with one addition

An unmodified copy of winit 0.30.13 from crates.io, except for one accessor:
`WindowExtWayland::xdg_surface()`, next to the existing `xdg_toplevel()`.
An `xdg_popup` needs its parent's `xdg_surface`, and winit doesn't expose it
(`src/popup.rs` in Ferricone). Changed: `src/platform/wayland.rs` and
`src/platform_impl/linux/wayland/window/mod.rs`.
