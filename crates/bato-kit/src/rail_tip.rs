//! `RailTip` (§4.9): the collapsed-sidebar tooltip, a [`Tooltip`] on the
//! right side (card bubble, radius 6, lineStrong border, shadow lg, 12/16
//! text, 8 px away, 400 ms, 150 ms fade-in, shown on hover or keyboard
//! focus).
//!
//! [`Tooltip`]: crate::overlay::tooltip::Tooltip

use crate::El;
use crate::ctx::Ctx;
use crate::floating::Placement;
use crate::overlay::tooltip::tooltip;

pub fn rail_tip<'a, M: Clone + 'a>(
    cx: &Ctx,
    anchor: impl Into<El<'a, M>>,
    text: &str,
) -> El<'a, M> {
    tooltip(cx, anchor, text).side(Placement::Right).into()
}
