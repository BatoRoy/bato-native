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
