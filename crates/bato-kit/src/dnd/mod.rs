//! In-app drag and drop (B7, INF-8): @dnd-kit's sensors, collision
//! detection and sortable strategies, on the kit's overlay layer and
//! scrollers.
//!
//! | Piece | dnd-kit | What |
//! |---|---|---|
//! | [`Session`] | `PointerSensor` (`distance: 4`), `KeyboardSensor` | press → drag past 4 px, Esc cancels, keyboard pick-up / move / drop |
//! | [`closest_center`], [`closest_corners`] | collision detection | which droppable the dragged rectangle is over |
//! | [`strategy`] | `vertical/horizontalListSortingStrategy` | where the other items slide to (200 ms `ease`, [`Motion`]) |
//! | [`Preview`] | the dragged node / `DragOverlay` | the dragged item drawn in the overlay layer, following the pointer |
//! | [`edge_speed`], [`crate::scroller::edge_scroll`] | `autoScroll` | scroll the innermost scroller whose 20 % edge band holds the pointer |
//! | [`sortable`] | `SortableContext` | a sortable list widget (SortableList, EditorTabs) |
//! | [`handle`], [`no_drag`] | handle `listeners`, `stopPropagation` | where a press may start a drag |
//! | [`os_drag`] | Electron's `webContents.startDrag` | drag files out to other apps (and know our own drag when it comes back) |
//!
//! The drag state lives in widget state; the app sees only the result
//! (`on_reorder(from, to)`, or the KanbanBoard's `on_change`). There is no
//! rotation transform in iced, so the KanbanBoard's `rotate-2` tilt is
//! dropped.
//!
//! **Touch.** A [`handle`] (`touch-action: none` in React) holds the finger
//! in the scrollers' touch latch from the press, so the page never scrolls
//! under it. An item without a handle lets the scroller take the finger
//! past its 15 px slop if it can scroll that way, which cancels the drag
//! (Chromium sends `pointercancel` when the page pans), so a swipe over a
//! Kanban card still scrolls the page.

mod engine;
mod list;
pub mod os_drag;
mod sort;

pub use engine::{
    ACTIVATION, DROP, Grab, Mark, MarkKind, Motion, Phase, Pointer, Preview, PreviewStyle, SORT,
    Session, Signal, ease, handle, may_drag, no_drag,
};
pub use list::{List, sortable};
pub use sort::{
    Axis, Dir, EDGE_ACCELERATION, EDGE_INTERVAL_MS, EDGE_THRESHOLD, array_move, closest_center,
    closest_corners, edge_speed, keyboard_target, strategy,
};
