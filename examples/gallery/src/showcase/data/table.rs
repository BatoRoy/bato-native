//! `data/table`: Data table. A role filter (SegmentedControl sm, each label
//! with a dimmed count) over a DataTable of 23 people: filterable,
//! selectable, 6 per page, sortable Name / Role (a Badge) / Age, and the row
//! actions View (labeled, accent), Edit and Delete (icons on row hover).

use bato_kit::data::{
    ActionTone, RowAction, TableMsg, TableState, Value, column_def, data_table,
};
use bato_kit::iced::widget::column;
use bato_kit::label::Align;
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The filter input, for the self-test.
pub const FILTER: &str = "data/table/filter";

#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub role: &'static str,
    pub age: u32,
}

/// The 23 people of the Showcase.
pub fn people() -> Vec<Person> {
    const NAMES: [&str; 7] = [
        "Ada Lovelace",
        "Alan Turing",
        "Grace Hopper",
        "Linus T",
        "Margaret H",
        "Dennis R",
        "Ken T",
    ];
    const ROLES: [&str; 4] = ["Engineer", "Designer", "PM", "QA"];
    (0..23)
        .map(|i| Person {
            id: (i + 1).to_string(),
            name: format!("{} {}", NAMES[i % 7], i + 1),
            role: ROLES[i % 4],
            age: 24 + (i as u32 % 30),
        })
        .collect()
}

/// The role filter: `None` = all.
pub type Role = Option<&'static str>;

