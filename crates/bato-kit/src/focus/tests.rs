use super::*;
use crate::surface::surface;

use iced::advanced::Shell;
use iced::advanced::clipboard;
use iced::advanced::layout::{Layout, Limits};
use iced::advanced::renderer::Headless;
use iced::advanced::widget::Tree;
use iced::keyboard::{Location, Modifiers, key::Code, key::Physical};
use iced::widget::{Space, column, row};
use iced::{Element, Font, Pixels, Point, Size};

// ── Pure logic ──────────────────────────────────────────────────────────

fn e(order: usize, tab_stop: bool, focused: bool, scopes: &[u64]) -> Entry {
    Entry {
        order,
        token: Some(order as u64),
        id: None,
        bounds: Rectangle::default(),
        focused,
        tab_stop,
        scopes: scopes.to_vec(),
    }
}

#[test]
fn tab_walks_document_order_and_wraps() {
    let mut es = vec![
        e(1, true, false, &[]),
        e(3, false, false, &[]),
        e(5, true, false, &[]),
    ];
    assert_eq!(
        plan(&es, &[], None, false),
        Some(0),
        "nothing focused: first"
    );
    assert_eq!(plan(&es, &[], None, true), Some(2), "Shift+Tab: last");
    es[0].focused = true;
    assert_eq!(
        plan(&es, &[], None, false),
        Some(2),
        "skips the non tab stop"
    );
    assert_eq!(plan(&es, &[], None, true), Some(2), "wraps backwards");
    es[0].focused = false;
    es[2].focused = true;
    assert_eq!(plan(&es, &[], None, false), Some(0), "wraps forwards");
    es[2].focused = false;
    es[1].focused = true;
    assert_eq!(
        plan(&es, &[], None, false),
        Some(2),
        "from a tabindex=-1 widget"
    );
    assert_eq!(plan(&es, &[], None, true), Some(0));
    assert_eq!(plan(&[], &[], None, false), None);
}

#[test]
fn tab_starts_after_the_pressed_widget() {
    let es = vec![
        e(2, true, false, &[]),
        e(6, true, false, &[]),
        e(9, true, false, &[]),
    ];
    let anchor = (4, vec![]);
    assert_eq!(plan(&es, &[], Some(&anchor), false), Some(1));
    assert_eq!(plan(&es, &[], Some(&anchor), true), Some(0));
    let end = (12, vec![]);
    assert_eq!(plan(&es, &[], Some(&end), false), Some(0), "wraps");
}

#[test]
fn the_topmost_trap_confines_tab() {
    let scopes = [
        ScopeInfo { key: 7, trap: true },
        ScopeInfo { key: 8, trap: true },
    ];
    let mut es = vec![
        e(1, true, true, &[]),
        e(4, true, false, &[7]),
        e(6, true, false, &[7, 8]),
        e(7, true, false, &[7, 8]),
    ];
    // Focus outside the top trap: pulled back in at the near end.
    assert_eq!(plan(&es, &scopes, None, false), Some(2));
    assert_eq!(plan(&es, &scopes, None, true), Some(3));
    es[0].focused = false;
    es[3].focused = true;
    assert_eq!(plan(&es, &scopes, None, false), Some(2), "wraps inside");
    // An anchor outside the trap is ignored.
    es[3].focused = false;
    let outside = (2, vec![]);
    assert_eq!(plan(&es, &scopes, Some(&outside), true), Some(3));
    // A non-trapping scope does not confine.
    let open = [ScopeInfo {
        key: 7,
        trap: false,
    }];
    es[1].focused = true;
    assert_eq!(plan(&es, &open, None, false), Some(2));
}

#[test]
fn scripts_parse() {
    use root::{Step, parse_script};
    assert_eq!(parse_script("focus").unwrap().len(), 3);
    let s = parse_script("focus:focus=a, tab*2,shift-tab,space,click=10;20").unwrap();
    assert_eq!(s[0], Step::Focus("a".into()));
    assert_eq!(s[1], Step::Key(Named::Tab, false));
    assert_eq!(s[3], Step::Key(Named::Tab, true));
    assert_eq!(s[4], Step::Key(Named::Space, false));
    assert_eq!(s[5], Step::Click(Point::new(10.0, 20.0)));
    assert!(parse_script("scroll").is_none());
    assert!(parse_script("focus:jump").is_none());
}

