//! `<kbd>` chips and the AppMark brand tile.

use bato_theme::Rgb;
use iced::{Color, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, Sh, surface};

/// `<kbd>` (§4.1): h 20, px 6, radius 4, `lineStrong` border, `text@.04`
/// over `cx.over`, mono 10/15 subtext.
pub fn kbd<'a, M: Clone + 'a>(cx: &Ctx, text: &str) -> El<'a, M> {
    let p = cx.p();
    let bg = cx.wash(0.04);
    let style = BoxStyle::default()
        .bg(c(bg))
        .border(c(cx.t.line_strong_on(bg)))
        .radius(4.0);
    surface(label(text, 10.0, 15.0).mono().color(c(p.subtext)))
        .height(20)
        .padding(Padding::from([0.0, 7.0]))
        .center_y()
        .fixed(style)
        .into()
}

/// AppMark (§4.6): accent square of `size`, radius round(S × .28), the brand
/// glyph at round(S / 2) in `accent_ink`, `0 1px 2px #000@.25` and a white@.2
/// top highlight.
pub fn app_mark<'a, M: Clone + 'a>(cx: &Ctx, glyph: Ico, size: f32) -> El<'a, M> {
    let p = cx.p();
    let style = BoxStyle::default()
        .bg(c(p.accent))
        .radius((size * 0.28).round())
        .shadows(vec![Sh {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
        }])
        .inset_top(c(bato_theme::blend(Rgb::WHITE, 0.2, p.accent)));
    surface(icon(glyph, (size * 0.5).round()).color(c(p.accent_ink)))
        .width(size)
        .height(size)
        .center()
        .fixed(style)
        .into()
}
