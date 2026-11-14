//! `Combobox` (`inputs/Combobox.tsx`): a searchable single select.
//!
//! The trigger is a control box (`controlClasses` + `sizeClasses`) showing
//! the current label (or the `muted` placeholder) and a `muted` chevron that
//! turns while open. It opens a `matchWidth` [`popover`] holding a search
//! field (autofocused, `Search…`) over the option list (`max-h-60`, scrolls):
//! typing filters by a case-insensitive substring and resets the active row;
//! ↑ ↓ move it (clamped, no wrap), Enter picks it, hovering a row makes it
//! active; the selected option carries an `accentBright` check; "No results"
//! when nothing matches. Picking closes the popover and clears the query;
//! dismissing keeps the query (React).
//!
//! Also here, shared by the pickers: [`ControlFrame`] (the control box with
//! React's `:focus` look around any content) and [`search_field`] (the small
//! search input of the Combobox and MultiSelect panels).
//!
//! ```ignore
//! combobox(&on, state.fruit.clone(), fruits(), Msg::Fruit).label("Combobox")
//! ```
//!
//! [`popover`]: crate::overlay::popover::popover

use std::rc::Rc;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, key::Named};
use iced::widget::{Space, column, container, mouse_area, row, stack, text_input};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Padding, Rectangle, Size, Vector,
    mouse, window,
};

use crate::anim::{ease_standard, opacity, with_opacity};
use crate::ctx::Ctx;
use crate::field::{ControlState, DISABLED_OPACITY, FieldSize, control_style, field};
use crate::focus::{FocusedBounds, is_pointer_press, revision};
use crate::fonts;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::layout::hline;
use crate::overlay::dropdown::chevron;
use crate::overlay::popover::{keys, local, min_fill, panel_presses, popover, popover_panel};
use crate::overlay::select::SelectOption;
use crate::scroller::scroll_area;
use crate::style::{c, rgba};
use crate::surface::{BoxStyle, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

// ── ControlFrame ────────────────────────────────────────────────────────

/// The 150 ms `transition-[border-color,box-shadow]`.
const TRANSITION: Duration = Duration::from_millis(150);

/// The control box (`controlClasses`) painted behind `content`, with the
/// `:focus` look (accentBright border, 4 px halo) whenever keyboard focus
/// is anywhere inside it, by click or by key, animated over 150 ms. The
/// content brings its own padding and behaviour (a pressable surface, a
/// text input).
pub struct ControlFrame<'a, M> {
    cx: Ctx,
    content: El<'a, M>,
    radius: f32,
    error: bool,
    disabled: bool,
    text: bool,
    on_focus: Option<M>,
    blur_on_panel_press: bool,
    in_panel: bool,
}

/// See [`ControlFrame`].
pub fn control_frame<'a, M>(cx: &Ctx, content: impl Into<El<'a, M>>) -> ControlFrame<'a, M> {
    ControlFrame {
        cx: cx.clone(),
        content: content.into(),
        radius: 8.0,
        error: false,
        disabled: false,
        text: false,
        on_focus: None,
        blur_on_panel_press: false,
        in_panel: false,
    }
}

impl<'a, M> ControlFrame<'a, M> {
    /// `rounded-md` (6) instead of `rounded-lg` (8).
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn error(mut self, on: bool) -> Self {
        self.error = on;
        self
    }
    /// `disabled:opacity-60 disabled:cursor-not-allowed`.
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// A text field inside: a middle press is the field's (no autoscroll)
    /// and Esc never reaches the input (it would blur it; it belongs to the
    /// popover or dialog).
    pub fn text(mut self, on: bool) -> Self {
        self.text = on;
        self
    }
    /// A popover trigger: a press inside any popover panel takes focus away
    /// from it (see [`crate::overlay::popover::panel_presses`]).
    pub fn blur_on_panel_press(mut self, on: bool) -> Self {
        self.blur_on_panel_press = on;
        self
    }
    /// Never scroll into view on focus. (Not needed in popover panels: a
    /// reveal from an overlay scrolls only the overlay's own scroll area.)
    pub fn in_panel(mut self) -> Self {
        self.in_panel = true;
        self
    }
    /// Published when focus enters (`onFocus`).
    pub fn on_focus(mut self, m: M) -> Self {
        self.on_focus = Some(m);
        self
    }
}

#[derive(Debug, Default)]
struct FrameState {
    focused: bool,
    from: f32,
    changed: Option<Instant>,
    seen: u64,
    /// [`panel_presses`] when last seen.
    presses: u64,
}

