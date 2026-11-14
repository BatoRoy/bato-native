//! The `NodeGraph` widget: one custom widget holding the uncontrolled graph
//! (React's `useNodesState(defaultNodes)`), its viewport and every gesture.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, Key, key::Named};
use iced::mouse::{self, ScrollDelta};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Vector, touch, window};

use iced::advanced::widget::operation::Focusable as _;

use crate::focus::{FocusState, focus_visible};
use crate::label::{Align, Fam, Shaped, blink_baseline, label};
use crate::scale::snap_rect;
use crate::scroller::{CHROMIUM_TOUCHPAD_GAIN, chain};

use super::model::{self as m, Side, Viewport};
use super::paint::{self, Colors, Control};
use super::{Edge, Node};

// ── Widget state ────────────────────────────────────────────────────────

struct GNode {
    id: String,
    label: String,
    pos: Point,
    size: Size,
    selected: bool,
    focus: FocusState,
}

impl GNode {
    fn rect(&self) -> Rectangle {
        paint::r_at(self.pos, self.size)
    }
}

struct GEdge {
    source: String,
    target: String,
    selected: bool,
    focus: FocusState,
}

/// What the pointer is over, topmost first.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    /// An enabled Controls button.
    Control(usize),
    /// A disabled button (`pointer-events: none` on the panel): nothing.
    Blocked,
    Minimap,
    /// The dotted box around a box selection (drags the selection).
    Selection,
    Handle(usize, Side),
    Node(usize),
    Edge(usize),
    Pane,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HandleRef {
    node: usize,
    side: Side,
}

#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    None,
    /// d3-zoom's drag: keeps the grabbed flow point under the pointer.
    /// `pans` is false for a Ctrl+press (d3's filter refuses it; only the
    /// click remains). `far`: moved beyond the click distance.
    Pan {
        anchor: Point,
        start: Point,
        far: bool,
        edge: Option<usize>,
        pans: bool,
        finger: Option<u64>,
    },
    NodePending {
        node: Option<usize>,
        start: Point,
        drags: bool,
        finger: Option<u64>,
    },
    NodeDrag {
        items: Vec<(usize, Vector)>,
        finger: Option<u64>,
    },
    Connect {
        from: HandleRef,
        start: Point,
        started: bool,
        to: Option<HandleRef>,
        finger: Option<u64>,
    },
    Select {
        start: Point,
    },
    Minimap {
        last: Point,
        finger: Option<u64>,
    },
    Pinch {
        a: (u64, Point),
        b: (u64, Point),
    },
}

impl Gesture {
    fn finger(&self) -> Option<u64> {
        match self {
            Gesture::Pan { finger, .. }
            | Gesture::NodePending { finger, .. }
            | Gesture::NodeDrag { finger, .. }
            | Gesture::Connect { finger, .. }
            | Gesture::Minimap { finger, .. } => *finger,
            _ => None,
        }
    }
}

pub(super) struct State {
    nodes: Vec<GNode>,
    edges: Vec<GEdge>,
    vp: Viewport,
    /// The pane (inside the border), local size.
    size: Size,
    fitted: bool,
    interactive: bool,
    gesture: Gesture,
    /// The pointer, local (mouse or the gesture's finger).
    pointer: Option<Point>,
    hover: Option<Hit>,
    controls: [FocusState; 4],
    pressed: Option<(usize, Option<u64>)>,
    last_click: Option<(Instant, Point)>,
    anim: Option<(m::ZoomAnim, Instant)>,
    autopan_at: Option<Instant>,
    connect_click: Option<HandleRef>,
    /// A box selection left its dotted box around the selected nodes.
    nodes_selection: bool,
    /// The user selection rectangle while Shift-dragging (local).
    select_rect: Option<Rectangle>,
    mods: keyboard::Modifiers,
    /// The latch identity for wheel gestures (scroller chain).
    wheel_id: u64,
    /// The touch claim token.
    token: u64,
    fingers: Vec<(u64, Point)>,
    /// Labels shaped at the current zoom, by node id.
    shaped: RefCell<HashMap<String, (u32, Shaped)>>,
    /// Which item was focused at the last event (to react to Tab).
    focused: Option<Item>,
}

/// A focusable item, in Tab order: edges, nodes, Controls buttons.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Edge(usize),
    Node(usize),
    Control(usize),
}

impl State {
    fn new(nodes: &[Node], edges: &[Edge]) -> Self {
        State {
            nodes: nodes
                .iter()
                .map(|n| GNode {
                    id: n.id.clone(),
                    label: n.label.clone(),
                    pos: Point::new(n.x, n.y),
                    size: Size::new(m::NODE_MIN_W, m::NODE_LINE + 2.0 * m::NODE_PAD_Y + 2.0),
                    selected: false,
                    focus: FocusState::default(),
                })
                .collect(),
            edges: edges
                .iter()
                .map(|e| GEdge {
                    source: e.source.clone(),
                    target: e.target.clone(),
                    selected: false,
                    focus: FocusState::default(),
                })
                .collect(),
            vp: Viewport::default(),
            size: Size::ZERO,
            fitted: false,
            interactive: true,
            gesture: Gesture::None,
            pointer: None,
            hover: None,
            controls: Default::default(),
            pressed: None,
            last_click: None,
            anim: None,
            autopan_at: None,
            connect_click: None,
            nodes_selection: false,
            select_rect: None,
            mods: keyboard::Modifiers::default(),
            wheel_id: chain::next_id(),
            token: crate::focus::next_token(),
            fingers: Vec::new(),
            shaped: RefCell::new(HashMap::new()),
            focused: None,
        }
    }

    fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The edge's curve in flow coordinates.
    fn edge_curve(&self, e: &GEdge) -> Option<m::Cubic> {
        let s = self.nodes.get(self.node_index(&e.source)?)?;
        let t = self.nodes.get(self.node_index(&e.target)?)?;
        Some(m::bezier(
            m::handle_anchor(s.rect(), Side::Source),
            m::handle_anchor(t.rect(), Side::Target),
        ))
    }

