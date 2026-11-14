//! Interaction lab section: scroll areas and fields (Phase 2, INF-7/INF-10).
//!
//! - a bounded list nested in the page (wheel/touch chaining, keys after a
//!   click), a horizontal strip, a two-axis grid, `Scrollable`'s faded list,
//!   a 10 000-row virtual list with variable heights, and scroll-to buttons;
//! - the Text fields card rebuilt on `field` + `text_control`, and the sizes.
//!
//! `BATO_SELFTEST=scroll` wraps the window in [`selftest`]: a script feeds
//! wheel, touchpad, touch, middle-click, key and scroll-to input through the
//! real widget tree, checks the scroll offsets, and prints
//! `[selftest] shot <name>` where a screenshot is worth taking.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bato_kit::field::FieldSize;
use bato_kit::iced::advanced::layout::{self, Layout};
use bato_kit::iced::advanced::renderer;
use bato_kit::iced::advanced::widget::{self as adv, Tree, Widget};
use bato_kit::iced::advanced::{Clipboard, Shell, overlay};
use bato_kit::iced::keyboard::{self, key};
use bato_kit::iced::widget::{Id, column, row};
use bato_kit::iced::{Element, Event, Point, Rectangle, Size, Task, Vector, mouse, touch, window};
use bato_kit::prelude::*;
use bato_kit::scroller::virtual_list::Extents;
use bato_kit::scroller::{self, Align};

const LIST: &str = "scroll-demo-list";
const STRIP: &str = "scroll-demo-strip";
const GRID: &str = "scroll-demo-grid";
const FADE: &str = "scroll-demo-fade";
const VLIST: &str = "scroll-demo-virtual";
const FIELDS: &str = "scroll-demo-fields";
const EMAIL: &str = "scroll-demo-email";

const LIST_ROWS: usize = 30;
const TILES: usize = 24;
const GRID_ROWS: usize = 24;
const GRID_COLS: usize = 16;
const VROWS: usize = 10_000;

/// A demo scroll area: (its scroller id, the container around it).
fn area(name: &str) -> (Id, Id) {
    (Id::from(name.to_string()), Id::from(format!("{name}-box")))
}

fn row_id(i: usize) -> Id {
    Id::from(format!("scroll-demo-row-{i}"))
}
fn tile_id(i: usize) -> Id {
    Id::from(format!("scroll-demo-tile-{i}"))
}
fn cell_id(r: usize, c: usize) -> Id {
    Id::from(format!("scroll-demo-cell-{r}-{c}"))
}

const WORDS: [&str; 8] = [
    "Aurora",
    "Basalt",
    "Cobalt",
    "Driftwood",
    "Ember",
    "Fjord",
    "Granite",
    "Harbor",
];

fn vheights() -> Vec<f32> {
    (0..VROWS)
        .map(|i| 28.0 + ((i * 7) % 5) as f32 * 9.0)
        .collect()
}

#[derive(Debug)]
pub struct State {
    picked: Option<usize>,
    search: String,
    password: String,
    reveal: bool,
    count: String,
    amount: String,
    name: String,
    heights: Vec<f32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            picked: None,
            search: String::new(),
            password: "hunter22".into(),
            reveal: false,
            count: String::new(),
            amount: String::new(),
            name: String::new(),
            heights: vheights(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pick(usize),
    ScrollTo(Goto),
    Search(String),
    Password(String),
    Reveal,
    Count(String),
    Amount(String),
    Name(String),
    Noop(String),
}

#[derive(Debug, Clone, Copy)]
pub enum Goto {
    Row(usize),
    Tile(usize),
    Cell(usize, usize),
    Virtual(usize),
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Pick(i) => state.picked = Some(i),
        Msg::ScrollTo(g) => goto(&state.heights, g),
        Msg::Search(s) => state.search = s,
        Msg::Password(s) => state.password = s,
        Msg::Reveal => state.reveal = !state.reveal,
        Msg::Count(s) => state.count = s.chars().take(20).collect(),
        Msg::Amount(s) => state.amount = s,
        Msg::Name(s) => state.name = s,
        Msg::Noop(_) => {}
    }
    Task::none()
}

