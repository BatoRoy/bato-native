//! The palette bridge: Hybrid tokens as iced colors. Every color comes from
//! `bato_theme`; the helpers here only composite translucent tokens over a
//! known backdrop, the way the browser does (`blend` in bato_theme rounds like
//! theme.tsx). Views never do color math inline.

use bato_theme::{Palette, Rgb, Rgba, Shadow, blend};
use iced::Color;

use crate::surface::Sh;

/// An opaque token as an iced color.
pub fn c(rgb: Rgb) -> Color {
    Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}

/// A translucent token as an iced color (drawn with alpha).
pub fn ca(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a)
}

/// `rgb@a`, drawn with alpha.
pub fn rgba(rgb: Rgb, a: f32) -> Color {
    Color::from_rgba8(rgb.r, rgb.g, rgb.b, a)
}

pub fn lerp(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// The resolved palette plus the composites the components use.
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub p: Palette,
}

impl Tokens {
    pub fn new(p: Palette) -> Self {
        Self { p }
    }

    /// `line` hairline over an opaque backdrop.
    pub fn line_on(&self, backdrop: Rgb) -> Rgb {
        self.p.line.over(backdrop)
    }
    pub fn line_strong_on(&self, backdrop: Rgb) -> Rgb {
        self.p.line_strong.over(backdrop)
    }
    /// `text@a` wash over a backdrop (hover/active fills).
    pub fn wash(&self, a: f64, backdrop: Rgb) -> Rgb {
        blend(self.p.text, a, backdrop)
    }
    /// `accent@a` tint over a backdrop.
    pub fn tint(&self, a: f64, backdrop: Rgb) -> Rgb {
        blend(self.p.accent, a, backdrop)
    }
    /// `bg@.5` over the backdrop: input fill and segmented track.
    pub fn recessed(&self, backdrop: Rgb) -> Rgb {
        blend(self.p.bg, 0.5, backdrop)
    }
}

/// One token shadow layer, its alpha multiplied by `alpha_mul`.
pub fn sh(s: &Shadow, alpha_mul: f32) -> Sh {
    Sh {
        x: s.x,
        y: s.y,
        blur: s.blur,
        spread: s.spread,
        color: Color {
            a: s.color.a * alpha_mul,
            ..ca(s.color)
        },
    }
}

/// Outer shadows of a token list (inset layers handled by [`inset_of`]).
pub fn shadows(list: &[Shadow]) -> Vec<Sh> {
    list.iter()
        .filter(|s| !s.inset)
        .map(|s| sh(s, 1.0))
        .collect()
}

/// The `inset 0 1px 0` highlight of a token list, composited over `bg`.
pub fn inset_of(list: &[Shadow], bg: Rgb) -> Option<Color> {
    list.iter().find(|s| s.inset).map(|s| c(s.color.over(bg)))
}

/// `inkFor(hex)`: white unless near-black reads better on `raw`.
pub fn ink_for(raw: Rgb) -> Rgb {
    let dark = Rgb::new(6, 6, 9);
    if bato_theme::contrast(Rgb::WHITE, raw) >= bato_theme::contrast(dark, raw) {
        Rgb::WHITE
    } else {
        dark
    }
}
