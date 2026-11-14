//! Logical → device pixel factor and pixel snapping.
//!
//! Chromium pixel-snaps boxes, text baselines and icons to device pixels; iced
//! does not. Widgets read the current factor (window scale × UI scale) from
//! here. The frame keeps it up to date ([`set_device_scale`]); an app that does
//! not use the frame must call it on start and on `window::Event::Rescaled`.

use std::sync::atomic::{AtomicU32, Ordering};

use iced::Rectangle;

static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

/// Logical → device px factor (window scale factor × UI scale).
pub fn device_scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

pub fn set_device_scale(s: f32) {
    SCALE.store(s.to_bits(), Ordering::Relaxed);
}

/// `r` with its edges rounded to whole device pixels.
pub fn snap_rect(r: Rectangle) -> Rectangle {
    let s = device_scale();
    let x0 = (r.x * s).round();
    let y0 = (r.y * s).round();
    let x1 = ((r.x + r.width) * s).round();
    let y1 = ((r.y + r.height) * s).round();
    Rectangle { x: x0 / s, y: y0 / s, width: (x1 - x0) / s, height: (y1 - y0) / s }
}
