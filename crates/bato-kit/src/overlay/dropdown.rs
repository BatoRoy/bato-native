//! `Dropdown` (`Dropdown.tsx`): a button that opens a [`MenuList`] 4 px under
//! it, its left (or right, `align_right`) edge on the trigger's, clamped to
//! the window, never flipped. The open state is the widget's own (React
//! `useState`); the items publish app messages.
//!
//! Trigger: `px-3 py-2 rounded-lg border-app-control bg-app-bg/50 text-sm
//! text-app-subtext`, hover → `text-app-text` and an `accentBright` border
//! (150 ms), `focus-visible:ring-2 ring-app-accentBright`, the chevron turns
//! 180° while open (`transition-transform`, 150 ms). Dismissal: outside
//! press (not on the trigger, which toggles), Esc, scroll, resize; not window
//! blur. Keyboard: the menu's (↑ ↓ wrap and skip disabled, Home End, Enter).
//!
//! ```ignore
//! dropdown(&on, "Actions", vec![
//!     action("Edit", Msg::Edit).icon(Ico::Pencil).into(),
//!     action("Delete", Msg::Delete).icon(Ico::Trash2).danger().into(),
//! ])
//! dropdown(&on, "Add widget", items).icon(Ico::Plus).align_right(true)
//! ```
//!
//! [`MenuList`]: crate::menu::MenuList

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::row;
use iced::{Alignment, Color, Element, Event, Length, Padding, Rectangle, Size, mouse, window};

use super::anchored::{Dismiss, Position, anchored};
use super::popover::{local, press_probe};
use crate::anim::{Tween, ease_standard};
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, Icon, icon};
use crate::label::label;
use crate::menu::{MenuAction, MenuEntry, menu_list};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

// ── Chevron ─────────────────────────────────────────────────────────────

/// `transition-transform` (Tailwind: 150 ms, `cubic-bezier(.4, 0, .2, 1)`).
const TURN: Duration = Duration::from_millis(150);

/// A `ChevronDown` that turns 180° while `open`, animated. Without a colour
/// it inherits the text colour (the trigger's hover transition).
pub struct Chevron {
    open: bool,
    size: f32,
    color: Option<Color>,
}

pub fn chevron(open: bool, size: f32) -> Chevron {
    Chevron {
        open,
        size,
        color: None,
    }
}

impl Chevron {
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
}

