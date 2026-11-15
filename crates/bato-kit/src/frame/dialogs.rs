//! App Options (spec §4.4, §4.5) and About (§4.7).

use std::time::Duration;

use bato_theme::{ACCENT_PRESETS, Rgb, SCALES, ThemeId};
use iced::widget::{Space, column, container, row, svg};
use iced::{Alignment, Length, Padding};

use super::{Frame, Msg, TestState, normalize};
use crate::El;
use crate::button::button;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::input::input;
use crate::kbd::app_mark;
use crate::label::{Align, label};
use crate::layout::{hline, vgap};
use crate::modal::modal_panel;
use crate::segmented::{Segment, segmented};
use crate::style::{c, ink_for, rgba};
use crate::surface::{BoxStyle, surface};
use crate::switch::switch;
use crate::timer::timer;

/// How long the Save button shows its check mark.
const SAVED_FOR: Duration = Duration::from_secs(2);

/// A labelled option row: title (and description) left, control right.
fn opt_row<'a, M: Clone + 'a>(
    cx: &Ctx,
    title: &'a str,
    desc: Option<&'a str>,
    control: El<'a, M>,
) -> El<'a, M> {
    let p = cx.p();
    let mut left: Vec<El<'a, M>> = vec![label(title, 13.0, 19.5).medium().color(c(p.text)).into()];
    if let Some(d) = desc {
        left.push(vgap(2.0));
        left.push(label(d, 12.0, 16.0).color(c(p.muted)).into());
    }
    container(
        row![column(left).width(Length::Fill), control]
            .spacing(24)
            .align_y(Alignment::Center),
    )
    .padding(Padding::from([14.0, 0.0]))
    .into()
}

/// An accent swatch: 28 px, raw hex, a check when active (rings 2 card + 4
/// text@.8), a filled star on the brand default, hover scale 1.1.
fn swatch<'a, M: Clone + 'a>(
    cx: &Ctx,
    hex: &str,
    is_default: bool,
    active: bool,
    msg: M,
) -> El<'a, M> {
    let p = cx.p();
    let raw = Rgb::hex(hex).unwrap_or(Rgb::BLACK);
    let ink = ink_for(raw);
    let glyph: El<'a, M> = if active {
        icon(Ico::Check, 14.0).color(c(ink)).into()
    } else if is_default {
        icon(Ico::Star, 11.0).color(rgba(ink, 0.9)).filled().into()
    } else {
        Space::new().width(0).height(0).into()
    };
    let mut st = BoxStyle::default().bg(c(raw)).radius(14.0);
    if active {
        st = st
            .ring(2.0, c(p.card))
            .ring(4.0, c(bato_theme::blend(p.text, 0.8, p.card)));
    }
    surface(glyph)
        .width(28)
        .height(28)
        .center()
        .hover_scale(1.1)
        .on_press(msg)
        .focus_ring(crate::focus::Ring::offset_on(cx, p.card))
        .fixed(st)
        .into()
}

/// The custom-accent swatch: CSS `conic-gradient(from 0deg, …)` in a circle.
/// SVG has no conic gradient, so it is 180 overlapping 2° wedges.
fn conic_svg() -> Vec<u8> {
    use std::fmt::Write;
    let stops: [(u8, u8, u8); 7] = [
        (0xf4, 0x3f, 0x5e),
        (0xf5, 0x9e, 0x0b),
        (0x10, 0xb9, 0x81),
        (0x06, 0xb6, 0xd4),
        (0x63, 0x66, 0xf1),
        (0xd9, 0x46, 0xef),
        (0xf4, 0x3f, 0x5e),
    ];
    let mut s = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 28 28"><clipPath id="c"><circle cx="14" cy="14" r="14"/></clipPath><g clip-path="url(#c)">"#,
    );
    let n = 180;
    for i in 0..n {
        let a0 = i as f32 / n as f32;
        let mid = (i as f32 + 0.5) / n as f32;
        let seg = (mid * 6.0).min(5.999);
        let k = seg.floor() as usize;
        let f = seg - k as f32;
        let (r0, g0, b0) = stops[k];
        let (r1, g1, b1) = stops[k + 1];
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
        let col = format!("#{:02x}{:02x}{:02x}", l(r0, r1), l(g0, g1), l(b0, b1));
        let ang = |t: f32| {
            let th = t * std::f32::consts::TAU;
            (14.0 + 20.0 * th.sin(), 14.0 - 20.0 * th.cos())
        };
        // Each wedge overlaps the next one fully, so AA seams never show.
        let (x0, y0) = ang(a0);
        let (x1, y1) = ang(a0 + 2.0 / n as f32);
        let _ = write!(
            s,
            r#"<path d="M14 14L{x0:.3} {y0:.3}L{x1:.3} {y1:.3}Z" fill="{col}" stroke="{col}" stroke-width="0.3"/>"#
        );
    }
    s.push_str("</g></svg>");
    s.into_bytes()
}