    /// Node indices bottom to top (selected nodes are elevated).
    fn draw_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| !self.nodes[i].selected)
            .collect();
        v.extend((0..self.nodes.len()).filter(|&i| self.nodes[i].selected));
        v
    }

    fn nodes_bounds(&self) -> Option<Rectangle> {
        m::bounds_of(self.nodes.iter().map(GNode::rect))
    }

    fn selected_bounds(&self) -> Option<Rectangle> {
        m::bounds_of(self.nodes.iter().filter(|n| n.selected).map(GNode::rect))
    }

    fn controls_rect(&self) -> Rectangle {
        Rectangle {
            x: m::PANEL_MARGIN,
            y: self.size.height - m::PANEL_MARGIN - 4.0 * 26.0,
            width: 26.0,
            height: 4.0 * 26.0,
        }
    }

    fn minimap_rect(&self) -> Rectangle {
        Rectangle {
            x: self.size.width - m::PANEL_MARGIN - m::MINIMAP_W,
            y: self.size.height - m::PANEL_MARGIN - m::MINIMAP_H,
            width: m::MINIMAP_W,
            height: m::MINIMAP_H,
        }
    }

    fn mini_view(&self) -> m::MiniView {
        m::MiniView::new(
            self.nodes_bounds(),
            self.vp.visible(self.size.width, self.size.height),
        )
    }

    fn control_enabled(&self, i: usize) -> bool {
        match Control::ALL[i] {
            Control::ZoomIn => self.vp.zoom < m::MAX_ZOOM,
            Control::ZoomOut => self.vp.zoom > m::MIN_ZOOM,
            _ => true,
        }
    }

    fn hit(&self, p: Point, minimap: bool) -> Hit {
        let cr = self.controls_rect();
        if cr.contains(p) {
            let i = (((p.y - cr.y) / 26.0) as usize).min(3);
            return if self.control_enabled(i) {
                Hit::Control(i)
            } else {
                Hit::Blocked
            };
        }
        if minimap && self.minimap_rect().contains(p) {
            return Hit::Minimap;
        }
        let f = self.vp.to_flow(p);
        if self.nodes_selection
            && let Some(b) = self.selected_bounds()
            && b.contains(f)
        {
            return Hit::Selection;
        }
        let r = m::HANDLE / 2.0;
        for &i in self.draw_order().iter().rev() {
            let n = &self.nodes[i];
            if self.interactive {
                for side in [Side::Source, Side::Target] {
                    if f.distance(m::handle_center(n.rect(), side)) <= r {
                        return Hit::Handle(i, side);
                    }
                }
            }
            if n.rect().contains(f) {
                return Hit::Node(i);
            }
        }
        if self.interactive {
            for (i, e) in self.edges.iter().enumerate().rev() {
                if let Some(c) = self.edge_curve(e)
                    && m::cubic_distance(&c, f) <= m::EDGE_INTERACTION / 2.0
                {
                    return Hit::Edge(i);
                }
            }
        }
        Hit::Pane
    }

    // ── Selection (xyflow's store actions) ──────────────────────────────

    fn unselect_all(&mut self) {
        self.nodes.iter_mut().for_each(|n| n.selected = false);
        self.edges.iter_mut().for_each(|e| e.selected = false);
    }

    /// `addSelectedNodes`: with the multi-selection key it adds, otherwise
    /// it replaces the whole selection.
    fn select_node(&mut self, i: usize) {
        if !self.mods.control() {
            self.unselect_all();
        }
        self.nodes[i].selected = true;
    }

    /// `handleNodeClick`.
    fn node_click(&mut self, i: usize, unselect: bool) {
        self.nodes_selection = false;
        if !self.nodes[i].selected {
            self.select_node(i);
        } else if unselect || self.mods.control() {
            self.nodes[i].selected = false;
        }
    }

    /// `onEdgeClick` / the edge's selection keys.
    fn edge_click(&mut self, i: usize) {
        self.nodes_selection = false;
        if self.edges[i].selected && self.mods.control() {
            self.edges[i].selected = false;
        } else {
            if !self.mods.control() {
                self.unselect_all();
            }
            self.edges[i].selected = true;
        }
    }

    /// The box selection (`SelectionMode.Full`): nodes entirely inside,
    /// plus every edge connected to one of them.
    fn select_box(&mut self, r: Rectangle) {
        let a = self.vp.to_flow(r.position());
        let fr = Rectangle {
            x: a.x,
            y: a.y,
            width: r.width / self.vp.zoom,
            height: r.height / self.vp.zoom,
        };
        let inside = |n: &GNode| {
            let b = n.rect();
            b.x >= fr.x
                && b.y >= fr.y
                && b.x + b.width <= fr.x + fr.width
                && b.y + b.height <= fr.y + fr.height
        };
        for n in &mut self.nodes {
            n.selected = inside(n);
        }
        let ids: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| n.selected)
            .map(|n| n.id.clone())
            .collect();
        for e in &mut self.edges {
            e.selected = ids.contains(&e.source) || ids.contains(&e.target);
        }
    }

    /// Backspace: the selected nodes with their edges, and the selected edges.
    fn delete_selected(&mut self) -> bool {
        let gone: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| n.selected)
            .map(|n| n.id.clone())
            .collect();
        let before = (self.nodes.len(), self.edges.len());
        self.nodes.retain(|n| !n.selected);
        self.edges
            .retain(|e| !e.selected && !gone.contains(&e.source) && !gone.contains(&e.target));
        if !gone.is_empty() {
            self.nodes_selection = false;
        }
        self.connect_click = None;
        before != (self.nodes.len(), self.edges.len())
    }

    /// `addEdge`: no duplicate source → target edges.
    fn connect(&mut self, from: HandleRef, to: HandleRef) -> bool {
        if from.side == to.side {
            return false;
        }
        let (s, t) = match from.side {
            Side::Source => (from.node, to.node),
            Side::Target => (to.node, from.node),
        };
        let (source, target) = (self.nodes[s].id.clone(), self.nodes[t].id.clone());
        if self
            .edges
            .iter()
            .any(|e| e.source == source && e.target == target)
        {
            return false;
        }
        self.edges.push(GEdge {
            source,
            target,
            selected: false,
            focus: FocusState::default(),
        });
        true
    }

    /// The handle a connection from `from` at `p` (local) ends on: the
    /// handle under the pointer, else the closest within the radius.
    fn connection_target(&self, from: HandleRef, p: Point) -> Option<HandleRef> {
        let f = self.vp.to_flow(p);
        if let Hit::Handle(node, side) = self.hit(p, false) {
            let h = HandleRef { node, side };
            if h != from {
                return Some(h);
            }
        }
        let mut best: Option<(f32, HandleRef)> = None;
        for (i, n) in self.nodes.iter().enumerate() {
            for side in [Side::Source, Side::Target] {
                let h = HandleRef { node: i, side };
                if h == from {
                    continue;
                }
                let d = f.distance(m::handle_center(n.rect(), side));
                if d <= m::CONNECTION_RADIUS
                    && best.is_none_or(|(bd, b)| d < bd || (d == bd && b.side == from.side))
                {
                    best = Some((d, h));
                }
            }
        }
        best.map(|b| b.1)
    }

    fn items(&self) -> Vec<Item> {
        (0..self.edges.len())
            .map(Item::Edge)
            .chain((0..self.nodes.len()).map(Item::Node))
            .chain((0..4).map(Item::Control))
            .collect()
    }

    fn focus_of(&mut self, it: Item) -> &mut FocusState {
        match it {
            Item::Edge(i) => &mut self.edges[i].focus,
            Item::Node(i) => &mut self.nodes[i].focus,
            Item::Control(i) => &mut self.controls[i],
        }
    }

    fn focused_item(&self) -> Option<Item> {
        if let Some(i) = self.edges.iter().position(|e| e.focus.is_focused()) {
            return Some(Item::Edge(i));
        }
        if let Some(i) = self.nodes.iter().position(|n| n.focus.is_focused()) {
            return Some(Item::Node(i));
        }
        self.controls
            .iter()
            .position(FocusState::is_focused)
            .map(Item::Control)
    }

    /// An item's bounds, local.
    fn item_rect(&self, it: Item) -> Rectangle {
        match it {
            Item::Edge(i) => self
                .edge_curve(&self.edges[i])
                .map(|c| self.vp.rect_to_local(m::cubic_bounds(&c)))
                .unwrap_or_default(),
            Item::Node(i) => self.vp.rect_to_local(self.nodes[i].rect()),
            Item::Control(i) => {
                let cr = self.controls_rect();
                Rectangle {
                    y: cr.y + i as f32 * 26.0,
                    height: 26.0,
                    ..cr
                }
            }
        }
    }

    fn fit(&mut self) {
        if let Some(b) = self.nodes_bounds() {
            self.vp = m::fit_view(b, self.size.width, self.size.height, m::FIT_PADDING);
        }
    }

    fn activate(&mut self, i: usize) {
        let center = Point::new(self.size.width / 2.0, self.size.height / 2.0);
        self.anim = None;
        match Control::ALL[i] {
            Control::ZoomIn => self.vp = self.vp.zoom_at(self.vp.zoom * 1.2, center),
            Control::ZoomOut => self.vp = self.vp.zoom_at(self.vp.zoom / 1.2, center),
            Control::FitView => self.fit(),
            Control::Interactive => self.interactive = !self.interactive,
        }
    }

    /// Place the dragged nodes under the pointer (`p` local).
    fn drag_nodes(&mut self, p: Point) {
        if let Gesture::NodeDrag { items, .. } = &self.gesture {
            let f = self.vp.to_flow(p);
            for (i, off) in items.clone() {
                if let Some(n) = self.nodes.get_mut(i) {
                    n.pos = f + off;
                }
            }
        }
    }
}