#[test]
fn roving_steps() {
    assert_eq!(
        roving::step(&Key::Named(Named::ArrowLeft), Axis::Horizontal),
        Some(-1)
    );
    assert_eq!(
        roving::step(&Key::Named(Named::ArrowDown), Axis::Horizontal),
        None
    );
    assert_eq!(
        roving::step(&Key::Named(Named::ArrowDown), Axis::Both),
        Some(1)
    );
    assert_eq!(roving::next(0, -1, 3, true), 2);
    assert_eq!(roving::next(2, 1, 3, true), 0);
    assert_eq!(roving::next(2, 1, 3, false), 2);
}

fn cx() -> Ctx {
    Ctx::from_palette(bato_theme::palette(
        bato_theme::ThemeId::Dark,
        Rgb::hex("#8b5cf6").unwrap(),
    ))
}

#[test]
fn rings_match_the_tailwind_shadows() {
    let cx = cx();
    let p = cx.p().clone();
    // focusRing: offset in the page bg even on a card.
    assert_eq!(
        Ring::focus(&cx.on_card()).layers(),
        vec![(2.0, c(p.bg)), (4.0, c(p.accent_bright))]
    );
    assert_eq!(Ring::plain(&cx).layers(), vec![(2.0, c(p.accent_bright))]);
    assert_eq!(Ring::offset_on(&cx, p.card).layers()[0], (2.0, c(p.card)));
}

// ── Widgets through the real update path (headless tiny-skia) ──────────

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    A,
    B,
    C,
    Text(String),
    Pick(usize),
}

type El<'a> = Element<'a, Msg, iced::Theme, iced::Renderer>;

fn renderer() -> iced::Renderer {
    let fut = <iced::Renderer as Headless>::new(Font::DEFAULT, Pixels(16.0), Some("tiny-skia"));
    let mut fut = std::pin::pin!(fut);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(r) = fut.as_mut().poll(&mut cx) {
            return r.expect("tiny-skia renderer");
        }
    }
}

fn pad<'a>() -> El<'a> {
    Space::new().width(20).height(20).into()
}

const VIEW: Size = Size::new(800.0, 600.0);

struct Harness {
    r: iced::Renderer,
    tree: Tree,
}

