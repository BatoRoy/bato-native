# tools

| Tool | What it does |
|---|---|
| `offscreen.sh check` | Exit 0 only if the off-screen output `BATOPOC` is up. Run `up`/`down` only if you own the desktop session. |
| `shot.sh <pid> <out.png>` | Screenshot a window, refusing any window that isn't on `BATOPOC`. |
| `compare.sh …` | The **fidelity harness**: builds and runs `compare/` (below). |
| `nested.sh …` | A **hidden nested session** with real input, for what the off-screen output can't test (below). |

## Fidelity harness (`tools/compare`)

`compare` renders a Showcase card in the native gallery and in Electron with the same `BATO_VIEW`,
both 1280×800 on the off-screen output, and reports how far apart they are. It writes a diff image and
a side-by-side image. Card ids are those of [`docs/CARDS.md`](../docs/CARDS.md): 7 tabs and 32 cards,
39 views in all.

### How a batch agent checks its cards

```sh
nice cargo build --release -p gallery                  # your native gallery build
tools/offscreen.sh check                               # must print "offscreen: ok"
tools/compare.sh batch inputs/text-fields inputs/specialized --native target/release/gallery
tools/compare.sh batch data --native target/release/gallery     # a whole tab: tab + its cards
tools/compare.sh one data/charts --native target/release/gallery
```

```
id                           mean     >8 %   px>24Σ  bbox (x0,y0–x1,y1)     notes
home                        0.184    0.51%     4842  16,16–1148,777         ref:stored native:cursor
```

That row is the sanity check: `tools/compare.sh one home --native target/release/gallery`.
The archived PoC (docs/poc-iced/README.md) reports 0.18 and 4 843 for the same view. Batch mode also writes
`target/compare/summary.tsv`.

Then open `target/compare/<tab>-<card>.side.png`, which shows **Electron | native | diff**:

- **mean**: the mean absolute difference per RGB channel, on a 0–255 scale. The iced PoC sits at 0.18–0.37
  on its views. Text anti-aliasing noise alone gives about 0.2–0.4. A wrong colour, size or position
  quickly pushes it past 1.
- **>8 %**: the share of pixels whose largest channel difference is more than 8 (set with `--threshold`).
- **px>24Σ**: the number of pixels whose summed RGB difference is more than 24. This is the PoC README's
  second column, so you can compare numbers directly.
- **bbox**: the bounding box of the >8 pixels. If it is small, the differences are local. If it spans the
  card, look at the diff image.
- **diff image** colours:
  - red: differs by more than the threshold;
  - faint amber: below the threshold (anti-aliasing noise);
  - grey: identical;
  - blue: masked;
  - cyan rectangle: the bbox.

  Red *shapes*, such as a whole edge or a text line shifted by a pixel, are real layout bugs. Red
  speckle on glyph edges is rasterizer noise.
- **notes**: where the reference came from, plus any capture flags:

  | Note | Meaning |
  |---|---|
  | `ref:stored` | The stored PNG was used. |
  | `ref:captured (…)` | Electron was captured just now. |
  | `native:cursor` | The cursor box was masked. |
  | `unstable` | The view was still animating after 8 s. |
  | `overlap:<class>(pid)` | Another window covered ours. |
  | `size …` | The two images differ in size. |

`--fail-above 0.5` makes the exit status 1 when any mean is above 0.5, which is handy in a script.
`compare list` prints every id with its `BATO_VIEW`. `compare shot <id> --native <bin> -o x.png` only
captures the native window. `compare diff a.png b.png` compares two existing images.

### What the native client must honour

The harness launches the native binary with the following environment:

| Variable | Value |
|---|---|
| `BATO_VIEW` | e.g. `view=examples&tab=data&card=charts`. Same keys and scroll rule as Electron: the card's top sits 24 px below the top of the content area, clamped to the scroll range. |
| `BATO_APP_ID` | `batopoc-compare` (change it with `--app-id batopoc-…`). The window rule routes only `^batopoc-.*` off-screen. A window that maps anywhere else is killed at once and reported. |
| `XDG_CONFIG_HOME` | A fresh private directory, so the settings are at their defaults: dark, brand accent, 100 %, comfortable width. |

Use `--env K=V` and `--arg X` to pass anything else, e.g. `--env BATO_BACKEND_URL=http://127.0.0.1:9`.

The window must be 1280×800. The off-screen rule pins it to that size anyway.

### Electron references

The references live in `docs/reference/examples/`:

- `<tab>.png`: the tab scrolled to the top;
- `<tab>-<card>.png`: the card at the top;
- `full/…png`: tall images for anything that doesn't fit in 800 px (more below);
- `manifest.json`: when each image was captured, from which template commit, and whether it was stable.

All of them were captured from the packaged template build (`app-client`, `npm run package:linux`). Each
capture used a fresh `--user-data-dir`, so `localStorage` was at its defaults: dark theme, brand accent,
100 % and comfortable width.

To recapture the references, for example after a template change:

```sh
export BATO_ELECTRON=~/Code/BatoApps/bato-template/dist/electron/linux-unpacked/app   # needs the BATO_VIEW tab/card hook
tools/compare.sh reference all --full      # or: reference data · reference data/charts
```