fn goto(heights: &[f32], g: Goto) {
    match g {
        Goto::Row(i) => scroller::scroll_to(row_id(i), Align::Nearest),
        Goto::Tile(i) => scroller::scroll_to(tile_id(i), Align::Center),
        Goto::Cell(r, c) => scroller::scroll_to(cell_id(r, c), Align::Center),
        // A virtual row may not exist yet: scroll to its computed top.
        Goto::Virtual(i) => {
            let e = Extents::new(VROWS, &Heights::Each(heights.to_vec()), 0.0);
            scroller::scroll_offset(area(VLIST).0, None, Some(e.top(i)), true);
        }
    }
}

fn caption<'a>(cx: &Ctx, text: &str) -> El<'a, Msg> {
    label(text.to_string(), 12.0, 16.0)
        .medium()
        .color(c(cx.p().muted))
        .into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    column![scroll_card(state, cx), vgap(24.0), fields_card(state, cx)].into()
}

fn hover_row(hover: bato_kit::iced::Color) -> impl Fn(Status) -> BoxStyle {
    move |s| match s {
        Status::Hover => BoxStyle::default().bg(hover),
        Status::Rest => BoxStyle::default(),
    }
}

fn scroll_card<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p().clone();
    let line = c(on.line());
    let hover = c(on.wash(0.05));
    let text = c(p.text);
    let subtext = c(p.subtext);

    // Bounded list, nested in the page.
    let rows = (0..LIST_ROWS).map(|i| {
        let picked = state.picked == Some(i);
        let r = surface(
            row![
                label(format!("Row {}", i + 1), 14.0, 20.0)
                    .medium()
                    .color(if picked { c(p.accent_bright) } else { text }),
                Space::new().width(Length::Fill),
                label(WORDS[i % WORDS.len()], 12.0, 16.0).color(subtext),
            ]
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .width(Length::Fill)
        .style(hover_row(hover))
        .on_press(Msg::Pick(i));
        El::from(container(r).id(row_id(i)))
    });
    let (list_id, list_box) = area(LIST);
    let list = container(
        scroll_area(&on, column(rows))
            .max_height(180.0)
            .border(line, 8.0)
            .id(list_id),
    )
    .id(list_box);

    // Horizontal strip.
    let tile_bg = c(on.tint(0.15));
    let tiles = (0..TILES).map(|i| {
        El::from(
            container(
                surface(label(format!("Tile {}", i + 1), 13.0, 19.5).color(text))
                    .width(120)
                    .height(64)
                    .center()
                    .fixed(BoxStyle::default().bg(tile_bg).radius(8.0)),
            )
            .id(tile_id(i)),
        )
    });
    let (strip_id, strip_box) = area(STRIP);
    let strip = container(
        scroll_area(&on, container(row(tiles).spacing(8)).padding(8))
            .horizontal()
            .border(line, 8.0)
            .id(strip_id),
    )
    .id(strip_box);

    // Two-axis grid.
    let cell_bg = c(on.wash(0.04));
    let grid_rows = (0..GRID_ROWS).map(|r| {
        El::from(
            row((0..GRID_COLS).map(|col| {
                El::from(
                    container(
                        surface(
                            label(format!("{},{}", r + 1, col + 1), 12.0, 16.0)
                                .mono()
                                .color(subtext),
                        )
                        .width(72)
                        .height(36)
                        .center()
                        .fixed(BoxStyle::default().bg(cell_bg).radius(4.0)),
                    )
                    .id(cell_id(r, col)),
                )
            }))
            .spacing(4),
        )
    });
    let (grid_id, grid_box) = area(GRID);
    let grid = container(
        scroll_area(&on, container(column(grid_rows).spacing(4)).padding(8))
            .both()
            .height(200)
            .border(line, 8.0)
            .id(grid_id),
    )
    .id(grid_box);

    // `<Scrollable maxHeight={150} fade className="rounded-lg border border-app-border">`.
    let bg = c(p.bg);
    let fade_rows = (0..14).map(|i| {
        El::from(
            surface(label(format!("Scrollable row {}", i + 1), 14.0, 20.0).color(subtext))
                .padding([4, 8])
                .width(Length::Fill)
                .fixed(BoxStyle::default().bg(bg).radius(4.0)),
        )
    });
    let (fade_id, fade_box) = area(FADE);
    let faded = container(
        scroll_area(&on, container(column(fade_rows).spacing(8)).padding(12))
            .max_height(150.0)
            .faded()
            .border(c(p.border), 8.0)
            .id(fade_id),
    )
    .id(fade_box);

    // Virtual list.
    let heights = &state.heights;
    let vrows = virtual_list(VROWS, Heights::Each(heights.clone()), move |i| {
        let h = heights[i];
        surface(
            row![
                label(format!("#{}", i + 1), 12.0, 16.0)
                    .mono()
                    .color(subtext),
                label(format!("{h:.0} px tall"), 13.0, 19.5).color(text),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .width(Length::Fill)
        .height(Length::Fill)
        .center_y()
        .style(hover_row(hover))
        .into()
    });
    let (v_id, v_box) = area(VLIST);
    let vlist = container(
        scroll_area(&on, vrows)
            .max_height(200.0)
            .border(line, 8.0)
            .id(v_id),
    )
    .id(v_box);

    let go = |t: &str, g: Goto| {
        button(&on)
            .secondary()
            .sm()
            .label(t)
            .on_press(Msg::ScrollTo(g))
    };
    let buttons = row![
        go("Row 1", Goto::Row(0)),
        go("Row 15", Goto::Row(14)),
        go("Row 30", Goto::Row(29)),
        go("Tile 18", Goto::Tile(17)),
        go("Cell 20,12", Goto::Cell(19, 11)),
        go("Virtual #5000", Goto::Virtual(4999)),
    ]
    .spacing(8);

    let picked = match state.picked {
        Some(i) => format!("Picked row {}", i + 1),
        None => "Click or tap a row; the keys then scroll the list.".into(),
    };
    card(cx, "Scroll areas")
        .body(
            column![
                label(
                    "Wheel, touchpad and touch latch to the innermost area that can move and chain to the page on the next gesture. Shift+wheel scrolls sideways; middle-click autoscrolls the area under the pointer.",
                    14.0,
                    20.0
                )
                .color(subtext)
                .wrap(),
                buttons,
                caption(&on, "Bounded (max-height 180), nested in the page"),
                list,
                label(picked, 12.0, 16.0).color(c(p.muted)),
                caption(&on, "Horizontal"),
                strip,
                caption(&on, "Both axes"),
                grid,
                caption(&on, "Edge fade (Scrollable maxHeight 150 fade)"),
                faded,
                caption(&on, "Virtual: 10 000 rows of variable height"),
                vlist,
            ]
            .spacing(12),
        )
        .into()
}

fn fields_card<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p().clone();
    let eye =
        surface(icon(if state.reveal { Ico::EyeOff } else { Ico::Eye }, 15.0).color(c(p.muted)))
            .on_press(Msg::Reveal);
    let pair = |a: El<'a, Msg>, b: El<'a, Msg>| -> El<'a, Msg> { row![a, b].spacing(16).into() };
    let grid = column![
        pair(
            field(
                &on,
                text_control(&on, "Search…", &state.search, Msg::Search).left_icon(Ico::Search)
            )
            .label("Search")
            .into(),
            container(
                field(
                    &on,
                    text_control(&on, "you@example.com", "", Msg::Noop).left_icon(Ico::Mail)
                )
                .label("Email")
            )
            .id(Id::new(EMAIL))
            .into(),
        ),
        pair(
            field(
                &on,
                text_control(&on, "••••••••", &state.password, Msg::Password)
                    .secure(!state.reveal)
                    .right(eye)
            )
            .label("Password")
            .into(),
            field(
                &on,
                text_control(&on, "", "bad value", Msg::Noop).error(true)
            )
            .label("With error")
            .error(Some("That doesn't look right"))
            .into(),
        ),
        pair(
            field(&on, text_control(&on, "Max 20", &state.count, Msg::Count))
                .label("Char count")
                .counter(format!("{}/20", state.count.chars().count()))
                .into(),
            field(
                &on,
                text_control(&on, "0.00", &state.amount, Msg::Amount)
                    .prefix("$")
                    .suffix("USD")
            )
            .label("Prefix / suffix")
            .into(),
        ),
        pair(
            field(
                &on,
                text_control(&on, "", "you@example.com", Msg::Noop)
                    .left_icon(Ico::Mail)
                    .disabled(true)
            )
            .label("Disabled")
            .into(),
            Space::new().width(Length::Fill).into(),
        ),
    ]
    .spacing(16);
    let sizes = row![
        field(
            &on,
            text_control(&on, "Small", &state.name, Msg::Name).size(FieldSize::Sm)
        )
        .label("Size sm")
        .required(true),
        field(&on, text_control(&on, "Medium", &state.name, Msg::Name))
            .label("Size md")
            .required(true)
            .hint("Required, with a hint"),
        field(
            &on,
            text_control(&on, "Large", &state.name, Msg::Name).size(FieldSize::Lg)
        )
        .label("Size lg"),
    ]
    .spacing(16);
    container(card(cx, "Text fields").body(column![grid, vgap(24.0), sizes]))
        .id(Id::new(FIELDS))
        .max_width(848)
        .into()
}

// ── Self-test ───────────────────────────────────────────────────────────

/// `BATO_SELFTEST=scroll`: run the scripted interaction over `root` (the
/// whole window); otherwise `root` unchanged.
pub fn selftest<'a, M: 'a>(root: El<'a, M>) -> El<'a, M> {
    if std::env::var("BATO_SELFTEST").as_deref() != Ok("scroll") {
        return root;
    }
    Element::new(SelfTest { content: root })
}

#[derive(Clone, Copy)]
enum Act {
    /// Scroll the page so the container is centred.
    Show(&'static str),
    /// Cursor to a fraction of a page-level container's box.
    At(&'static str, f32, f32),
    /// Print a page-level container's box on screen.
    Where(&'static str),
    /// Cursor by a delta.
    By(f32, f32),
    Wheel(f32, f32),
    Pixels(f32, f32),
    Shift(bool),
    Press(mouse::Button),
    Release(mouse::Button),
    /// Touch: 0 press, 1 move by (dx, dy), 2 lift.
    Touch(u8, f32, f32),
    Key(key::Named),
    Go(Goto),
    Mark,
    Check(&'static str, fn(&Probe) -> bool),
    Shot(&'static str),
}

struct Probe {
    now: HashMap<&'static str, (f32, f32)>,
    mark: HashMap<&'static str, (f32, f32)>,
}

impl Probe {
    fn at(&self, n: &str) -> (f32, f32) {
        self.now.get(n).copied().unwrap_or((f32::NAN, f32::NAN))
    }
    fn d(&self, n: &str) -> (f32, f32) {
        let (a, b) = (self.at(n), self.mark.get(n).copied().unwrap_or((0.0, 0.0)));
        (a.0 - b.0, a.1 - b.1)
    }
    fn still(&self, n: &str) -> bool {
        let d = self.d(n);
        d.0.abs() < 0.5 && d.1.abs() < 0.5
    }
}

fn script() -> Vec<(u64, Act)> {
    use Act::*;
    let mut s: Vec<(u64, Act)> = vec![(1500, Show(LIST)), (800, Mark), (0, At(LIST, 0.5, 0.5))];
    for _ in 0..3 {
        s.push((60, Wheel(0.0, -1.0)));
    }
    s.push((
        450,
        Check("wheel scrolls the inner list, the page stays", |p| {
            p.d(LIST).1 > 300.0 && p.still("page")
        }),
    ));
    for _ in 0..10 {
        s.push((60, Wheel(0.0, -1.0)));
    }
    s.push((
        450,
        Check("latched: at its end the list keeps the gesture", |p| {
            p.at(LIST).1 >= 899.0 && p.still("page")
        }),
    ));
    s.extend([
        (0, Shot("list-end")),
        (700, Mark),
        (0, Wheel(0.0, -1.0)),
        (
            450,
            Check("the next gesture chains to the page", |p| {
                p.d("page").1 > 100.0 && p.still(LIST)
            }),
        ),
        // Touch: a sideways swipe on the strip scrolls it…
        (700, Show(STRIP)),
        (800, Mark),
        (0, At(STRIP, 0.5, 0.5)),
        (0, Touch(0, 0.0, 0.0)),
    ]);
    for _ in 0..10 {
        s.push((16, Touch(1, -20.0, 0.0)));
    }
    s.push((16, Touch(2, 0.0, 0.0)));
    s.push((
        300,
        Check(
            "touch: a sideways swipe scrolls the strip, the page stays",
            |p| p.d(STRIP).0 > 150.0 && p.still("page"),
        ),
    ));
    // …a vertical one goes to the page (the strip can't scroll that way).
    s.extend([(800, Mark), (0, Touch(0, 0.0, 0.0))]);
    for _ in 0..8 {
        s.push((16, Touch(1, 0.0, -15.0)));
    }
    s.push((300, Touch(2, 0.0, 0.0)));
    s.push((
        300,
        Check(
            "touch: a vertical swipe on the strip scrolls the page",
            |p| p.d("page").1 > 80.0 && p.still(STRIP),
        ),
    ));
    s.extend([
        (300, Show(GRID)),
        (800, Mark),
        (0, At(GRID, 0.5, 0.5)),
        (0, Shift(true)),
        (60, Wheel(0.0, -1.0)),
        (60, Wheel(0.0, -1.0)),
        (0, Shift(false)),
        (
            450,
            Check("shift+wheel scrolls the grid sideways", |p| {
                p.d(GRID).0 > 200.0 && p.d(GRID).1.abs() < 0.5 && p.still("page")
            }),
        ),
        (700, Mark),
    ]);
    for _ in 0..10 {
        s.push((10, Pixels(-2.0, -3.0)));
    }
    s.push((10, Pixels(0.0, 0.0)));
    s.push((
        700,
        Check("touchpad pans both axes and flings", |p| {
            let d = p.d(GRID);
            // x runs into its end (max 445) first.
            d.0 > 40.0 && d.1 > 360.0 && p.still("page")
        }),
    ));
    s.extend([
        (0, Shot("grid")),
        (300, Show(FADE)),
        (800, Mark),
        (0, At(FADE, 0.5, 0.3)),
        (0, Press(mouse::Button::Middle)),
        (50, By(0.0, 70.0)),
        (500, Shot("autoscroll")),
        (0, Release(mouse::Button::Middle)),
        (
            200,
            Check(
                "middle-click autoscroll scrolls the area under the pointer",
                |p| p.d(FADE).1 > 20.0 && p.still("page"),
            ),
        ),
        (300, Show(LIST)),
        (800, At(LIST, 0.5, 0.5)),
        (0, Press(mouse::Button::Left)),
        (30, Release(mouse::Button::Left)),
        (100, Mark),
        (0, Key(key::Named::Home)),
        (
            450,
            Check("keys scroll the clicked list, not the page", |p| {
                p.at(LIST).1 < 0.5 && p.still("page")
            }),
        ),
        (0, Key(key::Named::PageDown)),
        (
            450,
            Check("PageDown pages the list (87.5 % of 178 px)", |p| {
                (p.at(LIST).1 - 155.75).abs() < 1.0
            }),
        ),
        (0, Go(Goto::Row(29))),
        (
            600,
            Check("scroll_to(row 30) reveals it inside the list", |p| {
                p.at(LIST).1 >= 899.0
            }),
        ),
        (0, Mark),
        (0, Go(Goto::Cell(19, 11))),
        (
            700,
            Check(
                "scroll_to(cell) moves the grid and the page together",
                |p| {
                    let g = p.at(GRID);
                    g.0 > 0.0 && g.1 > 0.0 && !p.still("page")
                },
            ),
        ),
        (0, Shot("nested-reveal")),
        (300, Show(VLIST)),
        (800, Go(Goto::Virtual(4999))),
        (
            700,
            Check("virtual list: scroll to row 5000 (built on demand)", |p| {
                let e = Extents::new(VROWS, &Heights::Each(vheights()), 0.0);
                (p.at(VLIST).1 - e.top(4999)).abs() < 1.0
            }),
        ),
        (0, Shot("virtual")),
        (300, Show(FIELDS)),
        (800, Where(FIELDS)),
        (0, Shot("fields")),
        (0, At(EMAIL, 0.5, 0.75)),
        (0, Press(mouse::Button::Left)),
        (30, Release(mouse::Button::Left)),
        (400, Shot("field-focus")),
    ]);
    s
}

struct SelfTest<'a, M> {
    content: El<'a, M>,
}

struct TState {
    started: bool,
    next: Instant,
    step: usize,
    cursor: Point,
    mark: HashMap<&'static str, (f32, f32)>,
    passed: usize,
    failed: usize,
    script: Vec<(u64, Act)>,
    heights: Vec<f32>,
}

struct Find {
    id: Id,
    found: Option<Rectangle>,
}

impl adv::Operation for Find {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn adv::Operation)) {
        if self.found.is_none() {
            operate(self);
        }
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if self.found.is_none() && id == Some(&self.id) {
            self.found = Some(bounds);
        }
    }
}

fn container_of(name: &'static str) -> Id {
    match name {
        FIELDS | EMAIL => Id::new(name),
        _ => area(name).1,
    }
}

fn probe_now() -> HashMap<&'static str, (f32, f32)> {
    let mut m = HashMap::new();
    let p = scroller::page_offset();
    m.insert("page", (p.x, p.y));
    for n in [LIST, STRIP, GRID, FADE, VLIST] {
        if let Some(o) = scroller::offsets_of(&area(n).0) {
            m.insert(n, o);
        }
    }
    m
}

fn key_event(named: key::Named) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(named),
        modified_key: keyboard::Key::Named(named),
        physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    })
}