struct ChevronState {
    open: bool,
    turn: Tween,
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Chevron {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ChevronState>()
    }
    fn state(&self) -> tree::State {
        let v = if self.open { PI } else { 0.0 };
        tree::State::new(ChevronState {
            open: self.open,
            turn: Tween::settled(v),
        })
    }
    fn diff(&self, tree: &mut Tree) {
        let st = tree.state.downcast_mut::<ChevronState>();
        if st.open != self.open {
            st.open = self.open;
            let now = Instant::now();
            st.turn = Tween {
                duration: TURN,
                easing: ease_standard,
                ..st.turn
            };
            st.turn.retarget(if self.open { PI } else { 0.0 }, now);
        }
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(self.size, self.size))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && tree
                .state
                .downcast_ref::<ChevronState>()
                .turn
                .is_running(*now)
        {
            shell.request_redraw();
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let r = tree
            .state
            .downcast_ref::<ChevronState>()
            .turn
            .value(Instant::now());
        let ico = icon(Ico::ChevronDown, self.size)
            .color_maybe(self.color)
            .rotation(r);
        <crate::icon::IconWidget as Widget<M, iced::Theme, iced::Renderer>>::draw(
            &ico,
            &Tree::empty(),
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

impl<'a, M: 'a> From<Chevron> for El<'a, M> {
    fn from(c: Chevron) -> Self {
        Element::new(c)
    }
}

// ── Entries ─────────────────────────────────────────────────────────────

/// Map the messages of menu entries (submenus included).
pub fn map_entries<M: Clone, N>(entries: &[MenuEntry<M>], f: &dyn Fn(M) -> N) -> Vec<MenuEntry<N>> {
    entries
        .iter()
        .map(|e| match e {
            MenuEntry::Separator => MenuEntry::Separator,
            MenuEntry::Action(a) => MenuEntry::Action(MenuAction {
                label: a.label.clone(),
                icon: a.icon,
                shortcut: a.shortcut.clone(),
                danger: a.danger,
                disabled: a.disabled,
                on_select: f(a.on_select.clone()),
                submenu: map_entries(&a.submenu, f),
            }),
        })
        .collect()
}

// ── Dropdown ────────────────────────────────────────────────────────────

pub struct Dropdown<M> {
    cx: Ctx,
    trigger: String,
    icon: Option<Icon>,
    items: Vec<MenuEntry<M>>,
    align_right: bool,
    width: Length,
}

/// A Dropdown on `cx` (the trigger's backdrop) with the label `trigger`.
pub fn dropdown<M: Clone>(
    cx: &Ctx,
    trigger: impl Into<String>,
    items: Vec<MenuEntry<M>>,
) -> Dropdown<M> {
    Dropdown {
        cx: cx.clone(),
        trigger: trigger.into(),
        icon: None,
        items,
        align_right: false,
        width: Length::Shrink,
    }
}

impl<M> Dropdown<M> {
    /// `align="right"`: the menu's right edge on the trigger's.
    pub fn align_right(mut self, on: bool) -> Self {
        self.align_right = on;
        self
    }
    /// A 14 px icon before the label, 6 px apart (Dashboard "Add widget").
    pub fn icon(mut self, ico: impl Into<Icon>) -> Self {
        self.icon = Some(ico.into());
        self
    }
    /// `className="w-full"` and the like.
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
}

/// The open state.
#[derive(Debug, Default)]
pub struct DropdownState {
    pub open: bool,
}

/// What the trigger and the menu tell the dropdown.
#[derive(Debug, Clone)]
pub enum DropdownEvent<M> {
    Toggle,
    Close,
    Pick(M),
}

/// The trigger button (`Dropdown.tsx`'s `<button>`).
pub fn trigger<'a, E: Clone + 'a>(
    cx: &Ctx,
    text: &str,
    ico: Option<Icon>,
    open: bool,
    width: Length,
    on_press: E,
) -> El<'a, E> {
    let p = cx.p().clone();
    let fill = c(cx.recessed());
    // A trigger wider than its text (`width(Fill)`, a fixed width): the
    // text takes the free space (`flex-1 text-left truncate`), so the
    // chevron sits at the right edge.
    let grow = width != Length::Shrink;
    let text_w = if grow { Length::Fill } else { Length::Shrink };
    let text_el: El<'a, E> = match ico {
        Some(i) => {
            let mut l = label(text.to_string(), 14.0, 20.0);
            if grow {
                l = l.ellipsis().width(Length::Fill);
            }
            row![icon(i, 14.0), l]
                .spacing(6)
                .align_y(Alignment::Center)
                .width(text_w)
                .into()
        }
        None => label(text.to_string(), 14.0, 20.0)
            .ellipsis()
            .width(text_w)
            .into(),
    };
    let body = row![text_el, chevron(open, 14.0)]
        .spacing(8)
        .align_y(Alignment::Center);
    surface(body)
        .padding(Padding::from([9.0, 13.0]))
        .width(width)
        .focus_ring(Ring::plain(cx))
        .style(move |s| {
            let hover = s == Status::Hover;
            BoxStyle::default()
                .bg(fill)
                .border(c(if hover { p.accent_bright } else { p.control }))
                .radius(8.0)
                .fg(c(if hover { p.text } else { p.subtext }))
        })
        .on_press(on_press)
        .into()
}