**How Electron is captured.** The packaged app takes Chromium's own screenshot (`BATO_CAPTURE=<file>`
in `app-client/electron/main.js`) once two captures 400 ms apart are identical, then quits. Its window
still opens on the off-screen workspace and is checked there. These captures are identical to a
`shot.sh` capture of the same window except for the 4×4 px in each corner, which the compositor rounds.
On Home: 28 corner pixels differ, mean 0.001. Those corners are masked in every diff. Chromium's
capture never contains the stray cursor or another window (see Limitations).

**Clock-dependent views.** These views show today's date or the current time: the whole Date tab, and
`data/pagination`, `data/charts` and `data/timeline`, which scroll into the Timeline. For these,
`compare` always captures Electron again, seconds before the native window, unless you pass `--stored`.
You therefore need `BATO_ELECTRON` to compare them. Pass `--fresh` to recapture every reference.

**Full-card images.** With `BATO_CAPTURE_FULL=1`, Electron emulates a viewport of 1280 × (card height +
48) before it captures, or the whole page height for a tab. A full image is kept only when it is taller
than 800 px. Today no card is taller than 752 px, so every card fits in its 1280×800 shot. The only full
images are the five long tabs: `inputs` (3032 px), `date`, `feedback`, `layout` and `data`.

The window itself can't grow: the off-screen rule pins it to 1280×800, and the output is 1920×1200. So
full images are **for looking at only**, and `compare` doesn't diff them. Below the fold, check a native
card by eye against its full image, or capture it with `compare shot` and diff that against a crop by
hand.

### Limitations

- **Stray cursor.** Hyprland 0.56 paints the desktop pointer on the headless output at the same
  output-local position it has on the user's monitor. A `grim`/`shot.sh` capture can therefore contain
  a cursor that the app never saw, and no hover is involved. For native shots, `compare` reads
  `hyprctl cursorpos` before and after the capture and masks a 52×56 box around the possible position,
  noted `native:cursor`. That hides a few hundred pixels. Electron shots are never affected.
- **Animated cards** never settle and get the `unstable` note (in the reference run: `feedback` and `feedback/atoms`, because of the Spinner, the Skeleton pulse and the progress animations).
  Their diff includes the animation phase, so judge the image rather than the number.
- **The last cards of a tab** can't scroll to 24 px. The view clamps at the end of the page, so the
  card sits lower, and both clients must clamp identically. A different page height shows up as a
  vertical shift in the bbox.
- **Concurrency.** Captures are serialised across `compare` processes with a lock in
  `$XDG_RUNTIME_DIR/bato-compare.lock`, because every test window floats at the same spot. Windows
  started by other means are detected as `overlap:` (after waiting up to 15 s for them to go away),
  not prevented.
- The Electron hook re-applies `tab`/`card` each time the Examples page mounts in that run, which is
  harmless because it is dev only.

### Why a standalone crate

`tools/compare` has its own `[workspace]`, `Cargo.lock` and `target/`, and depends only on `png` and
`serde_json`. It stays out of the root workspace for three reasons:

- the root manifest is shared with the kit and gallery work;
- the harness shouldn't add to the kit's build or lock;
- it builds in seconds, offline, from the cargo cache.

We chose Rust over Python because the harness diffs 39 × 1 M pixels per batch, and the standard
library has no PNG codec.

## Real input: the nested session (`tools/nested.sh`)

The off-screen output is part of the user's own Hyprland session, so nothing may click or type
there, and a window there never gets real keyboard focus. Paste, shortcuts, the primary selection
and file drops need focus. `nested.sh` starts a separate session that never shows on screen:
a windowless `kwin_wayland --virtual` with Hyprland nested in it. It has its own runtime dir, HOME,
config and D-Bus, so `hyprctl`, `wl-copy` and `grim` reach only the nested sockets.
`tools/wldriver` (built on first `up`) drives a virtual pointer and does real `text/uri-list` drags.

```sh
tools/nested.sh up
BATO_VIEW='view=examples&tab=inputs&card=text-fields' tools/nested.sh run target/release/gallery
tools/nested.sh exec wl-copy "hello"                 # the nested clipboard
tools/nested.sh drive "move 690 310" "sleep 200" "click 700 316"
tools/nested.sh key CTRL,V                          # also: key ,x  key SHIFT,O  key ,Escape
tools/nested.sh exec wl-paste -n                    # what the app copied
tools/nested.sh drive "drag some.png 900 890"       # a real file drop
tools/nested.sh shot out.png 486 260 860 100        # region in output pixels
tools/nested.sh down
```

- **Coordinates** are output pixels: 1600×1000 at scale 1 (`NESTED_W`/`NESTED_H`).
- **Pointer moves:** move the pointer once before the first click, so the window gets `wl_pointer.enter`.
- **Key names** are xkb keysyms (`Return`, `End`, `Escape`). Keys go to `class:batopoc-nested`; `run` sets that
  app_id.
- **Hyprland ≥ 0.57** reads Lua, so the config is `hyprland.lua` and `key`/`focus` use `hl.dsp.*`. Both fall back to
  the classic dispatch syntax.
- **Socket path length:** sockets live in `$BATO_NESTED_RUN` (default `/tmp/bn-$UID`). A Unix socket path must fit in
  107 bytes, so keep that directory short.
