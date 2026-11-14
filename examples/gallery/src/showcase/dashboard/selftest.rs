//! `BATO_SELFTEST=dashboard` (with `BATO_VIEW=view=examples&tab=dashboard`
//! and a fresh `XDG_CONFIG_HOME`): edit mode, a header drag compacted by
//! Snap, a corner resize, remove, "Add widget", a Free-mode overlap and
//! Done, all through real pointer input.

use std::time::Duration;

use bato_kit::iced::mouse;
use bato_kit::iced::widget::Id;
use bato_kit::iced::{Event, Vector};
use bato_kit::overlay::script::{Step, Target};

use super::Msg;
use super::dashboard::BOX;

/// The grid's top inside the dashboard box: the 36 px toolbar + `mb-3`.
const GY: f32 = 48.0;
/// The 848 px grid's cell pitch: (848 − 110) / 12 + 10, and 56 + 10.
const CW: f32 = 71.5;
const CH: f32 = 66.0;
/// The toolbar's middle.
const TY: f32 = 18.0;

fn at(x: f32, y: f32) -> Target {
    Target::Id(Id::new(BOX), Vector::new(x, y))
}
/// A point in the grid.
fn grid(x: f32, y: f32) -> Target {
    at(x, GY + y)
}

pub fn steps() -> Vec<Step<Msg>> {
    let left = mouse::Button::Left;
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let release = || Step::Event(Event::Mouse(mouse::Event::ButtonReleased(left)));
    // Drag from `(x, y)` by whole cells, in two moves.
    let drag = |x: f32, y: f32, dx: f32, dy: f32| {
        vec![
            Step::Press(grid(x, y), left),
            Step::Move(grid(x + dx * CW / 2.0, y + dy * CH / 2.0)),
            Step::Move(grid(x + dx * CW, y + dy * CH)),
        ]
    };
    let mut s = vec![
        Step::Shot("rest"),
        // Edit (the right end of the toolbar).
        Step::Click(at(848.0 - 40.0, TY), left),
        wait(300),
        Step::Shot("edit"),
    ];
    // d1's header 4 cells right, onto the chart: live, accent border.
    s.extend(drag(100.0, 14.0, 4.0, 0.0));
    s.extend([wait(200), Step::Shot("drag-move"), release(), wait(60)]);
    s.extend([wait(400), Step::Shot("snapped")]);
    // d4 (floated to the top left) resized 1 col, 1 row from its corner.
    s.extend(drag(3.0 * 61.5 + 20.0 - 6.0, 122.0 - 6.0, 1.0, 1.0));
    s.extend([wait(200), Step::Shot("resize"), release(), wait(400)]);
    s.push(Step::Shot("resized"));
    // The List's × (top right of column 9..12).
    s.extend([
        Step::Click(grid(848.0 - 1.0 - 12.0 - 6.5, 15.0), left),
        wait(400),
        Step::Shot("removed"),
        // "Add widget", left of Done.
        Step::Click(at(848.0 - 100.0 - 60.0, TY), left),
        wait(400),
        Step::Shot("add-menu"),
        Step::Click(at(848.0 - 160.0, 127.0), left),
        wait(400),
        Step::Shot("added"),
        // Free: drop the new List over the stat at the top left.
        Step::Click(at(75.0, TY), left),
        wait(300),
    ]);
    s.extend(drag(60.0, 7.0 * CH + 14.0, 1.0, -7.0));
    s.extend([wait(200), Step::Shot("free-drag"), release(), wait(400)]);
    s.extend([
        Step::Shot("free-overlap"),
        Step::Click(at(848.0 - 40.0, TY), left),
        wait(400),
        Step::Shot("done"),
    ]);
    s
}