thread_local! {
    static CONIC: svg::Handle = svg::Handle::from_memory(conic_svg());
}

impl<P: Copy + PartialEq + std::fmt::Debug + Send + 'static> Frame<P> {
    pub(super) fn options_view<'a, M: Clone + 'a>(&'a self, wrap: fn(Msg<P>) -> M) -> El<'a, M> {
        let cx = self.cx.on_card();
        let p = cx.p();
        let brand = self.brand.accent.to_lowercase();
        let accent = self.settings.accent_or(&brand).to_lowercase();
        let mut sw: Vec<El<'a, M>> = vec![swatch(
            &cx,
            &brand,
            true,
            accent == brand,
            wrap(Msg::Accent(brand.clone())),
        )];
        for (_, hex) in ACCENT_PRESETS.iter().filter(|(_, h)| *h != brand) {
            sw.push(swatch(
                &cx,
                hex,
                false,
                accent == *hex,
                wrap(Msg::Accent(hex.to_string())),
            ));
        }
        let ring = c(cx.line());
        sw.push(
            surface(svg(CONIC.with(Clone::clone)).width(28).height(28))
                .width(28)
                .height(28)
                .on_press(wrap(Msg::ToggleCustom))
                .fixed(BoxStyle::default().inner_ring(ring).radius(14.0))
                .into(),
        );
        sw.push(Space::new().width(Length::Fill).into());
        sw.push(
            label(accent.clone(), 11.0, 16.5)
                .mono()
                .color(c(p.muted))
                .into(),
        );
        let mut accent_block: Vec<El<'a, M>> =
            vec![row(sw).spacing(10).align_y(Alignment::Center).into()];
        if self.custom_open {
            accent_block.push(vgap(12.0));
            accent_block.push(
                row![
                    label("Custom hex", 12.0, 16.0).medium().color(c(p.subtext)),
                    container(
                        input(&cx, "#rrggbb", &self.custom_hex, move |h| wrap(
                            Msg::CustomHex(h)
                        ))
                        .mono()
                        .text(12.0, 16.0)
                        .height(32.0)
                    )
                    .width(140),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .into(),
            );
        }
        accent_block.push(vgap(20.0));
        accent_block.push(hline(c(cx.line())));

        let theme_seg = segmented(
            &cx,
            self.settings.theme,
            ThemeId::ALL.map(|t| Segment::new(t, t.label())),
            move |t| wrap(Msg::Theme(t)),
        );
        let scale_seg = segmented(
            &cx,
            self.settings.scale,
            SCALES.map(|s| Segment::new(s, format!("{s}%"))),
            move |s| wrap(Msg::Scale(s)),
        );
        let width_seg = segmented(
            &cx,
            self.settings.wide,
            [
                Segment::new(false, "Comfortable"),
                Segment::new(true, "Full"),
            ],
            move |w| wrap(Msg::Wide(w)),
        );
        let line = || hline::<M>(c(cx.line()));

        let mut body = column![
            column(accent_block),
            opt_row(&cx, "Theme", None, theme_seg.into()),
            line(),
            opt_row(
                &cx,
                "Scale",
                Some("Scales every element."),
                scale_seg.into()
            ),
            line(),
            opt_row(
                &cx,
                "Content width",
                Some("Full uses the whole window."),
                width_seg.into()
            ),
            line(),
            opt_row(
                &cx,
                "Text selection",
                Some("Off feels more native."),
                switch(&cx, self.settings.text_select, move |v| wrap(
                    Msg::TextSelect(v)
                ))
                .into()
            ),
        ];
        if self.server_card {
            body = body.push(line()).push(self.server_card_view(&cx, wrap));
        }
        let description = format!("Make {} yours.", self.brand.name);
        modal_panel(
            &self.cx,
            "App Options",
            Some(description.as_str()),
            Some(self.brand.icon.into()),
            wrap(Msg::Close),
            body,
        )
    }

    fn server_card_view<'a, M: Clone + 'a>(&'a self, cx: &Ctx, wrap: fn(Msg<P>) -> M) -> El<'a, M> {
        let p = cx.p();
        let saved_url = self.backend_url();
        let dirty = normalize(&self.server_url) != saved_url;
        let saved = self.saved_at.is_some();
        let save: El<'a, M> = button(cx)
            .square(36)
            .icon(if saved { Ico::Check } else { Ico::Save })
            .on_press_maybe((dirty || saved).then(|| wrap(Msg::SaveServer)))
            .into();
        let save = match self.saved_at {
            Some(at) => timer(save, at, SAVED_FOR, wrap(Msg::SavedReset)).into(),
            None => save,
        };
        let mut items: Vec<El<'a, M>> = vec![
            label("Server address", 13.0, 19.5)
                .medium()
                .color(c(p.text))
                .into(),
            row![
                input(
                    cx,
                    "http://localhost:8080",
                    &self.server_url,
                    move |u| wrap(Msg::ServerUrl(u))
                )
                .mono()
                .text(12.0, 16.0)
                .height(36.0),
                button(cx)
                    .secondary()
                    .square(36)
                    .icon(Ico::Plug)
                    .on_press_maybe(
                        (self.test != TestState::Testing).then(|| wrap(Msg::TestServer))
                    ),
                save,
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        ];
        match self.test {
            TestState::Ok => items.push(
                label(
                    if dirty {
                        "Server reached — remember to save"
                    } else {
                        "Server reached"
                    },
                    12.0,
                    16.0,
                )
                .color(c(p.green))
                .into(),
            ),
            TestState::Fail => items.push(
                label("No server answered at this address", 12.0, 16.0)
                    .color(c(p.red))
                    .into(),
            ),
            _ => {}
        }
        items.push(
            label("Backend address including port. Saved on this machine — point it at whichever machine runs the server.", 12.0, 16.0)
                .color(c(p.muted))
                .wrap()
                .into(),
        );
        container(column(items).spacing(8))
            .padding(Padding::from([16.0, 0.0]))
            .into()
    }

    pub(super) fn about_view<'a, M: Clone + 'a>(&'a self, wrap: fn(Msg<P>) -> M) -> El<'a, M> {
        let cx = self.cx.on_card();
        let p = cx.p();
        let body = column![
            app_mark(&cx, self.brand.icon, 56.0),
            vgap(12.0),
            label(self.brand.name, 16.0, 24.0)
                .semibold()
                .color(c(p.text)),
            vgap(2.0),
            label(format!("v{}", self.brand.version), 12.0, 19.2)
                .mono()
                .color(c(p.muted)),
            vgap(8.0),
            label(self.brand.tagline, 14.0, 20.0)
                .color(c(p.subtext))
                .align(Align::Center)
                .max_width(320.0),
            vgap(20.0),
            self.about_update_view(&cx, wrap),
        ]
        .align_x(Alignment::Center)
        .width(Length::Fill);
        let title = format!("About {}", self.brand.name);
        modal_panel(
            &self.cx,
            &title,
            None,
            None,
            wrap(Msg::Close),
            container(body).padding(Padding::from([8.0, 0.0])),
        )
    }
}