impl FrameState {
    fn progress(&self, now: Instant) -> f32 {
        let target = if self.focused { 1.0 } else { 0.0 };
        match self.changed {
            None => target,
            Some(t0) => {
                let k = (now.saturating_duration_since(t0).as_secs_f32()
                    / TRANSITION.as_secs_f32())
                .min(1.0);
                self.from + (target - self.from) * ease_standard(k)
            }
        }
    }
    fn running(&self, now: Instant) -> bool {
        self.changed
            .is_some_and(|t0| now.saturating_duration_since(t0) < TRANSITION)
    }
}

/// Drops the focus of every focusable it visits.
struct Unfocus;

impl widget::Operation for Unfocus {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn focusable(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn widget::operation::Focusable,
    ) {
        state.unfocus();
    }
}

fn is_escape(event: &Event) -> bool {
    matches!(
        event,
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(Named::Escape),
            ..
        })
    )
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for ControlFrame<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<FrameState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(FrameState {
            seen: revision().wrapping_sub(1),
            presses: panel_presses(),
            ..FrameState::default()
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
        renderer: &iced::Renderer,
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
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let child = layout.children().next().unwrap();
        if self.text
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle))
            )
            && cursor.is_over(layout.bounds())
        {
            shell.capture_event();
            return;
        }
        if !(self.text && is_escape(event)) {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
        let rev = revision();
        let now = Instant::now();
        let presses = panel_presses();
        let panel_press = {
            let st = tree.state.downcast_mut::<FrameState>();
            std::mem::replace(&mut st.presses, presses) != presses
        };
        if panel_press
            && self.blur_on_panel_press
            && tree.state.downcast_ref::<FrameState>().focused
        {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                child,
                renderer,
                &mut Unfocus,
            );
        }
        if tree.state.downcast_ref::<FrameState>().seen != rev
            || is_pointer_press(event)
            || panel_press
        {
            let mut op = FocusedBounds::default();
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, &mut op);
            let st = tree.state.downcast_mut::<FrameState>();
            st.seen = rev;
            let focused = op.0.is_some();
            if focused != st.focused {
                st.from = st.progress(now);
                st.focused = focused;
                st.changed = Some(now);
                shell.request_redraw();
                if focused && self.text && !self.in_panel && !is_pointer_press(event) {
                    // Focused by a key or a program: Chromium scrolls it
                    // into view (a click does not).
                    crate::scroller::focus_into_view(layout.bounds());
                }
                if focused && let Some(m) = &self.on_focus {
                    shell.publish(m.clone());
                }
            }
        }
        if let Event::Window(window::Event::RedrawRequested(at)) = event
            && tree.state.downcast_ref::<FrameState>().running(*at)
        {
            shell.request_redraw();
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if self.disabled && cursor.is_over(layout.bounds()) {
            return mouse::Interaction::NotAllowed;
        }
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
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
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<FrameState>();
        let mut boxed = control_style(
            &self.cx,
            ControlState {
                error: self.error,
                disabled: self.disabled,
                focus: st.progress(Instant::now()),
            },
        );
        boxed.radius = self.radius;
        let alpha = if self.disabled { DISABLED_OPACITY } else { 1.0 };
        with_opacity(alpha, || {
            crate::surface::paint(renderer, crate::scale::snap_rect(layout.bounds()), &boxed);
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<ControlFrame<'a, M>> for El<'a, M> {
    fn from(f: ControlFrame<'a, M>) -> Self {
        Element::new(f)
    }
}

// ── Shared pieces ───────────────────────────────────────────────────────

/// A transparent `text_input` styled like the kit's fields (text, muted
/// placeholder, accent selection).
pub fn bare_input<'a, E: Clone + 'a>(
    cx: &Ctx,
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> E + 'a,
) -> text_input::TextInput<'a, E> {
    let p = cx.p();
    let (value_c, placeholder_c, selection) = (c(p.text), c(p.muted), rgba(p.accent, 0.35));
    text_input(placeholder, value)
        .on_input(on_input)
        .font(fonts::sans())
        .size(14)
        .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
        .width(Length::Fill)
        .style(move |_theme, _status| text_input::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            icon: placeholder_c,
            placeholder: placeholder_c,
            value: value_c,
            selection,
        })
}

/// The panels' search field: `bg-app-bg/50 border-app-control rounded-md
/// pl-8 pr-2 py-1.5 text-sm`, a 14 px `muted` Search icon 10 px in, the
/// control's focus look (34 px tall).
pub fn search_field<'a, E: Clone + 'a>(
    cx: &Ctx,
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> E + 'a,
) -> El<'a, E> {
    let p = cx.p();
    let input = bare_input(cx, placeholder, value, on_input).padding(Padding {
        top: 7.0,
        bottom: 7.0,
        left: 33.0,
        right: 9.0,
    });
    let glyph = container(icon(Ico::Search, 14.0).color(c(p.muted)))
        .padding(Padding::default().left(10))
        .height(Length::Fill)
        .center_y(Length::Fill);
    control_frame(cx, stack![input, glyph].width(Length::Fill))
        .radius(6.0)
        .text(true)
        .into()
}

