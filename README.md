# gpui-ce for Hydra

The [gpui-ce](https://crates.io/crates/gpui-ce) 0.3.0 crates from crates.io,
with the patches Hydra Neo needs. Hydra Neo uses them as a git dependency,
through `[patch.crates-io]`:

```toml
[patch.crates-io]
gpui-ce = { git = "https://github.com/hydralauncher/gpui-ce", rev = "<commit>" }
gpui_ce_elements = { git = "https://github.com/hydralauncher/gpui-ce", rev = "<commit>" }
# ... and gpui_ce_parley, gpui_ce_render, gpui_ce_windows, gpui_ce_wgpu
```

One folder per crate. What changed from the crates.io releases, and why, is
in [gpui-ce/HYDRA-PATCHES.md](gpui-ce/HYDRA-PATCHES.md) (and
`gpui-ce-wgpu.hydra.patch` for the Linux renderer).

To change a patch: commit here, then bump `rev` in Hydra Neo's `Cargo.toml`
(and `tools/render-check/Cargo.toml`).

The crates keep their upstream license (Apache-2.0, `LICENSE-APACHE` in each
crate folder).