impl<'a, M: 'a> SelfTest<'a, M> {
    /// Run every step that is due, feeding its events to the window's
    /// widget tree like iced's runtime does.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &bato_kit::iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
        now: Instant,
    ) {
        let child = layout.children().next().unwrap();
        loop {
            let st = tree.state.downcast_mut::<TState>();
            if st.step >= st.script.len() || now < st.next {
                return;
            }
            let (_, act) = st.script[st.step];
            st.step += 1;
            let mut hold = 0;
            let mut events: Vec<Event> = Vec::new();
            let finger = touch::Finger(7);
            match act {
                Act::Show(name) => scroller::scroll_to(container_of(name), Align::Center),
                Act::Where(name) | Act::At(name, ..) => {
                    let (fx, fy) = match act {
                        Act::At(_, fx, fy) => (fx, fy),
                        _ => (-1.0, -1.0),
                    };
                    let mut f = Find {
                        id: container_of(name),
                        found: None,
                    };
                    self.content.as_widget_mut().operate(
                        &mut tree.children[0],
                        child,
                        renderer,
                        &mut f,
                    );
                    let st = tree.state.downcast_mut::<TState>();
                    match f.found {
                        Some(b) if fx < 0.0 => {
                            let o = scroller::page_offset();
                            eprintln!(
                                "[selftest] where {name} {:.0} {:.0} {:.0} {:.0}",
                                b.x - o.x,
                                b.y - o.y,
                                b.width,
                                b.height
                            );
                        }
                        Some(b) => {
                            // Page-level containers: on screen = layout - page offset.
                            let o = scroller::page_offset();
                            st.cursor =
                                Point::new(b.x - o.x + b.width * fx, b.y - o.y + b.height * fy);
                            events.push(Event::Mouse(mouse::Event::CursorMoved {
                                position: st.cursor,
                            }));
                        }
                        None => eprintln!("[selftest] no container {name}"),
                    }
                }
                Act::By(dx, dy) => {
                    st.cursor += Vector::new(dx, dy);
                    events.push(Event::Mouse(mouse::Event::CursorMoved {
                        position: st.cursor,
                    }));
                }
                Act::Wheel(x, y) => events.push(Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x, y },
                })),
                Act::Pixels(x, y) => events.push(Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels { x, y },
                })),
                Act::Shift(on) => {
                    events.push(Event::Keyboard(keyboard::Event::ModifiersChanged(if on {
                        keyboard::Modifiers::SHIFT
                    } else {
                        keyboard::Modifiers::empty()
                    })))
                }
                Act::Press(b) => events.push(Event::Mouse(mouse::Event::ButtonPressed(b))),
                Act::Release(b) => events.push(Event::Mouse(mouse::Event::ButtonReleased(b))),
                Act::Touch(kind, dx, dy) => {
                    st.cursor += Vector::new(dx, dy);
                    let position = st.cursor;
                    events.push(Event::Touch(match kind {
                        0 => touch::Event::FingerPressed {
                            id: finger,
                            position,
                        },
                        1 => touch::Event::FingerMoved {
                            id: finger,
                            position,
                        },
                        _ => touch::Event::FingerLifted {
                            id: finger,
                            position,
                        },
                    }));
                }
                Act::Key(named) => events.push(key_event(named)),
                // What the button's message does in the app's update.
                Act::Go(g) => goto(&st.heights, g),
                Act::Mark => st.mark = probe_now(),
                Act::Check(name, f) => {
                    let p = Probe {
                        now: probe_now(),
                        mark: st.mark.clone(),
                    };
                    let ok = f(&p);
                    if ok {
                        st.passed += 1;
                    } else {
                        st.failed += 1;
                    }
                    let mut keys: Vec<_> = p.now.keys().copied().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .map(|k| {
                            let (a, d) = (p.at(k), p.d(k));
                            format!("{k}=({:.0},{:.0})Δ({:.0},{:.0})", a.0, a.1, d.0, d.1)
                        })
                        .collect();
                    eprintln!(
                        "[selftest] {} {name}  [{}]",
                        if ok { "PASS" } else { "FAIL" },
                        vals.join(" ")
                    );
                }
                Act::Shot(name) => {
                    eprintln!("[selftest] shot {name}");
                    hold = 1500;
                }
            }
            let st = tree.state.downcast_mut::<TState>();
            let cursor = mouse::Cursor::Available(st.cursor);
            let wait = st.script.get(st.step).map_or(0, |s| s.0);
            st.next = now + Duration::from_millis(hold + wait);
            if st.step == st.script.len() {
                eprintln!("[selftest] done pass={} fail={}", st.passed, st.failed);
            }
            for e in events {
                self.content.as_widget_mut().update(
                    &mut tree.children[0],
                    &e,
                    child,
                    cursor,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
            }
            shell.request_redraw();
        }
    }
}