/// An option row (`px-3 py-1.5 text-sm gap-2.5`): `active` → `accent/15`
/// fill and `accentBright` text, else `subtext`; disabled → 40 %.
/// `focusable: false` for rows that act on press without taking focus
/// (`onMouseDown` + `preventDefault`).
pub fn option_row<'a, E: Clone + 'a>(
    cx: &Ctx,
    content: Vec<El<'a, E>>,
    active: bool,
    disabled: bool,
    focusable: bool,
    on_press: E,
    on_enter: Option<E>,
) -> El<'a, E> {
    let p = cx.p();
    let mut st = BoxStyle::default().fg(c(if active { p.accent_bright } else { p.subtext }));
    if active {
        st = st.bg(c(cx.tint(0.15)));
    }
    let s = surface(row(content).spacing(10).align_y(Alignment::Center))
        .padding(Padding::from([6.0, 12.0]))
        .width(Length::Fill)
        .fixed(st)
        .disabled(disabled)
        .focusable(focusable)
        .on_press(on_press);
    let el: El<'a, E> = match on_enter {
        Some(m) => mouse_area(s).on_enter(m).into(),
        None => s.into(),
    };
    if disabled {
        opacity(0.4, el).into()
    } else {
        el
    }
}

/// "No results" (`px-3 py-2 text-sm text-app-muted`).
pub fn no_results<'a, E: 'a>(cx: &Ctx) -> El<'a, E> {
    container(label("No results", 14.0, 20.0).color(c(cx.p().muted)))
        .padding(Padding::from([8.0, 12.0]))
        .into()
}

/// The case-insensitive substring filter: indices of matching options.
pub fn filter<T>(options: &[SelectOption<T>], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    options
        .iter()
        .enumerate()
        .filter(|(_, o)| o.label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// The panel of a search picker: the search section (`p-2 border-b`) over
/// the scrolling list (`max-h-60 py-1`).
pub fn search_panel<'a, E: Clone + 'a>(
    cx: &Ctx,
    search: El<'a, E>,
    rows: Vec<El<'a, E>>,
) -> El<'a, E> {
    let on = cx.on_card();
    let p = on.p();
    let list = scroll_area(
        &on,
        column(rows)
            .width(Length::Fill)
            .padding(Padding::from([4.0, 0.0])),
    )
    .max_height(240.0);
    min_fill(
        popover_panel(
            cx,
            column![
                container(search).padding(8).width(Length::Fill),
                hline(c(p.border)),
                list,
            ]
            .width(Length::Fill),
        )
        .padding(1)
        .width(Length::Fill),
    )
    .into()
}

// ── Combobox ────────────────────────────────────────────────────────────

pub struct Combobox<'a, T, M> {
    cx: Ctx,
    value: Option<T>,
    options: Vec<SelectOption<T>>,
    on_change: Box<dyn Fn(T) -> M + 'a>,
    placeholder: String,
    search_placeholder: String,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
    disabled: bool,
    size: FieldSize,
}

/// `<Combobox value options onChange />` on `cx` (the field's backdrop).
pub fn combobox<'a, T, M>(
    cx: &Ctx,
    value: Option<T>,
    options: Vec<SelectOption<T>>,
    on_change: impl Fn(T) -> M + 'a,
) -> Combobox<'a, T, M> {
    Combobox {
        cx: cx.clone(),
        value,
        options,
        on_change: Box::new(on_change),
        placeholder: "Select…".into(),
        search_placeholder: "Search…".into(),
        label: None,
        hint: None,
        error: None,
        required: false,
        disabled: false,
        size: FieldSize::Md,
    }
}

impl<'a, T, M> Combobox<'a, T, M> {
    pub fn placeholder(mut self, s: impl Into<String>) -> Self {
        self.placeholder = s.into();
        self
    }
    pub fn search_placeholder(mut self, s: impl Into<String>) -> Self {
        self.search_placeholder = s.into();
        self
    }
    pub fn label(mut self, s: impl Into<String>) -> Self {
        self.label = Some(s.into());
        self
    }
    pub fn hint(mut self, s: impl Into<String>) -> Self {
        self.hint = Some(s.into());
        self
    }
    pub fn error(mut self, s: Option<&str>) -> Self {
        self.error = s.map(str::to_string);
        self
    }
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
        self
    }
}