// ── Builder ─────────────────────────────────────────────────────────────

/// See [`super::node_graph`].
pub struct NodeGraph<'a, M> {
    pub(super) nodes: Vec<Node>,
    pub(super) edges: Vec<Edge>,
    pub(super) colors: Colors,
    pub(super) height: f32,
    pub(super) minimap: bool,
    pub(super) _m: PhantomData<&'a M>,
}

impl<'a, M> NodeGraph<'a, M> {
    /// The frame height in px (`height`, default 400).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    /// Show the MiniMap (`minimap`, default true).
    pub fn minimap(mut self, on: bool) -> Self {
        self.minimap = on;
        self
    }
}

fn local(b: Rectangle, p: Point) -> Point {
    Point::new(p.x - b.x - 1.0, p.y - b.y - 1.0)
}

fn inner(b: Rectangle) -> Rectangle {
    Rectangle {
        x: b.x + 1.0,
        y: b.y + 1.0,
        width: (b.width - 2.0).max(0.0),
        height: (b.height - 2.0).max(0.0),
    }
}

fn shaped_label(text: &str, z: f32) -> Shaped {
    label(text.to_string(), m::NODE_FONT * z, m::NODE_LINE * z)
        .align(Align::Center)
        .shape(f32::INFINITY)
}

impl<'a, M> Widget<M, iced::Theme, iced::Renderer> for NodeGraph<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height))
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(&self.nodes, &self.edges))
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fixed(self.height), Size::ZERO);
        let st = tree.state.downcast_mut::<State>();
        st.size = Size::new((size.width - 2.0).max(0.0), (size.height - 2.0).max(0.0));
        // Measure the nodes once (offsetWidth: whole px).
        for n in &mut st.nodes {
            let tw = shaped_label(&n.label, 1.0).width();
            let w = (tw + 2.0 * m::NODE_PAD_X + 2.0).round().max(m::NODE_MIN_W);
            n.size = Size::new(w, m::NODE_LINE + 2.0 * m::NODE_PAD_Y + 2.0);
        }
        if !st.fitted && st.size.width > 0.0 {
            st.fitted = true;
            st.fit();
        }
        layout::Node::new(size)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let st = tree.state.downcast_mut::<State>();
        let o = inner(snap_rect(layout.bounds())).position();
        for it in st.items() {
            let r = st.item_rect(it);
            let enabled = match it {
                Item::Control(i) => st.control_enabled(i),
                _ => true,
            };
            let b = Rectangle {
                x: r.x + o.x,
                y: r.y + o.y,
                ..r
            };
            st.focus_of(it).report(operation, None, b, enabled, true);
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<State>();
        let b = snap_rect(layout.bounds());
        let pane = inner(b);
        let visible = pane.intersection(viewport);
        let over = |p: Point| visible.is_some_and(|v| v.contains(p));
        let now = Instant::now();
        let minimap = self.minimap;

        // Focus follows pointer presses (Chromium focuses tabindex elements
        // on mousedown and blurs on a press anywhere else).
        if crate::focus::is_pointer_press(event) {
            let at = match event {
                Event::Touch(touch::Event::FingerPressed { position, .. }) => Some(*position),
                _ => cursor.position(),
            };
            let hit = at
                .filter(|p| over(*p))
                .map(|p| st.hit(local(b, p), minimap));
            let target = match hit {
                Some(Hit::Control(i)) => Some(Item::Control(i)),
                Some(Hit::Node(i) | Hit::Handle(i, _)) => Some(Item::Node(i)),
                Some(Hit::Edge(i)) => Some(Item::Edge(i)),
                _ => None,
            };
            let mut changed = false;
            for it in st.items() {
                changed |= st.focus_of(it).pointer(event, target == Some(it), true);
            }
            if changed {
                shell.request_redraw();
            }
        }

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(mods)) => st.mods = *mods,
            Event::Window(window::Event::RedrawRequested(at)) => self.tick(st, *at, shell),
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let p = cursor.position().map(|p| local(b, p));
                if st.gesture.finger().is_none()
                    && !matches!(st.gesture, Gesture::Pinch { .. })
                    && let Some(p) = p
                {
                    st.pointer = Some(p);
                    self.moved(st, p, None, shell);
                }
                let hover = cursor
                    .position()
                    .filter(|p| over(*p))
                    .map(|p| st.hit(local(b, p), minimap));
                if hover != st.hover {
                    st.hover = hover;
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                if st.hover.take().is_some() {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(
                btn @ (mouse::Button::Left | mouse::Button::Middle),
            )) => {
                if shell.is_event_captured() || st.gesture != Gesture::None {
                    return;
                }
                let Some(p) = cursor.position().filter(|p| over(*p)) else {
                    return;
                };
                let p = local(b, p);
                st.pointer = Some(p);
                if *btn == mouse::Button::Middle {
                    // xyflow's filter lets the middle button pan anywhere.
                    if !matches!(
                        st.hit(p, minimap),
                        Hit::Control(_) | Hit::Blocked | Hit::Minimap
                    ) {
                        st.anim = None;
                        st.gesture = Gesture::Pan {
                            anchor: st.vp.to_flow(p),
                            start: p,
                            far: true,
                            edge: None,
                            pans: true,
                            finger: None,
                        };
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    return;
                }
                self.pressed(st, p, None, shell);
            }
            Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left | mouse::Button::Middle,
            )) => {
                if st.gesture.finger().is_some() || matches!(st.gesture, Gesture::Pinch { .. }) {
                    return;
                }
                let p = cursor.position().map(|p| local(b, p)).or(st.pointer);
                if let Some(p) = p {
                    self.released(st, p, None, now, shell);
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(pw) = cursor.position().filter(|p| over(*p)) else {
                    return;
                };
                if shell.is_event_captured() {
                    return;
                }
                let p = local(b, pw);
                let hit = st.hit(p, minimap);
                if matches!(hit, Hit::Control(_) | Hit::Blocked) {
                    // The Controls panel is outside xyflow's zoom pane: the
                    // page scrolls.
                    return;
                }
                // Latching: a gesture that started on the page stays there.
                let (sx, sy) = chain::content_shift();
                let (dx, dy) = (-sx, -sy);
                let win = Point::new(pw.x + dx, pw.y + dy);
                let kind = match delta {
                    ScrollDelta::Lines { .. } => chain::Kind::Wheel,
                    ScrollDelta::Pixels { .. } => chain::Kind::Precise,
                };
                if let ScrollDelta::Pixels { x: 0.0, y: 0.0 } = delta {
                    if chain::wheel_end(now) == Some(st.wheel_id) {
                        shell.capture_event();
                    }
                    return;
                }
                let shift = chain::shift();
                if chain::wheel_latched(kind, win, shift, now).is_some_and(|h| h != st.wheel_id) {
                    return;
                }
                let vis = visible.unwrap_or(pane);
                chain::wheel_latch(
                    st.wheel_id,
                    kind,
                    win,
                    shift,
                    now,
                    Rectangle {
                        x: vis.x + dx,
                        y: vis.y + dy,
                        ..vis
                    },
                );
                // Browser deltaY: positive scrolls down (zooms out).
                let delta_y = match delta {
                    ScrollDelta::Lines { y, .. } => -y * m::PX_PER_NOTCH,
                    ScrollDelta::Pixels { y, .. } => -y * CHROMIUM_TOUCHPAD_GAIN,
                };
                let k = st.vp.zoom * m::wheel_factor(delta_y);
                let at = if hit == Hit::Minimap {
                    Point::new(st.size.width / 2.0, st.size.height / 2.0)
                } else {
                    p
                };
                st.anim = None;
                let next = st.vp.zoom_at(k, at);
                if next != st.vp {
                    st.vp = next;
                    st.drag_nodes_after_viewport();
                    shell.request_redraw();
                }
                shell.capture_event();
            }
            Event::Touch(te) => self.touch(st, te, b, &over, now, minimap, shell),
            Event::Keyboard(
                ke @ (keyboard::Event::KeyPressed { .. } | keyboard::Event::KeyReleased { .. }),
            ) => {
                // Controls: Enter clicks on press, Space on release.
                for i in 0..4 {
                    if st.controls[i].is_focused() && st.controls[i].activation(event, shell) {
                        if st.control_enabled(i) {
                            st.activate(i);
                        }
                        shell.request_redraw();
                    }
                }
                if let keyboard::Event::KeyPressed { key, modifiers, .. } = ke {
                    self.key(st, key, *modifiers, shell);
                }
            }
            _ => {}
        }

        // Keyboard / programmatic focus moves: reveal the item, auto-pan to
        // a node that is out of view (`autoPanOnNodeFocus`), repaint the
        // edge stroke.
        let now_focused = st.focused_item();
        if now_focused != st.focused {
            st.focused = now_focused;
            shell.request_redraw();
            if let Some(Item::Node(i)) = now_focused
                && focus_visible()
            {
                let r = st.vp.rect_to_local(st.nodes[i].rect());
                let pane_r = Rectangle::new(Point::ORIGIN, st.size);
                if r.intersection(&pane_r).is_none() {
                    let c = st.nodes[i].rect().center();
                    st.vp = m::grab(c, pane_r.center(), st.vp.zoom);
                }
            }
        }
        for it in st.items() {
            let r = st.item_rect(it);
            let wb = Rectangle {
                x: r.x + pane.x,
                y: r.y + pane.y,
                ..r
            };
            st.focus_of(it).reveal(wb, shell);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let st = tree.state.downcast_ref::<State>();
        match &st.gesture {
            Gesture::Pan { pans: true, .. } | Gesture::NodeDrag { .. } | Gesture::Pinch { .. } => {
                return mouse::Interaction::Grabbing;
            }
            Gesture::Connect { started: true, .. } => return mouse::Interaction::Crosshair,
            _ => {}
        }
        let b = snap_rect(layout.bounds());
        let Some(p) = cursor.position_over(inner(b).intersection(viewport).unwrap_or_default())
        else {
            return mouse::Interaction::None;
        };
        match st.hit(local(b, p), self.minimap) {
            Hit::Control(_) => mouse::Interaction::Pointer,
            Hit::Blocked | Hit::Minimap => mouse::Interaction::Idle,
            Hit::Handle(..) => mouse::Interaction::Crosshair,
            Hit::Node(_) if st.interactive => mouse::Interaction::Grab,
            Hit::Node(_) => mouse::Interaction::Idle,
            Hit::Edge(_) => mouse::Interaction::Pointer,
            Hit::Selection | Hit::Pane => mouse::Interaction::Grab,
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let st = tree.state.downcast_ref::<State>();
        let col = &self.colors;
        let b = snap_rect(layout.bounds());
        let pane = inner(b);
        let Some(clip) = pane.intersection(viewport) else {
            return;
        };
        let o = Vector::new(pane.x, pane.y);
        let z = st.vp.zoom;
        let to_win = |p: Point| st.vp.to_local(p) + o;
        let rect_win = |r: Rectangle| {
            let l = st.vp.rect_to_local(r);
            Rectangle {
                x: l.x + o.x,
                y: l.y + o.y,
                ..l
            }
        };

        // Background and dots.
        renderer.with_layer(clip, |r| {
            paint::rect(r, pane, 11.0, col.pane, None);
            let (pts, rad) = m::dots(&st.vp, st.size.width, st.size.height);
            for p in pts {
                paint::disc(r, p + o, rad, col.dot);
            }
        });

        // Edges, then the connection line (both under the nodes).
        renderer.with_layer(clip, |r| {
            for e in &st.edges {
                let Some(c) = st.edge_curve(e) else { continue };
                let color = if e.focus.is_focused() && st.interactive {
                    col.edge_focus
                } else if e.selected {
                    col.edge_selected
                } else {
                    col.edge
                };
                paint::draw_curve(r, &c.map(to_win), z * paint::STROKE_GAIN, color, clip);
            }
            if let Gesture::Connect {
                from,
                started: true,
                to,
                ..
            } = &st.gesture
                && let (Some(n), Some(p)) = (st.nodes.get(from.node), st.pointer)
            {
                let start = m::handle_center(n.rect(), from.side);
                let valid = to.filter(|t| t.side != from.side);
                let (end, end_side) =
                    match valid.and_then(|t| st.nodes.get(t.node).map(|tn| (t, tn))) {
                        Some((t, tn)) => (m::handle_center(tn.rect(), t.side), Some(t.side)),
                        None => (st.vp.to_flow(p), None),
                    };
                let c = m::connection_curve(start, from.side, end, end_side);
                paint::draw_curve(
                    r,
                    &c.map(to_win),
                    z * paint::STROKE_GAIN,
                    col.connection,
                    clip,
                );
            }
        });

        // Nodes, each on its own layer so a node's text never shows through
        // the node above it.
        let mut shaped = st.shaped.borrow_mut();
        for i in st.draw_order() {
            let n = &st.nodes[i];
            let nb = rect_win(n.rect());
            if nb.expand(24.0 * z).intersection(&clip).is_none() {
                continue;
            }
            let entry = shaped
                .entry(n.id.clone())
                .or_insert_with(|| (z.to_bits(), shaped_label(&n.label, z)));
            if entry.0 != z.to_bits() {
                *entry = (z.to_bits(), shaped_label(&n.label, z));
            }
            let text = &entry.1;
            renderer.with_layer(clip, |r| {
                paint::node_box(r, nb, z, col);
                for side in [Side::Target, Side::Source] {
                    paint::handle(r, to_win(m::handle_center(n.rect(), side)), z, col);
                }
                // Chromium lays the line out at 14 px and scales it: the
                // baseline is the 14 px one times the zoom, then snapped.
                // Shift the box so the label's own (scaled-font) baseline
                // rounding lands there.
                let s = crate::scale::device_scale();
                let want = blink_baseline(Fam::Sans, m::NODE_FONT, m::NODE_LINE, s) / s * z;
                let own = blink_baseline(Fam::Sans, m::NODE_FONT * z, m::NODE_LINE * z, s) / s;
                let tb = Rectangle {
                    x: nb.x + (m::NODE_PAD_X + 1.0) * z,
                    y: nb.y + (m::NODE_PAD_Y + 1.0) * z + want - own,
                    width: nb.width - 2.0 * (m::NODE_PAD_X + 1.0) * z,
                    height: m::NODE_LINE * z,
                };
                text.draw(r, tb, col.node_text, &clip);
            });
        }
        shaped.retain(|id, _| st.nodes.iter().any(|n| &n.id == id));
        drop(shaped);

        // The dotted box around a box selection.
        if st.nodes_selection
            && let Some(sb) = st.selected_bounds()
        {
            renderer.with_layer(clip, |r| {
                paint::dotted_box(r, rect_win(sb), z, col.selection_bg, col.selection_border);
            });
        }

        // Controls (bottom left).
        renderer.with_layer(clip, |r| {
            let cr = st.controls_rect();
            let panel = Rectangle {
                x: cr.x + o.x,
                y: cr.y + o.y,
                ..cr
            };
            paint::shadow(r, panel, col.ctrl_shadow);
            for (i, ctl) in Control::ALL.iter().enumerate() {
                let bb = Rectangle {
                    y: panel.y + i as f32 * 26.0,
                    height: 26.0,
                    ..panel
                };
                let enabled = st.control_enabled(i);
                let hovered = enabled && st.hover == Some(Hit::Control(i));
                paint::rect(
                    r,
                    bb,
                    0.0,
                    if hovered { col.ctrl_hover } else { col.ctrl_bg },
                    None,
                );
                if i < 3 {
                    paint::rect(
                        r,
                        Rectangle {
                            y: bb.y + 25.0,
                            height: 1.0,
                            ..bb
                        },
                        0.0,
                        col.ctrl_line,
                        None,
                    );
                }
                let (h, ib) = paint::control_icon(*ctl, st.interactive, bb.position());
                paint::draw_icon(r, h, ib, col.ctrl_fg, if enabled { 1.0 } else { 0.4 });
            }
        });
        // The focused control's ring above its neighbours.
        if let Some(i) = st.controls.iter().position(FocusState::ring_visible) {
            renderer.with_layer(clip, |r| {
                let cr = st.controls_rect();
                let bb = Rectangle {
                    x: cr.x + o.x,
                    y: cr.y + o.y + i as f32 * 26.0,
                    width: 26.0,
                    height: 26.0,
                };
                paint::rect(
                    r,
                    bb.expand(2.0),
                    0.0,
                    Color::TRANSPARENT,
                    Some((2.0, col.ctrl_ring)),
                );
            });
        }

        // MiniMap (bottom right).
        if self.minimap {
            let mr = st.minimap_rect();
            let mb = Rectangle {
                x: mr.x + o.x,
                y: mr.y + o.y,
                ..mr
            };
            if let Some(mclip) = mb.intersection(&clip) {
                renderer.with_layer(mclip, |r| {
                    let mv = st.mini_view();
                    paint::rect(r, mb, 0.0, col.mini_bg, None);
                    let rad = m::MINIMAP_NODE_RADIUS * mv.scale();
                    let s = crate::scale::device_scale();
                    let snap = |v: f32| (v * s).round() / s;
                    // `shapeRendering="crispEdges"` (xyflow's choice in
                    // Chrome): whole device pixels.
                    for n in &st.nodes {
                        let nr = mv.map(n.rect());
                        let (x0, y0) = (snap(nr.x + mb.x), snap(nr.y + mb.y));
                        let (x1, y1) =
                            (snap(nr.x + nr.width + mb.x), snap(nr.y + nr.height + mb.y));
                        let nr = Rectangle {
                            x: x0,
                            y: y0,
                            width: x1 - x0,
                            height: y1 - y0,
                        };
                        paint::rect(
                            r,
                            nr,
                            rad.min(nr.width / 2.0).min(nr.height / 2.0),
                            col.mini_node,
                            None,
                        );
                    }
                    // The mask: everything but the visible area (evenodd),
                    // as four bands with device-pixel edges (no seams).
                    let vr = mv.map(st.vp.visible(st.size.width, st.size.height));
                    let x0 = snap((vr.x + mb.x).clamp(mb.x, mb.x + mb.width));
                    let x1 = snap((vr.x + vr.width + mb.x).clamp(mb.x, mb.x + mb.width));
                    let y0 = snap((vr.y + mb.y).clamp(mb.y, mb.y + mb.height));
                    let y1 = snap((vr.y + vr.height + mb.y).clamp(mb.y, mb.y + mb.height));
                    let bands = [
                        Rectangle::new(mb.position(), Size::new(mb.width, y0 - mb.y)),
                        Rectangle::new(
                            Point::new(mb.x, y1),
                            Size::new(mb.width, mb.y + mb.height - y1),
                        ),
                        Rectangle::new(Point::new(mb.x, y0), Size::new(x0 - mb.x, y1 - y0)),
                        Rectangle::new(
                            Point::new(x1, y0),
                            Size::new(mb.x + mb.width - x1, y1 - y0),
                        ),
                    ];
                    for band in bands {
                        if band.width > 0.0 && band.height > 0.0 {
                            paint::rect(r, band, 0.0, col.mini_mask, None);
                        }
                    }
                });
            }
        }

        // The user selection rectangle (above the panels, z-index 6).
        if let Some(sr) = st.select_rect {
            renderer.with_layer(clip, |r| {
                let w = Rectangle {
                    x: sr.x + o.x,
                    y: sr.y + o.y,
                    ..sr
                };
                paint::dotted_box(r, w, 1.0, col.selection_bg, col.selection_border);
            });
        }

        // `rounded-xl overflow-hidden border`: the corners outside the
        // radius in the card colour, then the frame border on top.
        if let Some(fclip) = b.intersection(viewport) {
            renderer.with_layer(fclip.expand(1.0), |r| {
                let k = 14.0;
                paint::rect(
                    r,
                    b.expand(k),
                    12.0 + k,
                    Color::TRANSPARENT,
                    Some((k, col.backdrop)),
                );
                paint::rect(r, b, 12.0, Color::TRANSPARENT, Some((1.0, col.frame)));
            });
        }
    }
}

