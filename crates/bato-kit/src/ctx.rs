//! `Ctx` (INF-1): the theme tokens plus what a widget sits on.
//!
//! Hybrid colors like `text@.06` or `accent@.15` are translucent. iced draws a
//! quad's border *instead of* its background and has no group compositing, so
//! the kit composites every translucent token against the opaque colour behind
//! the widget, the way the browser would. Every component builder takes a
//! `&Ctx`; a container that changes the backdrop (a card, a dialog panel, a
//! popover) hands its children `cx.on(card)`.
//!
//! ```ignore
//! let cx = frame.ctx();                 // on the page background
//! let in_card = cx.on_card();           // inside a Card
//! button(&in_card, "Save").primary().on_press(Msg::Save)
//! ```

use std::rc::Rc;

use bato_theme::{Palette, Rgb};
use iced::Color;

use crate::style::{Tokens, c};

#[derive(Debug, Clone)]
pub struct Ctx {
    /// The resolved theme (shared, cheap to clone).
    pub t: Rc<Tokens>,
    /// The opaque colour this widget sits on.
    pub over: Rgb,
}

impl Ctx {
    /// On the page background.
    pub fn new(t: Rc<Tokens>) -> Self {
        let over = t.p.bg;
        Self { t, over }
    }

    pub fn from_palette(p: Palette) -> Self {
        Self::new(Rc::new(Tokens::new(p)))
    }

    /// The palette.
    pub fn p(&self) -> &Palette {
        &self.t.p
    }

    /// The same theme on another backdrop.
    pub fn on(&self, over: Rgb) -> Self {
        Self {
            t: self.t.clone(),
            over,
        }
    }

    /// Inside a Card, dialog panel or popover (`bg-app-card`).
    pub fn on_card(&self) -> Self {
        self.on(self.t.p.card)
    }

    /// On the page background (`bg-app-bg`).
    pub fn on_bg(&self) -> Self {
        self.on(self.t.p.bg)
    }

    // ── Composites over this backdrop ───────────────────────────────────

    /// `text@a` over the backdrop (hover / active fills).
    pub fn wash(&self, a: f64) -> Rgb {
        self.t.wash(a, self.over)
    }
    /// `accent@a` over the backdrop.
    pub fn tint(&self, a: f64) -> Rgb {
        self.t.tint(a, self.over)
    }
    /// `border-app-line` over the backdrop.
    pub fn line(&self) -> Rgb {
        self.t.line_on(self.over)
    }
    /// `border-app-lineStrong` over the backdrop.
    pub fn line_strong(&self) -> Rgb {
        self.t.line_strong_on(self.over)
    }
    /// `bg-app-bg/50` over the backdrop (input fill, segmented track).
    pub fn recessed(&self) -> Rgb {
        self.t.recessed(self.over)
    }
    /// `fg@a` over the backdrop.
    pub fn blend(&self, fg: Rgb, a: f64) -> Rgb {
        bato_theme::blend(fg, a, self.over)
    }

    /// An opaque token as an iced colour.
    pub fn c(&self, rgb: Rgb) -> Color {
        c(rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bato_theme::{ThemeId, palette};

    #[test]
    fn composites_follow_the_backdrop() {
        let cx = Ctx::from_palette(palette(ThemeId::Dark, Rgb::hex("#8b5cf6").unwrap()));
        let card = cx.on_card();
        assert_eq!(cx.over, cx.p().bg);
        assert_eq!(card.over, cx.p().card);
        assert_eq!(card.wash(0.05), cx.t.wash(0.05, cx.p().card));
        assert_ne!(cx.line(), card.line());
    }
}
