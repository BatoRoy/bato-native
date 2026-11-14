//! Scroll lab (`ScrollLabPage.tsx`): the text rendering card and 1 000
//! virtualized rows, the bench for scrolling and text.

use bato_kit::iced::widget::column;
use bato_kit::prelude::*;

use crate::lab_rows::{self, PARAGRAPHS, lab_rows};

fn fmt_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn view<'a, M: Clone + 'a>(cx: &Ctx, wide: bool) -> El<'a, M> {
    let t = &cx.t;
    let p = cx.p();
    let text_card = card_box(
        cx,
        column![
            label("Text rendering", 14.0, 20.0)
                .semibold()
                .color(c(p.text)),
            vgap(16.0),
            column![
                label(PARAGRAPHS[0], 16.0, 24.0).color(c(p.text)).wrap(),
                label(PARAGRAPHS[0], 14.0, 20.0).color(c(p.text)).wrap(),
                label(PARAGRAPHS[1], 13.0, 19.5).color(c(p.subtext)).wrap(),
                label(PARAGRAPHS[1], 12.0, 16.0).color(c(p.muted)).wrap(),
                label(
                    "0123456789 · const answer = 42; // JetBrains Mono 12px",
                    12.0,
                    16.0
                )
                .mono()
                .tabular()
                .color(c(p.subtext))
                .wrap(),
            ]
            .spacing(12),
        ],
    );
    let line = t.line_on(p.card);
    let hover = t.wash(0.04, p.card);
    let colors = lab_rows::Colors {
        line: c(line),
        line_on_hover: c(t.line_on(hover)),
        hover: c(hover),
        tile: c(t.tint(0.15, p.card)),
        tile_on_hover: c(t.tint(0.15, hover)),
        accent_bright: c(p.accent_bright),
        text: c(p.text),
        muted: c(p.muted),
        subtext: c(p.subtext),
        section: bato_kit::card::card_style(cx),
    };
    page(
        column![
            label("Scroll lab", 30.0, 36.0)
                .semibold()
                .tracking(-0.025)
                .color(c(p.text)),
            vgap(4.0),
            label(
                format!(
                    "{} rows · wheel, touchpad and fling · text at 16 / 14 / 13 / 12 px",
                    fmt_thousands(lab_rows::ROWS)
                ),
                14.0,
                20.0
            )
            .color(c(p.subtext)),
            vgap(32.0),
            text_card,
            vgap(24.0),
            lab_rows(colors),
        ],
        wide,
        [48.0, 32.0],
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands() {
        assert_eq!(super::fmt_thousands(1000), "1,000");
        assert_eq!(super::fmt_thousands(999), "999");
    }
}
