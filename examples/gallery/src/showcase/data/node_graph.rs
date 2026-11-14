//! `data/node-graph`: Node graph. Four themed nodes wired Input → Process /
//! Validate → Output in a 320 px NodeGraph (dots, Controls, MiniMap), then
//! "Drag nodes, pan/zoom, and connect handles." and a Tip button (a toast).
//!
//! A card owns its `State`, `Msg`, `update` and `view`; the tab lists it once.

use bato_kit::button::{Size, Variant, btn_label, button_style};
use bato_kit::graph::{edge, node, node_graph};
use bato_kit::iced::widget::{column, container, row};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The graph and the Tip button, for the self-test.
pub const GRAPH: &str = "data/node-graph/graph";
pub const TIP: &str = "data/node-graph/tip";

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {
    Tip,
}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Tip => Some(Effect::Toast(
            "Tip: drag from a node handle".into(),
            Tone::Info,
        )),
    }
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let nodes = [
        node("1", "Input", 0.0, 40.0),
        node("2", "Process", 180.0, 0.0),
        node("3", "Validate", 180.0, 90.0),
        node("4", "Output", 360.0, 45.0),
    ];
    let edges = [
        edge("e1-2", "1", "2"),
        edge("e1-3", "1", "3"),
        edge("e2-4", "2", "4"),
        edge("e3-4", "3", "4"),
    ];
    let graph = container(node_graph(&on, &nodes, &edges).height(320.0)).id(GRAPH);
    // `<Button variant="secondary" className="ml-1 !py-1 !px-2">`: h-9,
    // px 8 inside the 1 px border.
    let tip = surface(btn_label("Tip", Size::Md))
        .height(36)
        .padding([0, 9])
        .center_y()
        .on_press(Msg::Tip)
        .style(button_style(&on, Variant::Secondary, false))
        .focus_ring(Ring::focus(&on));
    // `<p className="text-xs text-app-muted mt-2">`, the button inline
    // after a space and `ml-1`.
    let note = row![
        // Baseline-aligned with the button's label (22.25 px down the
        // 36 px line box), not centred: 2 px lower than centring.
        container(
            label("Drag nodes, pan/zoom, and connect handles.", 12.0, 16.0).color(c(p.muted))
        )
        .padding(Padding {
            top: 2.0,
            ..Padding::ZERO
        }),
        gap(7.0),
        container(tip).id(TIP),
    ]
    .align_y(Alignment::Center);
    card(cx, "Node graph")
        .body(column![graph, vgap(8.0), note])
        .into()
}

