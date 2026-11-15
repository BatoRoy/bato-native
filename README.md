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
| `docs/` | Pixel spec, PoC brief, the port plan and its inventories, Electron reference shots, the archived iced PoC write-up (`docs/poc-iced`) |
| `tools/` | Off-screen Hyprland output and screenshot helper for automated visual checks |

## Patched crates

| Crate | Why |
|---|---|
| `winit` (iced's fork) | Wayland `axis_value120` hi-res wheel deltas, Wayland file drops, keypad keys that honour NumLock (`key_without_modifiers`, X11 and Wayland), clipboard reads through the file-drop `wl_data_device`, OS drags of files to other apps (`bato_drag`, `bato_kit::dnd::os_drag`; Wayland: a `wl_data_source` offering `text/uri-list` on that same data device; X11: an XDND source with an XI2 pointer grab), no panic when a pointer goes away (hot-unplug: events for a released `wl_pointer` are dropped) |
| `cosmic-text` 0.15.0 | FreeType light hinting, so text matches Chromium |
| `cryoglyph` 0.1.0 | Skia's gamma preblend in the glyph shader |
| `iced_wgpu` 0.14.0 | `COPY_SRC` surface usage, for the dialog backdrop blur |
| `smithay-clipboard` 0.7.3 | iced's clipboard thread survives a pointer going away (the same `unwrap` as winit's), so copy/paste keeps working |
| `iced_winit` 0.14.1 | Wayland paste: reads the clipboard through winit's data device. Hyprland sends the selection only to a client's first `wl_data_device` (winit's, for file drops), so window_clipboard's own never sees one; writes and the primary selection stay on window_clipboard |

`scripts/regen-patched.sh [out-dir]` rebuilds `patched/` from the upstream sources (sha256-checked
`.crate` files, iced's winit fork at a pinned commit), `patches/*.patch` and the tripwire blocks in
`patches/tripwires/`; a dry run into a temp dir must `diff -r` clean against `patched/`.

Each exports `BATO_PATCH`, which `bato-kit` references: if a build ever resolves the unpatched
crates.io release instead (a `[patch]` that no longer applies is only a warning in Cargo),
compilation fails rather than silently losing the fix. Build with `--locked`.

## Using bato-native in an app

`[patch]` is honoured only in the root manifest, so each app carries the block itself. Every
bato-native `tag` must match: the template's `app-client-native/Cargo.toml` has nine (kit,
shell, gallery and the six patches); `make native-bump TAG=…` keeps them in step and `make lint`
fails if they disagree:

```toml
[dependencies]
bato-kit   = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
bato-shell = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
gallery    = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1", optional = true }  # Examples + Scroll lab (`examples` feature)

[patch.crates-io]
winit       = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
cosmic-text = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
cryoglyph   = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
iced_wgpu   = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
iced_winit  = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
smithay-clipboard = { git = "https://github.com/BatoRoy/bato-native", tag = "v1.1.1" }
```

Working on the kit and an app together, without tagging: in the app, a git-ignored
`.cargo/config.toml` with

```toml
[patch."https://github.com/BatoRoy/bato-native"]
bato-kit   = { path = "/path/to/bato-native/crates/bato-kit" }
bato-shell = { path = "/path/to/bato-native/crates/bato-shell" }
gallery    = { path = "/path/to/bato-native/examples/gallery" }
```

## Develop

```sh
cargo test --workspace
cargo run -p gallery --release          # the Showcase
tools/compare.sh batch home scroll-lab options about --native target/release/gallery   # fidelity
```

## FreeType

`bato-kit` depends on `freetype-sys` with `bundled`, so the patched cosmic-text rasterizer links a
FreeType built from source (2.13.2) instead of the system one: the same hinting on every machine and
no `libfreetype` requirement. Measured against the Electron references (`tools/compare.sh`, mean
diff / px > 24): Home 0.184 / 4 817 bundled vs 0.184 / 4 842 system, Scroll lab 0.270 / 8 950 vs
0.270 / 8 968, rows 0.302 / 8 942 vs 0.305 / 9 029, Options 0.370 / 4 410 vs 0.369 / 4 400, About
0.290 / 734 both.

## Using the kit

```rust
use bato_kit::frame::{self, Action, Brand, Frame, NavItem};
use bato_kit::{Ico, prelude::*};

const BRAND: Brand = Brand { name: "Notes", tagline: "Plain text, fast.", version: "1.0.0",
                             slug: "batonotes", accent: "#2563eb", icon: Ico::FileText };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page { Notes, Trash }

let frame = Frame::new(BRAND, vec![
    NavItem::new(Page::Notes, Ico::FileText, "Notes"),
    NavItem::new(Page::Trash, Ico::Trash2, "Trash"),
], settings, Page::Notes);

// view(): the app draws its page, the frame does the rest.
let cx = frame.ctx();
let page = match frame.page() {
    Page::Notes => page(card(cx, "Notes").icon(Ico::FileText).body(/* … */), frame.wide(), [48.0, 32.0]),
    Page::Trash => page(empty_state(cx, "Trash is empty").icon(Ico::Trash2), frame.wide(), 24),
};
frame.view(page, Msg::Frame)
```

`frame.update(msg)` returns an `Action` for what needs the platform (persist settings, test a
server address, check for updates); `examples/gallery/src/main.rs` is the complete wiring with
bato-shell.