fn key_ev(named: Named, shift: bool, pressed: bool) -> Event {
    let k = Key::Named(named);
    let code = match named {
        Named::Tab => Code::Tab,
        Named::Space => Code::Space,
        Named::Enter => Code::Enter,
        Named::Escape => Code::Escape,
        Named::ArrowLeft => Code::ArrowLeft,
        _ => Code::ArrowRight,
    };
    let modifiers = if shift {
        Modifiers::SHIFT
    } else {
        Modifiers::empty()
    };
    Event::Keyboard(if pressed {
        keyboard::Event::KeyPressed {
            key: k.clone(),
            modified_key: k,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    } else {
        keyboard::Event::KeyReleased {
            key: k.clone(),
            modified_key: k,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers,
        }
    })
}

impl Harness {
    fn new(el: &El<'_>) -> Self {
        Self {
            r: renderer(),
            tree: Tree::new(el),
        }
    }

    fn send(&mut self, el: &mut El<'_>, event: Event, cursor: mouse::Cursor) -> Vec<Msg> {
        self.tree.diff(&*el);
        let node =
            el.as_widget_mut()
                .layout(&mut self.tree, &self.r, &Limits::new(Size::ZERO, VIEW));
        let mut msgs = Vec::new();
        let mut shell = Shell::new(&mut msgs);
        el.as_widget_mut().update(
            &mut self.tree,
            &event,
            Layout::new(&node),
            cursor,
            &self.r,
            &mut clipboard::Null,
            &mut shell,
            &Rectangle::new(Point::ORIGIN, VIEW),
        );
        msgs
    }

    fn idle(&mut self, el: &mut El<'_>) {
        self.send(
            el,
            Event::Mouse(mouse::Event::CursorLeft),
            mouse::Cursor::Unavailable,
        );
    }

    fn key(&mut self, el: &mut El<'_>, named: Named, shift: bool) -> Vec<Msg> {
        let none = mouse::Cursor::Unavailable;
        let mut out = self.send(el, key_ev(named, shift, true), none);
        out.extend(self.send(el, key_ev(named, shift, false), none));
        out
    }

    fn click(&mut self, el: &mut El<'_>, p: Point) -> Vec<Msg> {
        let at = mouse::Cursor::Available(p);
        let mut out = self.send(
            el,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            at,
        );
        out.extend(self.send(
            el,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            at,
        ));
        out
    }

    /// The focused widget's id.
    fn focused(&mut self, el: &mut El<'_>) -> Option<Id> {
        self.tree.diff(&*el);
        let node =
            el.as_widget_mut()
                .layout(&mut self.tree, &self.r, &Limits::new(Size::ZERO, VIEW));
        let mut col = Collect::default();
        el.as_widget_mut()
            .operate(&mut self.tree, Layout::new(&node), &self.r, &mut col);
        col.entries
            .into_iter()
            .rfind(|e| e.focused)
            .and_then(|e| e.id)
    }
}

fn id(s: &'static str) -> Option<Id> {
    Some(Id::new(s))
}

fn page<'a>(cx: &Ctx, text: &str) -> El<'a> {
    focus_root(column![
        row![
            surface(pad()).on_press(Msg::A).id("a"),
            surface(pad()).on_press(Msg::B).id("b").disabled(true),
            surface(pad()).on_press(Msg::C).id("c").tab_stop(false),
            crate::button::button(cx)
                .label("B")
                .id("btn")
                .on_press(Msg::B),
        ],
        crate::input::input(cx, "Name", text, Msg::Text).id("name"),
        surface(pad()).on_press(Msg::C).id("last"),
    ])
    .into()
}

#[test]
fn tab_order_activation_and_modality() {
    let cx = cx();
    let mut el = page(&cx, "");
    let mut h = Harness::new(&el);
    assert_eq!(h.focused(&mut el), None);

    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("a"));
    assert!(focus_visible(), "a key press switches to keyboard modality");
    let r0 = revision();
    h.key(&mut el, Named::Tab, false);
    assert_eq!(
        h.focused(&mut el),
        id("btn"),
        "disabled and tab_stop(false) are skipped"
    );
    assert!(revision() > r0);
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("name"), "into the text input");
    h.key(&mut el, Named::Tab, false);
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("a"), "wraps");
    h.key(&mut el, Named::Tab, true);
    assert_eq!(h.focused(&mut el), id("last"), "Shift+Tab");

    // Enter clicks on press, Space on release.
    assert_eq!(h.key(&mut el, Named::Enter, false), vec![Msg::C]);
    assert_eq!(h.key(&mut el, Named::Space, false), vec![Msg::C]);

    // A click focuses without a ring; a later key shows it.
    assert_eq!(h.click(&mut el, Point::new(10.0, 10.0)), vec![Msg::A]);
    assert_eq!(h.focused(&mut el), id("a"));
    assert!(!focus_visible());
    h.key(&mut el, Named::Space, false);
    assert!(focus_visible());

    // A click on nothing blurs; Tab continues after the pressed widget.
    h.click(&mut el, Point::new(500.0, 500.0));
    assert_eq!(h.focused(&mut el), None);
    h.click(&mut el, Point::new(50.0, 10.0)); // "c": not a Tab stop, but focusable
    assert_eq!(h.focused(&mut el), id("c"));
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("btn"));
}

#[test]
fn requests_and_escape_in_a_field() {
    let cx = cx();
    let mut el = page(&cx, "x");
    let mut h = Harness::new(&el);
    request(Target::Id(Id::new("name")));
    h.idle(&mut el);
    assert_eq!(h.focused(&mut el), id("name"));
    // Esc does not blur a field (Chromium); it is left for the dialog.
    h.key(&mut el, Named::Escape, false);
    assert_eq!(h.focused(&mut el), id("name"));
}

fn with_scope<'a>(open: bool) -> El<'a> {
    let mut col = column![
        surface(pad()).on_press(Msg::A).id("opener"),
        surface(pad()).on_press(Msg::B).id("page"),
    ];
    if open {
        col = col.push(focus_scope(row![
            surface(pad()).on_press(Msg::A).id("s1"),
            surface(pad()).on_press(Msg::B).id("s2"),
        ]));
    }
    focus_root(col).into()
}

#[test]
fn scope_traps_autofocuses_and_restores() {
    let mut el = with_scope(false);
    let mut h = Harness::new(&el);
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("opener"));

    // Open: the first control inside takes focus.
    let mut el = with_scope(true);
    h.idle(&mut el);
    assert_eq!(h.focused(&mut el), id("s1"));
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("s2"));
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("s1"), "trapped");
    h.key(&mut el, Named::Tab, true);
    assert_eq!(h.focused(&mut el), id("s2"));

    // Close: focus goes back to the opener.
    let mut el = with_scope(false);
    h.idle(&mut el);
    assert_eq!(h.focused(&mut el), id("opener"));
}

