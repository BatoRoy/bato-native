//! `BATO_SELFTEST=layout` (with `BATO_VIEW=view=examples&tab=layout&card=resizable`):
//! the splitters (hover, drag, nested drag, clamping) and the AppShell
//! action, through real input events. Sizes persist: run it again with the
//! same `XDG_CONFIG_HOME` and the first shot shows the dragged sizes.

use std::time::Duration;

use bato_kit::iced::mouse;
use bato_kit::iced::widget::Id;
use bato_kit::iced::{Event, Vector};
use bato_kit::overlay::script::{Step, Target};

use super::{Msg, app_shell, resizable};

fn at(id: &'static str, x: f32, y: f32) -> Target {
    Target::Id(Id::new(id), Vector::new(x, y))
}

pub fn steps() -> Vec<Step<Msg>> {
    let left = mouse::Button::Left;
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let release = Step::Event(Event::Mouse(mouse::Event::ButtonReleased(left)));
    // The box is 806 px wide inside a 1 px border: the outer hairline is at
    // 1 + 28 % of 804; the nested one at 1 + 68 % of 254.
    let h = 1.0 + 0.28 * 804.0;
    vec![
        Step::Shot("rest"),
        // Hover the strip 3 px off the hairline: lineStrong + col-resize.
        Step::Move(at(resizable::BOX, h + 3.0, 120.0)),
        wait(300),
        Step::Shot("splitter-hover"),
        // Drag it 120 px right; accentBright while held.
        Step::Press(at(resizable::BOX, h + 3.0, 120.0), left),
        Step::Move(at(resizable::BOX, h + 63.0, 120.0)),
        Step::Move(at(resizable::BOX, h + 123.0, 120.0)),
        wait(300),
        Step::Shot("splitter-drag"),
        release.clone(),
        wait(300),
        // Far left: clamped at the sidebar's 15 % minimum.
        Step::Press(at(resizable::BOX, h + 120.0, 60.0), left),
        Step::Move(at(resizable::BOX, 10.0, 60.0)),
        wait(200),
        Step::Shot("splitter-min"),
        Step::Move(at(resizable::BOX, h + 120.0, 60.0)),
        release.clone(),
        // The nested vertical splitter: drag it up 60 px.
        Step::Press(at(resizable::BOX, 600.0, 1.0 + 0.68 * 254.0), left),
        Step::Move(at(resizable::BOX, 600.0, 1.0 + 0.68 * 254.0 - 60.0)),
        wait(300),
        Step::Shot("nested-drag"),
        release,
        wait(300),
        Step::Shot("dropped"),
        // AppShell: the Action button (a toast).
        Step::Click(Target::Center(Id::new(app_shell::ACTION)), left),
        wait(400),
        Step::Shot("appshell-action"),
    ]
}