#[derive(Debug)]
pub struct State {
    pub people: Vec<Person>,
    pub role: Role,
    pub filtered: Vec<Person>,
    pub table: TableState,
    pub selected: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        let people = people();
        Self {
            filtered: people.clone(),
            people,
            role: None,
            table: TableState::default(),
            selected: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Role(Role),
    Table(TableMsg),
    Selected(Vec<String>),
    View(String),
    Edit(String),
    Delete(String),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Role(r) => {
            state.role = r;
            state.filtered = state
                .people
                .iter()
                .filter(|p| r.is_none_or(|r| p.role == r))
                .cloned()
                .collect();
        }
        Msg::Table(m) => state.table.update(m),
        Msg::Selected(s) => state.selected = s,
        Msg::View(n) => return Some(Effect::Toast(format!("View {n}"), Tone::Info)),
        Msg::Edit(n) => return Some(Effect::Toast(format!("Edit {n}"), Tone::Info)),
        Msg::Delete(n) => return Some(Effect::Toast(format!("Deleted {n}"), Tone::Error)),
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let count = |r: Role| {
        state
            .people
            .iter()
            .filter(|p| r.is_none_or(|r| p.role == r))
            .count()
    };
    // `flex items-center gap-1.5`: an en space is exactly the 6 px gap.
    let seg = |r: Role, text: &str| {
        Segment::rich(
            r,
            [
                (text.to_string(), false),
                (format!("\u{2002}{}", count(r)), true),
            ],
        )
    };
    let filter = segmented(
        &on,
        state.role,
        [
            seg(None, "All"),
            seg(Some("Engineer"), "Engineers"),
            seg(Some("Designer"), "Designers"),
            seg(Some("PM"), "PMs"),
            seg(Some("QA"), "QA"),
        ],
        Msg::Role,
    )
    .sm();
    let columns = vec![
        column_def("name", "Name")
            .accessor(|p: &Person| Value::from(p.name.clone()))
            .sortable(),
        column_def("role", "Role")
            .accessor(|p: &Person| Value::from(p.role))
            .render(|p: &Person, cx: &Ctx| badge(cx, Tone::Info, p.role).into())
            .sortable(),
        column_def("age", "Age")
            .accessor(|p: &Person| Value::from(p.age))
            .sortable()
            .align(Align::Right),
    ];
    let table = data_table(&on, &state.filtered, columns, &state.table, Msg::Table)
        .row_key(|p| p.id.clone())
        .filterable(true)
        .filter_id(FILTER)
        .selectable(&state.selected, Msg::Selected)
        .page_size(6)
        .row_actions(|p| {
            vec![
                RowAction::new(Ico::Eye, Msg::View(p.name.clone()))
                    .label("View")
                    .tone(ActionTone::Accent),
                RowAction::new(Ico::Pencil, Msg::Edit(p.name.clone())).title("Edit"),
                RowAction::new(Ico::Trash2, Msg::Delete(p.name.clone()))
                    .tone(ActionTone::Danger)
                    .title("Delete"),
            ]
        });
    card(cx, "Data table")
        .body(column![filter, table].spacing(12))
        .into()
}

// ── BATO_SELFTEST=data:<card> (batch B6) ─────────────────────────────────

/// `BATO_SELFTEST=data:table|pagination|charts|timeline`, with
/// `BATO_VIEW=view=examples&tab=data&card=<card>`. Window points (the card
/// is scrolled to 24 px under the top), measured on the Electron reference.
pub fn b6_selftest(card: &str) -> Option<Vec<bato_kit::overlay::script::Step<super::Msg>>> {
    use bato_kit::iced::keyboard::Modifiers;
    use bato_kit::iced::keyboard::key::Named;
    use bato_kit::iced::mouse::{self, ScrollDelta};
    use bato_kit::iced::{Event, Point};
    use bato_kit::overlay::script::{Step, Target};
    use std::time::Duration;

    let at = |x: f32, y: f32| Target::At(Point::new(x, y));
    let left = mouse::Button::Left;
    let none = Modifiers::default();
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let chars = |s: &str| s.chars().map(|c| Step::Char(c, none)).collect::<Vec<_>>();
    let click = |x, y| Step::Click(at(x, y), left);
    let steps: Vec<Step<super::Msg>> = match card {
        "table" => [
            vec![
                Step::Shot("rest"),
                // Row hover: the fill and the Edit / Delete icons fade in.
                Step::Move(at(500.0, 240.0)),
                wait(400),
                Step::Shot("row-hover"),
                // Edit's title tooltip; the Delete icon tints red on hover.
                Step::Move(at(1091.0, 240.0)),
                wait(700),
                Step::Shot("edit-tooltip"),
                Step::Move(at(1122.0, 240.0)),
                wait(300),
                Step::Shot("delete-hover"),
                click(1122.0, 240.0),
                wait(400),
                Step::Shot("delete-toast"),
                // Select all on the page, then drop one (indeterminate).
                click(368.0, 194.0),
                wait(300),
                Step::Shot("select-all"),
                click(368.0, 292.0),
                wait(300),
                Step::Shot("indeterminate"),
                // Sort: Name asc, Name desc, then Age asc.
                click(420.0, 194.0),
                wait(300),
                Step::Shot("sort-name-asc"),
                click(420.0, 194.0),
                wait(300),
                Step::Shot("sort-name-desc"),
                click(855.0, 194.0),
                wait(300),
                Step::Shot("sort-age"),
                // Page 2.
                click(1028.0, 554.0),
                wait(300),
                Step::Shot("page-2"),
                // Filter (back to page 1), then no match: the empty state.
                click(500.0, 141.0),
            ],
            chars("grace"),
            vec![wait(300), Step::Shot("filter-grace")],
            chars("zz"),
            vec![
                wait(300),
                Step::Shot("filter-empty"),
            ],
            vec![Step::Key(Named::Backspace, none); 7],
            vec![
                // Role filter: Designers.
                click(535.0, 95.0),
                wait(300),
                Step::Shot("role-designers"),
                // Keyboard: Tab from the filter to the header checkbox.
                click(500.0, 141.0),
                Step::Key(Named::Tab, none),
                wait(200),
                Step::Shot("tab-checkbox"),
                Step::Key(Named::Tab, none),
                wait(200),
                Step::Shot("tab-sort-name"),
                Step::Key(Named::Enter, none),
                wait(300),
                Step::Shot("enter-sorts"),
            ],
        ]
        .concat(),
        "pagination" => vec![
            Step::Shot("rest"),
            Step::Move(at(398.0, 96.0)),
            wait(300),
            Step::Shot("hover-1"),
            click(614.0, 96.0),
            wait(300),
            Step::Shot("next-4"),
            // `1 … 3 [4] 5 … 12`: the 12 is now where Next was.
            click(614.0, 96.0),
            wait(300),
            Step::Shot("last-12"),
            Step::Key(Named::Tab, none),
            wait(200),
            Step::Shot("tab-focus"),
        ],
        "charts" => vec![
            Step::Move(at(460.0, 150.0)),
            wait(200),
            Step::Shot("line-hover-3"),
            Step::Move(at(700.0, 150.0)),
            wait(200),
            Step::Shot("line-hover-13"),
            Step::Move(at(1000.0, 150.0)),
            wait(200),
            Step::Shot("area-hover"),
            Step::Move(at(608.0, 470.0)),
            wait(300),
            Step::Shot("bar-hover-thu"),
            Step::Move(at(545.0, 573.0)),
            wait(300),
            Step::Shot("bar-label-hover-wed"),
            Step::Move(at(1000.0, 470.0)),
            wait(300),
            Step::Shot("left"),
        ],
        "timeline" => {
            // The last cards of the tab cannot scroll to the top: the page
            // clamps. 125 px lower while sortable / kanban / node-graph are
            // stubs (the Target::Id page-offset bug rules out ids).
            let dy = 125.0;
            vec![
                Step::Move(at(600.0, 137.0 + dy)),
                wait(700),
                Step::Shot("bar-hover-tooltip"),
                Step::Event(Event::Mouse(mouse::Event::WheelScrolled {
                    delta: ScrollDelta::Lines { x: -2.0, y: 0.0 },
                })),
                wait(800),
                Step::Shot("scrolled-sticky"),
                Step::Move(at(420.0, 185.0 + dy)),
                wait(300),
                Step::Shot("hover-under-label"),
                click(1017.0, 54.0 + dy),
                wait(400),
                Step::Shot("no-scroll"),
            ]
        }
        _ => return None,
    };
    Some(steps)
}
