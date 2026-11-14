//! Anchored overlays and the modal family: Popover, Dropdown, Select, ContextMenu, Tooltip (B2);
//! ConfirmDialog, Drawer, CommandPalette (B3).
//!
//! The floating layer itself (F2) is [`anchored`] (the portal: position, dismissal, enter
//! animation) on top of [`crate::floating`], with [`crate::menu`] for menus and
//! [`crate::modal`] for the stacked levels. [`script`] drives scripted self-tests.
//!
//! Phase 3 batches add one file per component here and one `mod` line below.

pub mod anchored;
pub mod confirm;
pub mod drawer;
pub mod palette;
pub mod script;

pub use anchored::{Anchored, Dismiss, DismissReason, Enter, Position, anchored, context_area};
pub use confirm::{ConfirmDialog, confirm_dialog};
pub use drawer::{Drawer, drawer};
pub use palette::{Command, Palette};