// ── Gestures ────────────────────────────────────────────────────────────

impl<M> NodeGraph<'_, M> {
    fn pressed(&self, st: &mut State, p: Point, finger: Option<u64>, shell: &mut Shell<'_, M>) {
        let hit = st.hit(p, self.minimap);
        st.anim = None;
        let pan = |st: &mut State, edge: Option<usize>, pans: bool| Gesture::Pan {
            anchor: st.vp.to_flow(p),
            start: p,
            far: false,
            edge,
            pans,
            finger,
        };
        st.gesture = match hit {
            Hit::Control(i) => {
                st.pressed = Some((i, finger));
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            Hit::Blocked => return,
            Hit::Minimap => Gesture::Minimap { last: p, finger },
            Hit::Selection => Gesture::NodePending {
                node: None,
                start: p,
                drags: true,
                finger,
            },
            Hit::Handle(node, side) => Gesture::Connect {
                from: HandleRef { node, side },
                start: p,
                started: false,
                to: None,
                finger,
            },
            // A draggable node is `nopan`; a locked one is not.
            Hit::Node(i) if st.interactive => Gesture::NodePending {
                node: Some(i),
                start: p,
                // d3-drag ignores Ctrl+press (the click still selects).
                drags: !st.mods.control(),
                finger,
            },
            Hit::Node(_) => pan(st, None, !st.mods.control()),
            Hit::Edge(i) => pan(st, Some(i), !st.mods.control()),
            Hit::Pane if st.mods.shift() && st.interactive && finger.is_none() => {
                st.unselect_all();
                st.nodes_selection = false;
                st.select_rect = Some(Rectangle::new(p, Size::ZERO));
                Gesture::Select { start: p }
            }
            Hit::Pane => pan(st, None, !st.mods.control()),
        };
        shell.capture_event();
        shell.request_redraw();
    }

    fn moved(&self, st: &mut State, p: Point, finger: Option<u64>, shell: &mut Shell<'_, M>) {
        if st.gesture.finger() != finger {
            return;
        }
        let gesture = std::mem::replace(&mut st.gesture, Gesture::None);
        st.gesture = match gesture {
            Gesture::Pan {
                anchor,
                start,
                far,
                edge,
                pans,
                finger,
            } => {
                if pans {
                    let next = m::grab(anchor, p, st.vp.zoom);
                    if next != st.vp {
                        st.vp = next;
                        shell.request_redraw();
                    }
                }
                Gesture::Pan {
                    anchor,
                    start,
                    far: far || p.distance(start) > m::DRAG_THRESHOLD,
                    edge,
                    pans,
                    finger,
                }
            }
            Gesture::NodePending {
                node,
                start,
                drags,
                finger,
            } => {
                if drags && p.distance(start) > m::DRAG_THRESHOLD {
                    if let Some(i) = node
                        && !st.nodes[i].selected
                    {
                        // `selectNodesOnDrag`.
                        st.unselect_all();
                        st.nodes_selection = false;
                        st.nodes[i].selected = true;
                    }
                    let f = st.vp.to_flow(start);
                    let items = st
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| n.selected)
                        .map(|(i, n)| (i, n.pos - f))
                        .collect();
                    st.gesture = Gesture::NodeDrag { items, finger };
                    st.drag_nodes(p);
                    st.autopan_at = None;
                    shell.request_redraw();
                    return;
                }
                Gesture::NodePending {
                    node,
                    start,
                    drags,
                    finger,
                }
            }
            g @ Gesture::NodeDrag { .. } => {
                st.gesture = g;
                st.drag_nodes(p);
                shell.request_redraw();
                return;
            }
            Gesture::Connect {
                from,
                start,
                started,
                finger,
                ..
            } => {
                let started = started || p.distance(start) > m::DRAG_THRESHOLD;
                if started {
                    shell.request_redraw();
                }
                Gesture::Connect {
                    from,
                    start,
                    started,
                    to: started.then(|| st.connection_target(from, p)).flatten(),
                    finger,
                }
            }
            Gesture::Select { start } => {
                let r = Rectangle {
                    x: start.x.min(p.x),
                    y: start.y.min(p.y),
                    width: (p.x - start.x).abs(),
                    height: (p.y - start.y).abs(),
                };
                st.select_rect = Some(r);
                st.select_box(r);
                shell.request_redraw();
                Gesture::Select { start }
            }
            Gesture::Minimap { last, finger } => {
                let mv = st.mini_view();
                st.vp = m::minimap_pan(&st.vp, mv.view_scale, p - last);
                shell.request_redraw();
                Gesture::Minimap { last: p, finger }
            }
            g => g,
        };
    }

    fn released(
        &self,
        st: &mut State,
        p: Point,
        finger: Option<u64>,
        now: Instant,
        shell: &mut Shell<'_, M>,
    ) {
        if let Some((i, f)) = st.pressed {
            if f == finger {
                st.pressed = None;
                if st.hit(p, self.minimap) == Hit::Control(i) {
                    st.activate(i);
                }
                shell.request_redraw();
            }
            return;
        }
        if st.gesture.finger() != finger {
            return;
        }
        let gesture = std::mem::replace(&mut st.gesture, Gesture::None);
        match gesture {
            Gesture::None => return,
            Gesture::Pan { far, edge, .. } => {
                if !far {
                    match edge {
                        Some(e) => st.edge_click(e),
                        None => {
                            st.unselect_all();
                            st.nodes_selection = false;
                        }
                    }
                    // d3-zoom's dblclick (the pane and edges aren't `nopan`).
                    let dbl = st.last_click.is_some_and(|(t, q)| {
                        now.duration_since(t) <= Duration::from_millis(m::DBLCLICK_TIME_MS)
                            && q.distance(p) <= m::DBLCLICK_DIST
                    });
                    if dbl {
                        st.last_click = None;
                        let k = st.vp.zoom * if st.mods.shift() { 0.5 } else { 2.0 };
                        let to = m::clamp_zoom(k);
                        if to != st.vp.zoom {
                            st.anim = Some((
                                m::ZoomAnim {
                                    from: st.vp,
                                    to,
                                    at: p,
                                },
                                now,
                            ));
                        }
                    } else {
                        st.last_click = Some((now, p));
                    }
                }
            }
            Gesture::NodePending { node: Some(i), .. } => {
                st.last_click = None;
                st.node_click(i, false);
            }
            Gesture::NodePending { node: None, .. } => {}
            Gesture::NodeDrag { .. } => st.autopan_at = None,
            Gesture::Connect {
                from, started, to, ..
            } => {
                st.autopan_at = None;
                if started {
                    if let Some(t) = to {
                        st.connect(from, t);
                    }
                } else {
                    // `connectOnClick`: a click on one handle, then another.
                    match st.connect_click.take() {
                        Some(start) if start != from => {
                            st.connect(start, from);
                        }
                        Some(_) => {}
                        None => st.connect_click = Some(from),
                    }
                }
            }
            Gesture::Select { .. } => {
                st.select_rect = None;
                st.nodes_selection = st.nodes.iter().any(|n| n.selected);
            }
            Gesture::Minimap { .. } | Gesture::Pinch { .. } => {}
        }
        shell.request_redraw();
    }

    /// Frame callbacks: the double-click zoom transition and auto-pan.
    fn tick(&self, st: &mut State, at: Instant, shell: &mut Shell<'_, M>) {
        if let Some((anim, start)) = st.anim {
            let t = at.saturating_duration_since(start).as_secs_f32()
                / (m::DBLCLICK_MS as f32 / 1000.0);
            st.vp = anim.at_progress(t);
            if t >= 1.0 {
                st.anim = None;
            } else {
                shell.request_redraw();
            }
        }
        let dragging = matches!(
            st.gesture,
            Gesture::NodeDrag { .. } | Gesture::Connect { started: true, .. }
        );
        if dragging && let Some(p) = st.pointer {
            let v = m::autopan(p, st.size.width, st.size.height);
            if v != Vector::new(0.0, 0.0) {
                let dt = st.autopan_at.map_or(1.0, |t| {
                    (at.saturating_duration_since(t).as_secs_f32() * 60.0).min(4.0)
                });
                st.autopan_at = Some(at);
                st.vp = st.vp.pan(v * dt);
                st.drag_nodes(p);
                if let Gesture::Connect { from, .. } = st.gesture.clone() {
                    let t = st.connection_target(from, p);
                    if let Gesture::Connect { to, .. } = &mut st.gesture {
                        *to = t;
                    }
                }
                shell.request_redraw();
            } else {
                st.autopan_at = None;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn touch(
        &self,
        st: &mut State,
        te: &touch::Event,
        b: Rectangle,
        over: &dyn Fn(Point) -> bool,
        now: Instant,
        minimap: bool,
        shell: &mut Shell<'_, M>,
    ) {
        match *te {
            touch::Event::FingerPressed { id, position } => {
                if !over(position) || shell.is_event_captured() {
                    return;
                }
                let p = local(b, position);
                if st.fingers.is_empty() {
                    if st.gesture != Gesture::None || st.pressed.is_some() {
                        return;
                    }
                    let hit = st.hit(p, minimap);
                    if !matches!(hit, Hit::Control(_) | Hit::Blocked) {
                        // `touch-action: none`: the page must not scroll.
                        chain::touch_claim(id.0, st.token);
                    }
                    st.fingers.push((id.0, p));
                    st.pointer = Some(p);
                    self.pressed(st, p, Some(id.0), shell);
                    shell.capture_event();
                } else if st.fingers.len() == 1 && st.pressed.is_none() {
                    // A second finger: d3-zoom's pinch takes over whatever
                    // the first finger was doing.
                    let (f0, p0) = st.fingers[0];
                    st.fingers.push((id.0, p));
                    st.select_rect = None;
                    st.gesture = Gesture::Pinch {
                        a: (f0, st.vp.to_flow(p0)),
                        b: (id.0, st.vp.to_flow(p)),
                    };
                    st.anim = None;
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            touch::Event::FingerMoved { id, position } => {
                let Some(slot) = st.fingers.iter_mut().find(|f| f.0 == id.0) else {
                    return;
                };
                let p = local(b, position);
                slot.1 = p;
                if let Gesture::Pinch { a, b: bb } = st.gesture {
                    let pa = st.fingers.iter().find(|f| f.0 == a.0).map(|f| f.1);
                    let pb = st.fingers.iter().find(|f| f.0 == bb.0).map(|f| f.1);
                    if let (Some(pa), Some(pb)) = (pa, pb) {
                        st.vp = m::pinch(a.1, bb.1, pa, pb);
                        shell.request_redraw();
                    }
                } else {
                    st.pointer = Some(p);
                    self.moved(st, p, Some(id.0), shell);
                }
                shell.capture_event();
            }
            touch::Event::FingerLifted { id, position } => {
                if !st.fingers.iter().any(|f| f.0 == id.0) {
                    if st.pressed.is_some_and(|(_, f)| f == Some(id.0)) {
                        self.released(st, local(b, position), Some(id.0), now, shell);
                    }
                    return;
                }
                chain::touch_release(id.0, st.token);
                let p = local(b, position);
                st.fingers.retain(|f| f.0 != id.0);
                if let Gesture::Pinch { .. } = st.gesture {
                    // The remaining finger pans from where it is.
                    st.gesture = match st.fingers.first() {
                        Some(&(f, q)) => Gesture::Pan {
                            anchor: st.vp.to_flow(q),
                            start: q,
                            far: true,
                            edge: None,
                            pans: true,
                            finger: Some(f),
                        },
                        None => Gesture::None,
                    };
                } else {
                    self.released(st, p, Some(id.0), now, shell);
                }
                shell.capture_event();
            }
            touch::Event::FingerLost { id, .. } => {
                if chain::is_synthetic_lost() && chain::touch_holder(id.0) == Some(st.token) {
                    return;
                }
                if st.pressed.is_some_and(|(_, f)| f == Some(id.0)) {
                    st.pressed = None;
                    shell.request_redraw();
                }
                if st.fingers.iter().any(|f| f.0 == id.0) {
                    chain::touch_release(id.0, st.token);
                    st.fingers.retain(|f| f.0 != id.0);
                    st.gesture = Gesture::None;
                    st.select_rect = None;
                    shell.request_redraw();
                }
            }
        }
    }

    fn key(&self, st: &mut State, key: &Key, mods: keyboard::Modifiers, shell: &mut Shell<'_, M>) {
        if shell.is_event_captured() {
            return;
        }
        let named = match key {
            Key::Named(n) => Some(*n),
            Key::Character(c) if c.as_str() == " " => Some(Named::Space),
            _ => None,
        };
        let Some(named) = named else { return };
        // The focused node / edge: the selection keys and arrow moves.
        match st.focused_item() {
            Some(Item::Node(i)) => match named {
                Named::Enter | Named::Space | Named::Escape if st.interactive => {
                    st.node_click(i, named == Named::Escape);
                    if named == Named::Escape {
                        st.nodes[i].focus.unfocus();
                    }
                    shell.request_redraw();
                    return;
                }
                Named::ArrowUp | Named::ArrowDown | Named::ArrowLeft | Named::ArrowRight
                    if st.interactive && st.nodes[i].selected =>
                {
                    let f = if mods.shift() { 4.0 } else { 1.0 } * 5.0;
                    let d = match named {
                        Named::ArrowUp => Vector::new(0.0, -f),
                        Named::ArrowDown => Vector::new(0.0, f),
                        Named::ArrowLeft => Vector::new(-f, 0.0),
                        _ => Vector::new(f, 0.0),
                    };
                    for n in st.nodes.iter_mut().filter(|n| n.selected) {
                        n.pos += d;
                    }
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
                _ => {}
            },
            Some(Item::Edge(i))
                if matches!(named, Named::Enter | Named::Space | Named::Escape)
                    && st.interactive =>
            {
                if named == Named::Escape {
                    st.edges[i].selected = false;
                    st.edges[i].focus.unfocus();
                } else {
                    st.edge_click(i);
                }
                shell.request_redraw();
                return;
            }
            _ => {}
        }
        // `deleteKeyCode`: Backspace anywhere (not while typing).
        if named == Named::Backspace && st.delete_selected() {
            shell.capture_event();
            shell.request_redraw();
        }
    }
}

impl State {
    /// After a wheel zoom during a node drag the nodes stay under the
    /// pointer.
    fn drag_nodes_after_viewport(&mut self) {
        if let Some(p) = self.pointer {
            self.drag_nodes(p);
        }
    }
}

impl<'a, M: 'a> From<NodeGraph<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(g: NodeGraph<'a, M>) -> Self {
        Element::new(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{edge, node};

    /// The Showcase graph, fitted into the reference pane (802 × 318).
    fn showcase() -> State {
        let mut st = State::new(
            &[
                node("1", "Input", 0.0, 40.0),
                node("2", "Process", 180.0, 0.0),
                node("3", "Validate", 180.0, 90.0),
                node("4", "Output", 360.0, 45.0),
            ],
            &[
                edge("e1-2", "1", "2"),
                edge("e1-3", "1", "3"),
                edge("e2-4", "2", "4"),
                edge("e3-4", "3", "4"),
            ],
        );
        st.size = Size::new(802.0, 318.0);
        st.fit();
        st
    }

    fn local_of(st: &State, f: Point) -> Point {
        st.vp.to_local(f)
    }

    #[test]
    fn hit_order_is_xyflows() {
        let st = showcase();
        // Node body, its handles, an edge, the pane, the panels.
        assert_eq!(
            st.hit(local_of(&st, Point::new(56.0, 59.0)), true),
            Hit::Node(0)
        );
        assert_eq!(
            st.hit(local_of(&st, Point::new(112.0, 59.0)), true),
            Hit::Handle(0, Side::Source)
        );
        assert_eq!(
            st.hit(local_of(&st, Point::new(180.0, 19.0)), true),
            Hit::Handle(1, Side::Target)
        );
        let c = st.edge_curve(&st.edges[0]).unwrap();
        assert_eq!(
            st.hit(local_of(&st, m::cubic_at(&c, 0.5)), true),
            Hit::Edge(0)
        );
        assert_eq!(st.hit(Point::new(400.0, 10.0), true), Hit::Pane);
        assert_eq!(
            st.hit(Point::new(20.0, 318.0 - 15.0 - 100.0), true),
            Hit::Control(0)
        );
        assert_eq!(st.hit(Point::new(700.0, 250.0), true), Hit::Minimap);
        assert_eq!(st.hit(Point::new(700.0, 250.0), false), Hit::Pane);
    }

    #[test]
    fn locked_graph_has_no_handles_or_edges_to_hit() {
        let mut st = showcase();
        st.activate(3);
        assert!(!st.interactive);
        let c = st.edge_curve(&st.edges[0]).unwrap();
        assert_eq!(st.hit(local_of(&st, m::cubic_at(&c, 0.5)), true), Hit::Pane);
        assert_eq!(
            st.hit(local_of(&st, Point::new(111.0, 59.0)), true),
            Hit::Node(0)
        );
    }

    #[test]
    fn clicks_replace_the_selection_unless_ctrl() {
        let mut st = showcase();
        st.node_click(0, false);
        st.edge_click(2);
        assert!(!st.nodes[0].selected && st.edges[2].selected);
        st.mods = keyboard::Modifiers::CTRL;
        st.node_click(1, false);
        assert!(st.nodes[1].selected && st.edges[2].selected);
        // Ctrl+click on a selected node unselects it.
        st.node_click(1, false);
        assert!(!st.nodes[1].selected);
    }

    #[test]
    fn box_selection_takes_nodes_inside_and_their_edges() {
        let mut st = showcase();
        // Around Process only.
        let a = local_of(&st, Point::new(170.0, -5.0));
        let b = local_of(&st, Point::new(300.0, 45.0));
        st.select_box(Rectangle::new(a, Size::new(b.x - a.x, b.y - a.y)));
        let sel: Vec<bool> = st.nodes.iter().map(|n| n.selected).collect();
        assert_eq!(sel, [false, true, false, false]);
        let esel: Vec<bool> = st.edges.iter().map(|e| e.selected).collect();
        assert_eq!(esel, [true, false, true, false]);
    }

    #[test]
    fn backspace_deletes_nodes_with_their_edges() {
        let mut st = showcase();
        st.node_click(1, false);
        assert!(st.delete_selected());
        assert_eq!(st.nodes.len(), 3);
        assert_eq!(st.edges.len(), 2);
        assert!(!st.delete_selected());
        st.edge_click(0);
        assert!(st.delete_selected());
        assert_eq!(st.edges.len(), 1);
    }

    #[test]
    fn connect_is_source_to_target_without_duplicates() {
        let mut st = showcase();
        let h = |node, side| HandleRef { node, side };
        // Same side: refused (strict mode).
        assert!(!st.connect(h(0, Side::Source), h(3, Side::Source)));
        // Input → Output, dragged from either end.
        assert!(st.connect(h(3, Side::Target), h(0, Side::Source)));
        assert_eq!(
            (st.edges[4].source.as_str(), st.edges[4].target.as_str()),
            ("1", "4")
        );
        assert!(!st.connect(h(0, Side::Source), h(3, Side::Target)));
        assert!(!st.connect(h(0, Side::Source), h(1, Side::Target)));
    }

    #[test]
    fn connection_snaps_to_the_closest_handle_within_the_radius() {
        let st = showcase();
        let from = HandleRef {
            node: 0,
            side: Side::Source,
        };
        let near = local_of(&st, Point::new(180.0 - 12.0, 19.0 + 5.0));
        assert_eq!(
            st.connection_target(from, near),
            Some(HandleRef {
                node: 1,
                side: Side::Target
            })
        );
        let far = local_of(&st, Point::new(250.0, 60.0));
        assert_eq!(st.connection_target(from, far), None);
    }

    #[test]
    fn controls_zoom_around_the_centre_and_disable_at_the_limits() {
        let mut st = showcase();
        let c = Point::new(401.0, 159.0);
        let f = st.vp.to_flow(c);
        st.activate(0);
        let back = st.vp.to_local(f);
        assert!((back.x - c.x).abs() < 1e-3 && (back.y - c.y).abs() < 1e-3);
        for _ in 0..10 {
            st.activate(0);
        }
        assert_eq!(st.vp.zoom, m::MAX_ZOOM);
        assert!(!st.control_enabled(0) && st.control_enabled(1));
        assert_eq!(
            st.hit(Point::new(20.0, 318.0 - 15.0 - 100.0), true),
            Hit::Blocked
        );
        st.activate(2);
        assert!((st.vp.zoom - 730.0 / 472.0).abs() < 1e-4);
    }

    #[test]
    fn tab_order_is_edges_nodes_controls() {
        let st = showcase();
        let items = st.items();
        assert_eq!(items.len(), 4 + 4 + 4);
        assert_eq!(items[0], Item::Edge(0));
        assert_eq!(items[4], Item::Node(0));
        assert_eq!(items[8], Item::Control(0));
    }

    #[test]
    fn selected_nodes_draw_last() {
        let mut st = showcase();
        st.node_click(0, false);
        assert_eq!(st.draw_order(), vec![1, 2, 3, 0]);
    }
}
