# egui-wgpu 0.36.2, patched for Craft Anything

This is a copy of `egui-wgpu` 0.36.2 (MIT OR Apache-2.0, see `LICENSE-*`) with one change, used through
`[patch.crates-io]` in the workspace `Cargo.toml`.

## The change

When the window draws with WARP (Windows' software renderer: the DX12 adapter of type `Cpu`), egui's
shader filters textures by hand and never reads a sampler.

- `src/lib.rs`: `RenderState::create` turns on `predictable_texture_filtering` for WARP (`is_warp`).
- `src/egui.wgsl`: the choice between hardware and manual filtering is the pipeline constant
  `HARDWARE_FILTERING` instead of a uniform, so with manual filtering the compiled shader contains no
  sampler access at all.
- `src/renderer.rs`: the fragment stage sets `HARDWARE_FILTERING` from `predictable_texture_filtering`.
  Manual filtering is bilinear and clamps to the edge, so with it on, a second pipeline with hardware
  filtering draws the textures it can't (`Nearest`, a wrap mode other than clamp, mipmaps, a sampler
  of the texture's own, or one swapped in later): `manual_filtering_draws`, `replaced_samplers`.
  Craft Anything's own textures are all linear and clamped, so on WARP they all take the fast pipeline.

On every other adapter nothing changes: hardware filtering, as upstream. Everything else in this
directory is upstream's code as published, including its `unsafe` (window handles for surfaces, and
macOS's Metal layer), which the workspace's `unsafe_code = "forbid"` doesn't reach: like
`vendor/wgpu-hal`, this is a dependency kept as published, not the project's own code. Upstream
issues in the unchanged code are left to upstream.

## Why

wgpu 25 and later bind DX12 samplers through one sampler heap: each shader reads its sampler's index from
a buffer and picks the sampler from the heap. On WARP a texture read through a sampler is about ten times
slower than the same read with `textureLoad`; the heap lookup is the likely reason (not confirmed inside
WARP). Measured on a Windows 11 VM without a GPU (QEMU, 6 vCPUs, desktop over RDP, window 2294×1264,
release build):

| | upstream | patched |
|---|---|---|
| Empty egui window, redrawn continuously | 0.25 s, 5.3 G cycles a frame | 0.03 s (the display's rate), 0.4 G cycles |
| Craft Anything, mouse moving over it | 2.6 frames/s, 5 of 6 cores busy | 13 frames/s, 2.6 cores |

A sampled profile of the upstream build put 99% of the process's time in WARP's rasterizer and the code it
compiles. Screenshots of the app drawn with egui's hardware filtering and with its manual filtering differed
by at most 1 level in 34 pixels.

## Removing it

Delete this directory and its `[patch.crates-io]` entry once egui-wgpu leaves the sampler out of the
pipeline when it filters by hand, or wgpu's DX12 backend samples quickly on WARP.

Measure before and after with the same work: a release build of `main` and of the candidate, each drawing
with WARP (on a machine with a graphics card: `WGPU_BACKEND=dx12 CRAFT_ANYTHING_RENDERER=software`, and
`desktop.log` says `drawing with … device_type: Cpu, … backend: Dx12`), the window maximized on the same
screen, nothing else running. Start it, wait 10 s, then (1) read the process's cycles
(`QueryProcessCycleTime`) just before one click on an empty spot and once it stops rising, and (2) move
the mouse over the window continuously for 8 s and read cycles over that time. On the VM above, with the
0.6.0 release against this patch on `main` at `88d1d9c`: (1) 20 G cycles against 1.8 G, (2) the table's
second row.
