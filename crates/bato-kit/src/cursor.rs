//! Cursors (INF-17): the CSS `cursor` values the Hybrid components use,
//! mapped to iced's `mouse::Interaction` (winit `CursorIcon` underneath).

use iced::mouse::Interaction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// `cursor: default`
    Default,
    /// `cursor: pointer` (buttons, rows)
    Pointer,
    /// `cursor: not-allowed` (disabled controls)
    NotAllowed,
    /// `cursor: text`
    Text,
    /// `cursor: grab` / `grabbing` (sliders, drag handles)
    Grab,
    Grabbing,
    /// `cursor: move` (dashboard widgets)
    Move,
    /// `cursor: col-resize` / `row-resize` (split handles)
    ColResize,
    RowResize,
    /// `cursor: ew-resize` / `ns-resize`
    EwResize,
    NsResize,
    /// `cursor: nwse-resize` / `se-resize` (dashboard resize corner)
    NwseResize,
    /// `cursor: nesw-resize`
    NeswResize,
    /// `cursor: crosshair` (colour picker square)
    Crosshair,
    /// `cursor: all-scroll` (BatoScroll)
    AllScroll,
    /// `cursor: wait` / `progress`
    Wait,
    Progress,
    /// `cursor: copy` / `no-drop` (drag and drop feedback)
    Copy,
    NoDrop,
    /// `cursor: none`
    Hidden,
}

impl From<Cursor> for Interaction {
    fn from(c: Cursor) -> Self {
        match c {
            Cursor::Default => Interaction::Idle,
            Cursor::Pointer => Interaction::Pointer,
            Cursor::NotAllowed => Interaction::NotAllowed,
            Cursor::Text => Interaction::Text,
            Cursor::Grab => Interaction::Grab,
            Cursor::Grabbing => Interaction::Grabbing,
            Cursor::Move => Interaction::Move,
            Cursor::ColResize => Interaction::ResizingColumn,
            Cursor::RowResize => Interaction::ResizingRow,
            Cursor::EwResize => Interaction::ResizingHorizontally,
            Cursor::NsResize => Interaction::ResizingVertically,
            Cursor::NwseResize => Interaction::ResizingDiagonallyDown,
            Cursor::NeswResize => Interaction::ResizingDiagonallyUp,
            Cursor::Crosshair => Interaction::Crosshair,
            Cursor::AllScroll => Interaction::AllScroll,
            Cursor::Wait => Interaction::Wait,
            Cursor::Progress => Interaction::Progress,
            Cursor::Copy => Interaction::Copy,
            Cursor::NoDrop => Interaction::NoDrop,
            Cursor::Hidden => Interaction::Hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_resize_cursors() {
        assert_eq!(
            Interaction::from(Cursor::ColResize),
            Interaction::ResizingColumn
        );
        assert_eq!(
            Interaction::from(Cursor::NwseResize),
            Interaction::ResizingDiagonallyDown
        );
        assert_eq!(Interaction::from(Cursor::Pointer), Interaction::Pointer);
    }
}