impl<'a, M: 'a> Widget<M, bato_kit::iced::Theme, bato_kit::iced::Renderer> for SelfTest<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn tag(&self) -> adv::tree::Tag {
        adv::tree::Tag::of::<TState>()
    }
    fn state(&self) -> adv::tree::State {
        adv::tree::State::new(TState {
            started: false,
            next: Instant::now(),
            step: 0,
            cursor: Point::new(-100.0, -100.0),
            mark: HashMap::new(),
            passed: 0,
            failed: 0,
            script: script(),
            heights: vheights(),
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &bato_kit::iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(n.size(), vec![n])
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        renderer: &bato_kit::iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        // The scripted pointer replaces the real one (off-screen anyway).
        if matches!(event, Event::Mouse(_) | Event::Touch(_)) {
            return;
        }
        let st = tree.state.downcast_mut::<TState>();
        let cursor = mouse::Cursor::Available(st.cursor);
        if !st.started {
            st.started = true;
            st.next = Instant::now() + Duration::from_millis(st.script.first().map_or(0, |s| s.0));
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let st = tree.state.downcast_ref::<TState>();
            if st.step < st.script.len() {
                self.run(
                    tree,
                    layout,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                    Instant::now(),
                );
                shell.request_redraw();
            }
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &bato_kit::iced::Renderer,
    ) -> mouse::Interaction {
        let st = tree.state.downcast_ref::<TState>();
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            mouse::Cursor::Available(st.cursor),
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &bato_kit::iced::Renderer,
        operation: &mut dyn adv::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut bato_kit::iced::Renderer,
        theme: &bato_kit::iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<TState>();
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            mouse::Cursor::Available(st.cursor),
            viewport,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &bato_kit::iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, bato_kit::iced::Theme, bato_kit::iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}
