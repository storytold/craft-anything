# winit 0.30.13, patched for Craft Anything

This is a copy of `winit` 0.30.13 (Apache-2.0, see `LICENSE`) with one change, used through
`[patch.crates-io]` in the workspace `Cargo.toml`.

## The change

`src/platform_impl/windows/event_loop.rs`, `WM_DPICHANGED`: on Windows 11 the window is placed at the
rectangle Windows suggests. winit's own adjustment, which shifts the window onto the monitor whose
scaling it now has, runs only on Windows 10 (build below 22000). `dark_mode.rs` makes its Windows
build number visible to the event loop for this.

This is upstream's fix (rust-windowing/winit#4341, commit 488c036), backported. Upstream merged it
for 0.31 only.

## Why

On Windows 11 the adjustment found the window "not on" the new monitor and moved it back onto the
old one, which changed the scaling back again. A window dragged towards a screen with different
scaling stopped about half-way and couldn't cross. Moving it with Alt+Space → Move or with the arrow
keys did the same.

- Craft Anything issue #1814 (a laptop screen and an external one with different scaling).
- Upstream: rust-windowing/winit#4041, fixed by #4341 (via #4119).

## Removing it

Delete this directory and the `[patch.crates-io]` entry once eframe uses a winit with #4341 (0.31 or a
0.30 release that backports it).