/// Transient state: open, the query, the active row (an index into the
/// filtered list).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ComboState {
    pub open: bool,
    pub query: String,
    pub active: usize,
}

#[derive(Debug, Clone)]
pub enum ComboEvent {
    Toggle,
    Close,
    Query(String),
    Active(usize),
    /// An option, by its index in `options`.
    Pick(usize),
    Key(Named),
}

/// What an event does to the state; `Some(i)` = choose option `i`.
pub fn combo_step<T>(
    s: &mut ComboState,
    options: &[SelectOption<T>],
    e: ComboEvent,
) -> Option<usize> {
    let filtered = filter(options, &s.query);
    let choose = |s: &mut ComboState, i: usize| {
        if options[i].disabled {
            return None;
        }
        s.open = false;
        s.query.clear();
        Some(i)
    };
    match e {
        ComboEvent::Toggle => {
            s.open = !s.open;
            s.active = 0;
            None
        }
        ComboEvent::Close => {
            s.open = false;
            None
        }
        ComboEvent::Query(q) => {
            s.query = q;
            s.active = 0;
            None
        }
        ComboEvent::Active(i) => {
            s.active = i;
            None
        }
        ComboEvent::Pick(i) => choose(s, i),
        ComboEvent::Key(Named::ArrowDown) => {
            s.active = (s.active + 1).min(filtered.len().saturating_sub(1));
            None
        }
        ComboEvent::Key(Named::ArrowUp) => {
            s.active = s.active.saturating_sub(1);
            None
        }
        ComboEvent::Key(Named::Enter) => filtered.get(s.active).and_then(|&i| choose(s, i)),
        ComboEvent::Key(_) => None,
    }
}

/// The trigger's content: the label (`flex-1 truncate`) and the chevron.
fn trigger_row<'a, E: 'a>(
    cx: &Ctx,
    text: &str,
    placeholder: bool,
    open: bool,
    size: FieldSize,
) -> El<'a, E> {
    let p = cx.p();
    let m = size.metrics();
    row![
        label(text.to_string(), m.text, m.line)
            .ellipsis()
            .width(Length::Fill)
            .color(c(if placeholder { p.muted } else { p.text })),
        chevron(open, 14.0).color(c(p.muted)),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// A trigger button in a control frame (`controlClasses`, `sizeClasses`).
#[allow(clippy::too_many_arguments)]
pub fn control_trigger<'a, E: Clone + 'a>(
    cx: &Ctx,
    content: El<'a, E>,
    padding: Padding,
    error: bool,
    disabled: bool,
    on_press: E,
) -> El<'a, E> {
    control_frame(
        cx,
        surface(content)
            .padding(padding)
            .width(Length::Fill)
            .disabled(disabled)
            .on_press(on_press),
    )
    .error(error)
    .disabled(disabled)
    .blur_on_panel_press(true)
    .into()
}

impl<'a, T, M> From<Combobox<'a, T, M>> for El<'a, M>
where
    T: PartialEq + Clone + 'a,
    M: Clone + 'a,
{
    fn from(cb: Combobox<'a, T, M>) -> Self {
        let Combobox {
            cx,
            value,
            options,
            on_change,
            placeholder,
            search_placeholder,
            label: lab,
            hint,
            error,
            required,
            disabled,
            size,
        } = cb;
        let options = Rc::new(options);
        let opts = options.clone();
        let fcx = cx.clone();
        let has_error = error.is_some();
        let control = local(
            move |s: &ComboState| {
                let p = cx.p();
                let current = value
                    .as_ref()
                    .and_then(|v| opts.iter().find(|o| &o.value == v));
                let m = size.metrics();
                let trigger = control_trigger(
                    &cx,
                    trigger_row(
                        &cx,
                        current.map_or(placeholder.as_str(), |o| o.label.as_str()),
                        current.is_none(),
                        s.open,
                        size,
                    ),
                    Padding::from([m.pad_y + 1.0, m.pad_x + 1.0]),
                    has_error,
                    disabled,
                    ComboEvent::Toggle,
                );
                let panel = s.open.then(|| {
                    let on = cx.on_card();
                    let filtered = filter(&opts, &s.query);
                    let rows: Vec<El<'_, ComboEvent>> = if filtered.is_empty() {
                        vec![no_results(&on)]
                    } else {
                        filtered
                            .iter()
                            .enumerate()
                            .map(|(fi, &i)| {
                                let o = &opts[i];
                                let mut items: Vec<El<'_, ComboEvent>> = Vec::new();
                                if let Some(ico) = o.icon {
                                    items.push(icon(ico, 15.0).into());
                                }
                                items.push(
                                    label(o.label.clone(), 14.0, 20.0)
                                        .ellipsis()
                                        .width(Length::Fill)
                                        .into(),
                                );
                                if value.as_ref() == Some(&o.value) {
                                    items.push(
                                        icon(Ico::Check, 15.0).color(c(p.accent_bright)).into(),
                                    );
                                }
                                option_row(
                                    &on,
                                    items,
                                    fi == s.active && !o.disabled,
                                    o.disabled,
                                    true,
                                    ComboEvent::Pick(i),
                                    Some(ComboEvent::Active(fi)),
                                )
                            })
                            .collect()
                    };
                    let search = keys(
                        crate::focus::autofocus(search_field(
                            &on,
                            &search_placeholder,
                            &s.query,
                            ComboEvent::Query,
                        )),
                        |k, _| {
                            matches!(k, Named::ArrowDown | Named::ArrowUp | Named::Enter)
                                .then_some(ComboEvent::Key(k))
                        },
                    );
                    search_panel(&cx, search.into(), rows)
                });
                popover(trigger, panel)
                    .match_width(true)
                    .on_dismiss(|_| ComboEvent::Close)
                    .into()
            },
            move |s: &mut ComboState, e| {
                combo_step(s, &options, e).map(|i| on_change(options[i].value.clone()))
            },
        );
        let mut f = field(&fcx, control)
            .required(required)
            .error(error.as_deref());
        if let Some(l) = lab {
            f = f.label(l);
        }
        if let Some(h) = hint {
            f = f.hint(h);
        }
        f.into()
    }
}