impl<'a, M: Clone + 'a> From<Dropdown<M>> for El<'a, M> {
    fn from(d: Dropdown<M>) -> Self {
        let Dropdown {
            cx,
            trigger: text,
            icon,
            items,
            align_right,
            width,
        } = d;
        local(
            move |s: &DropdownState| {
                let menu = s.open.then(|| {
                    press_probe(
                        menu_list(&cx, map_entries(&items, &DropdownEvent::Pick))
                            .on_close(DropdownEvent::Close),
                    )
                });
                anchored(
                    trigger(&cx, &text, icon, s.open, width, DropdownEvent::Toggle),
                    menu,
                )
                .position(Position::Below {
                    align_right,
                    gap: 4.0,
                })
                .dismiss(Dismiss::DROPDOWN)
                .on_dismiss(|_| DropdownEvent::Close)
                .into()
            },
            |s: &mut DropdownState, e| match e {
                DropdownEvent::Toggle => {
                    s.open = !s.open;
                    None
                }
                DropdownEvent::Close => {
                    s.open = false;
                    None
                }
                DropdownEvent::Pick(m) => Some(m),
            },
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{action, separator};

    #[test]
    fn entries_map_keeps_everything_but_the_message() {
        let items: Vec<MenuEntry<u8>> = vec![
            action("Edit", 1).icon(Ico::Pencil).shortcut("E").into(),
            separator(),
            action("Delete", 2)
                .danger()
                .disabled(true)
                .submenu(vec![action("Really", 3).into()])
                .into(),
        ];
        let mapped = map_entries(&items, &|m| u16::from(m) * 10);
        let MenuEntry::Action(a) = &mapped[0] else {
            panic!()
        };
        assert_eq!(
            (a.label.as_str(), a.icon, a.shortcut.as_deref(), a.on_select),
            ("Edit", Some(Ico::Pencil.into()), Some("E"), 10)
        );
        assert!(matches!(mapped[1], MenuEntry::Separator));
        let MenuEntry::Action(d) = &mapped[2] else {
            panic!()
        };
        assert!(d.danger && d.disabled);
        let MenuEntry::Action(sub) = &d.submenu[0] else {
            panic!()
        };
        assert_eq!(sub.on_select, 30);
    }

    /// `select(..).width(Fill)`: the chevron at the right edge, not next
    /// to the text; a shrinking trigger keeps it beside the text.
    #[test]
    fn a_wide_trigger_puts_the_chevron_at_the_right_edge() {
        use iced::advanced::layout::Limits;
        use iced::advanced::renderer::Headless;
        use iced::advanced::widget::Tree;
        crate::fonts::setup();
        let r = {
            let fut = <iced::Renderer as Headless>::new(
                iced::Font::DEFAULT,
                iced::Pixels(16.0),
                Some("tiny-skia"),
            );
            let mut fut = std::pin::pin!(fut);
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            loop {
                if let std::task::Poll::Ready(r) = fut.as_mut().poll(&mut cx) {
                    break r.expect("renderer");
                }
            }
        };
        let cx = Ctx::from_palette(bato_theme::palette(
            bato_theme::ThemeId::Dark,
            bato_theme::Rgb::new(0x8b, 0x5c, 0xf6),
        ));
        let chevron_right = |w: Length| {
            let mut el: El<'_, u8> = trigger(&cx, "Apple", None, false, w, 1);
            let mut tree = Tree::new(&el);
            let node = el.as_widget_mut().layout(
                &mut tree,
                &r,
                &Limits::new(iced::Size::ZERO, iced::Size::new(300.0, 100.0)),
            );
            let outer = node.bounds();
            // surface → row → [text, chevron]
            let row = &node.children()[0];
            let chev = row.children()[1].bounds();
            (outer.width, row.bounds().x + chev.x + chev.width)
        };
        let (w, right) = chevron_right(Length::Fill);
        assert_eq!(w, 300.0);
        assert!((w - 13.0 - right).abs() < 1.0, "{right}");
        let (w, right) = chevron_right(Length::Shrink);
        assert!(w < 150.0 && (w - 13.0 - right).abs() < 1.0, "{w} {right}");
    }
}
