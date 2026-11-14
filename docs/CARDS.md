# Showcase card IDs

Shared by the Electron Showcase (`BATO_VIEW=view=examples&tab=<tab>&card=<card>`), the native
gallery (same variable, same IDs), and the fidelity harness (`tools/compare`). One card = one
native file `examples/gallery/src/showcase/<tab>/<card>.rs` (dashes → underscores).

| Tab | Card IDs, in page order |
|---|---|
| `inputs` | `text-fields` · `specialized` · `selects` · `sliders-color` · `time-input` · `search` · `code-editor` · `toggles-upload` |
| `date` | `pickers` · `calendar-time` (the 2-col Calendar / TimePicker row) · `calendar-view` |
| `feedback` | `atoms` · `alerts` · `navigation` · `stepper` · `accordion` |
| `layout` | `primitives` · `responsive` · `scrollable` · `editor-tabs` · `resizable` · `app-shell` |
| `dashboard` | `dashboard` |
| `overlays` | `context-menu` · `triggers` |
| `data` | `table` · `pagination` · `charts` (the 2×2 Line/Area/Bar/Sparkline) · `timeline` · `sortable` · `kanban` · `node-graph` |

`card=<id>` scrolls the card to the top of the content area (24 px below it), so a capture of
the window shows that card first. Without `card`, the tab opens scrolled to the top.
