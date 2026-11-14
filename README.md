# bato-native

Shared code for the **native** (Rust + iced) flavor of Bato apps. `bato-template`'s
`app-client-native/` and every app scaffolded from it depend on this repo by git tag, so a
fix to text, scrolling, theme or a widget is made once and reaches an app when it bumps the tag.

| Path | What |
|---|---|
| `crates/bato-theme` | Hybrid tokens, contrast-safe accents, persisted settings (port of `theme.tsx`) |
| `crates/scroll-physics` | Chromium wheel/touchpad/touchscreen scrolling and BatoScroll autoscroll, toolkit-neutral |
| `crates/bato-kit` | The widget kit; re-exports `iced` (apps use iced only through `bato_kit::iced`) |
| `crates/bato-shell` | Platform services: window identity, settings, HTTP, portal dialogs, notifications, backend supervisor, updater |
| `examples/gallery` | The Showcase: every kit component, and the visual test bench |
| `patched/` | Patched upstream crates (normalised sources); regenerated from `patches/` by `scripts/regen-patched.sh` |
| `assets/` | Fonts (Inter, JetBrains Mono; OFL) and lucide icons, compiled in |
| `docs/` | Pixel spec, PoC brief, the port plan and its inventories, Electron reference shots |
| `poc/iced` | The proof of concept the kit is extracted from (removed once the kit replaces it) |
| `tools/` | Off-screen Hyprland output and screenshot helper for automated visual checks |

## Patched crates

| Crate | Why |
|---|---|
| `winit` (iced's fork) | Wayland `axis_value120` hi-res wheel deltas |
| `cosmic-text` 0.15.0 | FreeType light hinting, so text matches Chromium |
| `cryoglyph` 0.1.0 | Skia's gamma preblend in the glyph shader |
| `iced_wgpu` 0.14.0 | `COPY_SRC` surface usage, for the dialog backdrop blur |

Each exports `BATO_PATCH`, which `bato-kit` references: if a build ever resolves the unpatched
crates.io release instead (a `[patch]` that no longer applies is only a warning in Cargo),
compilation fails rather than silently losing the fix. Build with `--locked`.

## Using bato-native in an app

`[patch]` is honoured only in the root manifest, so each app carries the block itself. All six
`tag`s must match (`make native-bump TAG=…` in the template keeps them in step):

```toml
[dependencies]
bato-kit   = { git = "https://github.com/BatoRoy/bato-native", tag = "v0.1.0" }
bato-shell = { git = "https://github.com/BatoRoy/bato-native", tag = "v0.1.0" }

[patch.crates-io]
winit       = { git = "https://github.com/BatoRoy/bato-native", tag = "v0.1.0" }
cosmic-text = { git = "https://github.com/BatoRoy/bato-native", tag = "v0.1.0" }
cryoglyph   = { git = "https://github.com/BatoRoy/bato-native", tag = "v0.1.0" }
iced_wgpu   = { git = "https://github.com/BatoRoy/bato-native", tag = "v0.1.0" }
```

Working on the kit and an app together, without tagging: in the app, a git-ignored
`.cargo/config.toml` with

```toml
[patch."https://github.com/BatoRoy/bato-native"]
bato-kit   = { path = "/home/batoroy/Code/BatoApps/bato-native/crates/bato-kit" }
bato-shell = { path = "/home/batoroy/Code/BatoApps/bato-native/crates/bato-shell" }
```

## Develop

```sh
cargo test --workspace
cargo run -p gallery --release          # the Showcase
(cd poc/iced && cargo run --release)    # the proof of concept
```