#[test]
fn roving_moves_focus_and_selection() {
    let mut el: El<'_> = focus_root(
        roving(row![
            surface(pad()).on_press(Msg::A).id("r0"),
            surface(pad()).on_press(Msg::B).id("r1").tab_stop(false),
            surface(pad()).on_press(Msg::C).id("r2").tab_stop(false),
        ])
        .on_move(Msg::Pick),
    )
    .into();
    let mut h = Harness::new(&el);
    h.key(&mut el, Named::Tab, false);
    assert_eq!(h.focused(&mut el), id("r0"));
    assert_eq!(
        h.key(&mut el, Named::ArrowLeft, false),
        vec![Msg::Pick(2)],
        "wraps to the last member"
    );
    assert_eq!(h.focused(&mut el), id("r2"));
    assert_eq!(h.key(&mut el, Named::ArrowRight, false), vec![Msg::Pick(0)]);
    assert_eq!(h.focused(&mut el), id("r0"));
}

#[test]
fn focus_out_reports_keyboard_focus_leaving() {
    let mut el: El<'_> = focus_root(column![
        surface(pad()).on_press(Msg::A).id("out"),
        on_focus_out(
            row![
                surface(pad()).on_press(Msg::A).id("in1"),
                surface(pad()).on_press(Msg::B).id("in2"),
            ],
            Msg::C,
        ),
    ])
    .into();
    let mut h = Harness::new(&el);
    request(Target::Id(Id::new("in1")));
    h.idle(&mut el);
    h.idle(&mut el);
    assert_eq!(h.key(&mut el, Named::Tab, false), vec![], "still inside");
    assert_eq!(h.focused(&mut el), id("in2"));
    assert_eq!(
        h.key(&mut el, Named::Tab, false),
        vec![Msg::C],
        "left the panel"
    );
    assert_eq!(h.focused(&mut el), id("out"));
}

#[test]
fn rename_selects_the_stem_like_react() {
    use super::{grapheme_range, stem_range};
    assert_eq!(stem_range("notes.txt"), 0..5);
    assert_eq!(stem_range("archive.tar.gz"), 0..11);
    assert_eq!(
        stem_range(".bashrc"),
        0..7,
        "a leading dot is not an extension"
    );
    assert_eq!(stem_range("Makefile"), 0..8);
    assert_eq!(stem_range(""), 0..0);
    // Bytes → text_input cursor positions (graphemes).
    assert_eq!(grapheme_range("notes.txt", 0..5), (0, 5));
    let s = "café.png"; // é is 2 bytes
    assert_eq!(stem_range(s), 0..5);
    assert_eq!(grapheme_range(s, stem_range(s)), (0, 4));
    let s = "e\u{301}x.md"; // e + combining acute: one grapheme
    assert_eq!(grapheme_range(s, stem_range(s)), (0, 2));
    // Inside a character: rounds down; reversed: ordered; past the end: clamped.
    assert_eq!(grapheme_range("é", 1..2), (0, 1));
    assert_eq!(grapheme_range("abc", 3..1), (1, 3));
    assert_eq!(grapheme_range("abc", 0..99), (0, 3));
}

/// A dialog that opens on its name field with the stem selected: typing
/// replaces the stem and keeps the extension (BatoBrowse's rename prompt).
#[test]
fn scope_initial_focus_selects_the_stem() {
    let cx = cx();
    let name = "notes.txt";
    let sel = super::grapheme_range(name, super::stem_range(name));
    let mut el: El<'_> = focus_root(column![
        surface(pad()).on_press(Msg::A).id("a"),
        focus_scope(column![
            crate::button::button(&cx)
                .label("Close")
                .id("x")
                .on_press(Msg::B),
            crate::input::input(&cx, "Name", name, Msg::Text).id("name"),
        ])
        .initial("name")
        .select(Some(sel)),
    ])
    .into();
    let mut h = Harness::new(&el);
    h.idle(&mut el);
    h.idle(&mut el);
    assert_eq!(h.focused(&mut el), id("name"), "the initial field, not ✕");
    let k = Key::Character("x".into());
    let typed = h.send(
        &mut el,
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: k.clone(),
            modified_key: k,
            physical_key: Physical::Code(Code::KeyX),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
            text: Some("x".into()),
            repeat: false,
        }),
        mouse::Cursor::Unavailable,
    );
    assert!(
        typed
            .iter()
            .any(|m| matches!(m, Msg::Text(t) if t == "x.txt")),
        "{typed:?}"
    );
}
