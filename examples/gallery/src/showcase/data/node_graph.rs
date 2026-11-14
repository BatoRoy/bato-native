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
