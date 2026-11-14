//! `BATO_SELFTEST=feedback` (with `BATO_VIEW=view=examples&tab=feedback&card=alerts`):
//! every interactive state of the Feedback tab through real input events.

use std::time::Duration;

use bato_kit::iced::keyboard::Modifiers;
use bato_kit::iced::keyboard::key::Named;
use bato_kit::iced::mouse::{self, ScrollDelta};
use bato_kit::iced::widget::Id;
use bato_kit::iced::{Event, Vector};
use bato_kit::overlay::script::{Step, Target};

use super::{Msg, accordion, alerts, navigation, stepper};

fn at(id: &'static str, x: f32, y: f32) -> Target {
    Target::Id(Id::new(id), Vector::new(x, y))
}

pub fn steps() -> Vec<Step<Msg>> {
    let left = mouse::Button::Left;
    let none = Modifiers::default();
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    vec![
        Step::Shot("rest"),
        // Breadcrumbs: hover "Library" (muted → text).
        Step::Move(at(navigation::LIBRARY, 80.0, 10.0)),
        wait(300),
        Step::Shot("crumb-hover"),
        // Alerts: hover then press the warning's close button (a toast).
        Step::Move(at(alerts::CLOSE, 782.0, 33.0)),
        wait(200),
        Step::Shot("alert-close-hover"),
        Step::Click(at(alerts::CLOSE, 782.0, 33.0), left),
        wait(400),
        Step::Shot("alert-dismissed"),
        // Down to the Stepper and the Accordion.
        Step::Move(at(navigation::LIBRARY, 400.0, 60.0)),
        Step::Event(Event::Mouse(mouse::Event::WheelScrolled {
            delta: ScrollDelta::Lines { x: 0.0, y: -6.0 },
        })),
        wait(800),
        // Stepper: Next (Profile complete, Done current), then the first disc.
        Step::Click(Target::Center(Id::new(stepper::NEXT)), left),
        wait(300),
        Step::Shot("stepper-next"),
        Step::Click(at(stepper::STEPS, 16.0, 16.0), left),
        wait(300),
        Step::Shot("stepper-first"),
        // Keyboard: Tab to the second disc (ring), Enter selects it.
        Step::Key(Named::Tab, none),
        wait(200),
        Step::Shot("stepper-focus"),
        Step::Key(Named::Enter, none),
        wait(300),
        Step::Shot("stepper-enter"),
        // Accordion: open the first, catch it mid-way, then open the second
        // (the first closes).
        Step::Click(at(accordion::ITEMS, 200.0, 23.0), left),
        wait(90),
        Step::Shot("accordion-opening"),
        wait(300),
        Step::Shot("accordion-open"),
        Step::Click(at(accordion::ITEMS, 200.0, 91.0 + 8.0 + 23.0), left),
        wait(400),
        Step::Shot("accordion-second"),
        // Hover a header (text@.06).
        Step::Move(at(
            accordion::ITEMS,
            200.0,
            46.0 + 8.0 + 46.0 + 45.0 + 8.0 + 23.0,
        )),
        wait(300),
        Step::Shot("accordion-hover"),
    ]
}
