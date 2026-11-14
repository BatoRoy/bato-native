//! `NodeGraph` (§4.2): the flow editor of `components/data/NodeGraph.tsx`
//! (@xyflow/react 12 with the themed node, `<Background dots gap 16>`,
//! `<Controls/>` and a pannable, zoomable `<MiniMap/>`), as one widget.
//!
//! Uncontrolled like React's `defaultNodes` / `defaultEdges`: the graph lives
//! in widget state, seeded once, and follows the user (drag, connect,
//! delete) until the widget leaves the tree.
//!
//! | Input | Does |
//! |---|---|
//! | drag the pane (mouse, one finger) | pan |
//! | wheel, Ctrl+wheel, touchpad, two fingers | zoom around the pointer (0.5–2) |
//! | double-click / double-tap the pane (Shift: out) | zoom ×2 in 250 ms |
//! | drag a node | move it (with the other selected nodes) |
//! | drag from a handle to another (or click one, then the other) | new edge |
//! | click a node / edge, Ctrl adds; Shift+drag the pane | select; box select |
//! | Backspace | delete the selection |
//! | Tab | edges, nodes, then the Controls buttons; Enter/Space select, arrows move |
//! | Controls | zoom in, zoom out, fit view, lock (no drag / connect / select) |
//! | MiniMap | drag pans, wheel zooms around the pane centre |
//!
//! **Drawing.** Nodes are laid out at `zoom × size` (boxes and handles as
//! quads, labels shaped at `14 × zoom` px), never a scaled bitmap, so text
//! stays crisp at every zoom like Chromium's re-rasterized `scale()` layer.
//! Edges are Bezier strokes rasterized by resvg (see [`paint`]).
//!
//! ```ignore
//! use bato_kit::graph::{edge, node, node_graph};
//! node_graph(&cx.on_card(), &[node("1", "Input", 0.0, 40.0)], &[]).height(320.0)
//! ```

mod model;
pub mod paint;
mod widget;

use std::marker::PhantomData;

pub use model::{MAX_ZOOM, MIN_ZOOM, Viewport};
pub use widget::NodeGraph;

use crate::ctx::Ctx;

/// A node: `{ id, position: { x, y }, data: { label } }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
}

pub fn node(id: impl Into<String>, label: impl Into<String>, x: f32, y: f32) -> Node {
    Node {
        id: id.into(),
        label: label.into(),
        x,
        y,
    }
}

/// An edge between two nodes' handles (source on the right of `source`,
/// target on the left of `target`).
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
}

pub fn edge(id: impl Into<String>, source: impl Into<String>, target: impl Into<String>) -> Edge {
    Edge {
        id: id.into(),
        source: source.into(),
        target: target.into(),
    }
}

/// A node graph on `cx`'s backdrop (the card it sits in), 400 px tall with
/// a MiniMap by default.
pub fn node_graph<'a, M>(cx: &Ctx, nodes: &[Node], edges: &[Edge]) -> NodeGraph<'a, M> {
    NodeGraph {
        nodes: nodes.to_vec(),
        edges: edges.to_vec(),
        colors: paint::Colors::new(cx),
        height: 400.0,
        minimap: true,
        _m: PhantomData,
    }
}