/// `BATO_SELFTEST=graph` (with `BATO_VIEW=view=examples&tab=data&card=node-graph`):
/// every interactive state of the graph through real input events. Window
/// points: the card is the last on the page, so it sits clamped at the
/// bottom (pane at 348,392; fitted zoom 730/472; nodes as in React).
pub fn selftest<M>() -> Vec<bato_kit::overlay::script::Step<M>> {
    use std::time::Duration;

    use bato_kit::iced::keyboard::key::Named;
    use bato_kit::iced::keyboard::{self, Modifiers};
    use bato_kit::iced::mouse::{self, ScrollDelta};
    use bato_kit::iced::touch::{self, Finger};
    use bato_kit::iced::{Event, Point};
    use bato_kit::overlay::script::{Step, Target};

    let at = |x: f32, y: f32| Target::At(Point::new(x, y));
    let left = mouse::Button::Left;
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let release = || Step::Event(Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)));
    let mods = |m: Modifiers| Step::Event(Event::Keyboard(keyboard::Event::ModifiersChanged(m)));
    let finger = |e: fn(Finger, Point) -> touch::Event, id: u64, x: f32, y: f32| {
        Step::Event(Event::Touch(e(Finger(id), Point::new(x, y))))
    };
    let down = |id, position| touch::Event::FingerPressed { id, position };
    let moved = |id, position| touch::Event::FingerMoved { id, position };
    let up = |id, position| touch::Event::FingerLifted { id, position };
    vec![
        Step::Shot("rest"),
        // Controls: hover (bg-app-border).
        Step::Move(at(376.0, 604.0)),
        wait(150),
        Step::Shot("control-hover"),
        // Click the Input → Validate edge: selected and focused (xyflow's
        // focus stroke outranks the accent).
        Step::Click(at(609.8, 581.9), left),
        wait(150),
        Step::Shot("edge-clicked"),
        // A pane click clears it.
        Step::Click(at(448.0, 432.0), left),
        wait(450),
        // Drag Process up and right.
        Step::Press(at(749.0, 481.0), left),
        Step::Move(at(760.0, 472.0)),
        Step::Move(at(800.0, 440.0)),
        release(),
        wait(150),
        Step::Shot("node-dragged"),
        // Connect Validate's source to Input's target (snaps within 20).
        Step::Press(at(835.6, 620.6), left),
        Step::Move(at(820.0, 610.0)),
        Step::Move(at(700.0, 580.0)),
        wait(150),
        Step::Shot("connecting"),
        Step::Move(at(392.0, 550.0)),
        wait(150),
        Step::Shot("connect-snapped"),
        release(),
        wait(150),
        Step::Shot("connected"),
        // Shift+drag a box around Validate: it and its edges select.
        mods(Modifiers::SHIFT),
        Step::Press(at(650.0, 662.0), left),
        Step::Move(at(700.0, 640.0)),
        Step::Move(at(850.0, 582.0)),
        wait(150),
        Step::Shot("box-selecting"),
        release(),
        mods(Modifiers::default()),
        wait(150),
        Step::Shot("box-selected"),
        // Backspace deletes them.
        Step::Key(Named::Backspace, Modifiers::default()),
        wait(150),
        Step::Shot("deleted"),
        // Wheel: one notch up zooms in around the pointer (the page stays).
        Step::Move(at(600.0, 500.0)),
        Step::Event(Event::Mouse(mouse::Event::WheelScrolled {
            delta: ScrollDelta::Lines { x: 0.0, y: 1.0 },
        })),
        wait(200),
        Step::Shot("wheel-zoom"),
        // Fit view.
        Step::Click(at(376.0, 656.0), left),
        wait(700),
        Step::Shot("fit-view"),
        // Pan by dragging the pane.
        Step::Press(at(448.0, 432.0), left),
        Step::Move(at(470.0, 450.0)),
        Step::Move(at(520.0, 470.0)),
        release(),
        wait(600),
        Step::Shot("panned"),
        // Double-click the pane: ×2 in 250 ms.
        Step::Click(at(560.0, 440.0), left),
        Step::Click(at(560.0, 440.0), left),
        wait(500),
        Step::Shot("dblclick-zoom"),
        // MiniMap drag pans the view.
        Step::Press(at(1035.0, 620.0), left),
        Step::Move(at(1030.0, 618.0)),
        Step::Move(at(1015.0, 612.0)),
        release(),
        wait(150),
        Step::Shot("minimap-pan"),
        // Fit, then lock (the icon changes); Shift+Tab rings Fit view.
        Step::Click(at(376.0, 656.0), left),
        Step::Click(at(376.0, 682.0), left),
        wait(150),
        Step::Shot("locked"),
        Step::Key(Named::Tab, Modifiers::SHIFT),
        wait(150),
        Step::Shot("control-focus"),
        Step::Click(at(376.0, 682.0), left),
        // Touch: two fingers pinch out (zoom in around them).
        finger(down, 1, 500.0, 450.0),
        finger(down, 2, 560.0, 450.0),
        finger(moved, 2, 600.0, 450.0),
        finger(moved, 1, 470.0, 450.0),
        finger(moved, 2, 640.0, 450.0),
        wait(150),
        Step::Shot("pinch"),
        finger(up, 2, 640.0, 450.0),
        // One finger keeps panning.
        finger(moved, 1, 440.0, 470.0),
        finger(up, 1, 440.0, 470.0),
        wait(600),
        Step::Shot("touch-pan"),
        // Tip: the toast.
        Step::Click(at(631.0, 737.0), left),
        wait(400),
        Step::Shot("tip-toast"),
    ]
}