/// `Space` that fills a row (`ml-auto`).
pub fn fill<'a, E: 'a>() -> El<'a, E> {
    Space::new().width(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::select::opt;

    fn fruits() -> Vec<SelectOption<&'static str>> {
        vec![
            opt("apple", "Apple"),
            opt("banana", "Banana"),
            opt("cherry", "Cherry"),
            opt("date", "Date").disabled(true),
            opt("elderberry", "Elderberry"),
        ]
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let f = fruits();
        assert_eq!(filter(&f, ""), vec![0, 1, 2, 3, 4]);
        assert_eq!(filter(&f, "ERR"), vec![2, 4]);
        assert_eq!(filter(&f, "an"), vec![1]);
        assert!(filter(&f, "zz").is_empty());
    }

    #[test]
    fn keyboard_clamps_and_enter_picks() {
        let f = fruits();
        let mut s = ComboState::default();
        assert_eq!(combo_step(&mut s, &f, ComboEvent::Toggle), None);
        assert!(s.open);
        combo_step(&mut s, &f, ComboEvent::Key(Named::ArrowUp));
        assert_eq!(s.active, 0, "no wrap at the top");
        for _ in 0..9 {
            combo_step(&mut s, &f, ComboEvent::Key(Named::ArrowDown));
        }
        assert_eq!(s.active, 4, "no wrap at the bottom");
        combo_step(&mut s, &f, ComboEvent::Query("e".into()));
        assert_eq!(s.active, 0, "typing resets the active row");
        // "e": Apple, Cherry, Date, Elderberry → ↓ ↓ = Date (disabled).
        combo_step(&mut s, &f, ComboEvent::Key(Named::ArrowDown));
        combo_step(&mut s, &f, ComboEvent::Key(Named::ArrowDown));
        assert_eq!(combo_step(&mut s, &f, ComboEvent::Key(Named::Enter)), None);
        assert!(s.open, "a disabled option is not chosen");
        combo_step(&mut s, &f, ComboEvent::Key(Named::ArrowDown));
        assert_eq!(
            combo_step(&mut s, &f, ComboEvent::Key(Named::Enter)),
            Some(4)
        );
        assert!(!s.open);
        assert_eq!(s.query, "", "choosing clears the query");
    }

    #[test]
    fn dismissing_keeps_the_query() {
        let f = fruits();
        let mut s = ComboState::default();
        combo_step(&mut s, &f, ComboEvent::Toggle);
        combo_step(&mut s, &f, ComboEvent::Query("ch".into()));
        combo_step(&mut s, &f, ComboEvent::Close);
        assert_eq!((s.open, s.query.as_str()), (false, "ch"));
        combo_step(&mut s, &f, ComboEvent::Active(0));
        assert_eq!(combo_step(&mut s, &f, ComboEvent::Pick(2)), Some(2));
        assert_eq!(
            combo_step(&mut s, &f, ComboEvent::Key(Named::Enter)),
            Some(0)
        );
    }
}
