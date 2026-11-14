//! `Field` and the control style (INF-10): `inputs/Field.tsx` and the box
//! every text control shares (`controlClasses` + `sizeClasses`).
//!
//! - [`field`]: the label / hint / error scaffold around any control: a
//!   column with 6 px gaps; label 12/16 medium `subtext` with a red `*` 2 px
//!   after it when required; then the control; then the error (12/16 `red`,
//!   wins) or the hint (12/16 `muted`). An optional counter (TextField's
//!   `showCount`) sits right-aligned under the control.
//! - [`control_style`]: the box as a [`BoxStyle`]: fill `bg@.5`, 1 px
//!   `control` border, radius 8; focused: `accentBright` border and a 4 px
//!   `accent@.15` halo; error: `red` border, `red@.15` halo. `focused` is an
//!   input: the focus system decides it.
//! - [`text_control`]: an iced `text_input` in that box, with left/right
//!   adornment slots placed like TextField's (`left-3` icon or prefix with
//!   the text at `pl-9`; a `right-2.5` group with the text ending at
//!   `pr-9`), the 150 ms border/halo transition, disabled `opacity .6` and
//!   `not-allowed`, and the three sizes (sm 30, md 38, lg 42 px tall).
//!
//! ```ignore
//! field(&on, text_control(&on, "you@example.com", &email, Msg::Email).left_icon(Ico::Mail))
//!     .label("Email").required(true).error(self.email_error.as_deref())
//! field(&on, text_control(&on, "0.00", &amount, Msg::Amount).prefix("$").suffix("USD"))
//!     .label("Amount").hint("Before tax")
//! ```
//!
//! Focus visuals follow `text_input`'s own focus until the focus system
//! drives them: [`Control::focused`] overrides the detection.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{column, container, row, text_input};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Padding, Point, Rectangle, Size,
    Vector, mouse, window,
};

use crate::El;
use crate::anim::{ease_standard, with_opacity};
use crate::ctx::Ctx;
use crate::fonts;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::{c, rgba};
use crate::surface::{BoxStyle, paint};

type P = <iced::Renderer as TextRenderer>::Paragraph;

/// `sizeClasses`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldSize {
    /// `px-2.5 py-1.5 text-xs`: 30 px.
    Sm,
    /// `px-3 py-2 text-sm`: 38 px.
    #[default]
    Md,
    /// `px-3.5 py-2.5 text-sm`: 42 px.
    Lg,
}

/// A control's box metrics (CSS px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub pad_x: f32,
    pub pad_y: f32,
    pub text: f32,
    pub line: f32,
}

impl Metrics {
    /// Border-box height (1 px borders included).
    pub fn height(&self) -> f32 {
        self.line + 2.0 * self.pad_y + 2.0
    }
}

impl FieldSize {
    pub fn metrics(self) -> Metrics {
        match self {
            FieldSize::Sm => Metrics {
                pad_x: 10.0,
                pad_y: 6.0,
                text: 12.0,
                line: 16.0,
            },
            FieldSize::Md => Metrics {
                pad_x: 12.0,
                pad_y: 8.0,
                text: 14.0,
                line: 20.0,
            },
            FieldSize::Lg => Metrics {
                pad_x: 14.0,
                pad_y: 10.0,
                text: 14.0,
                line: 20.0,
            },
        }
    }
}

/// What the control box depends on.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ControlState {
    pub error: bool,
    pub disabled: bool,
    /// 0 = at rest, 1 = focused; in between during the 150 ms transition.
    pub focus: f32,
}

/// Disabled controls: `disabled:opacity-60`.
pub const DISABLED_OPACITY: f32 = 0.6;

/// `controlClasses(error)` on `cx`'s backdrop. Disabled opacity is not
/// baked in: draw inside [`with_opacity`]`(DISABLED_OPACITY, ..)`.
pub fn control_style(cx: &Ctx, st: ControlState) -> BoxStyle {
    let p = cx.p();
    let t = st.focus.clamp(0.0, 1.0);
    let (rest, focus, halo) = if st.error {
        (p.red, p.red, p.red)
    } else {
        (p.control, p.accent_bright, p.accent)
    };
    let border = if t >= 1.0 {
        c(focus)
    } else {
        crate::style::lerp(c(rest), c(focus), t)
    };
    let mut s = BoxStyle::default()
        .bg(c(cx.recessed()))
        .border(border)
        .radius(8.0);
    if t > 0.0 {
        // `0 0 0 4px rgb(…/.15)` from `0 0 #0000`: spread and alpha grow
        // together.
        s = s.ring(4.0 * t, c(cx.blend(halo, 0.15 * t as f64)));
    }
    s
}

/// The colours the `text_input` inside a control draws with.
#[derive(Debug, Clone, Copy)]
struct TextColors {
    value: Color,
    placeholder: Color,
    selection: Color,
}

fn text_colors(cx: &Ctx, disabled: bool) -> TextColors {
    let p = cx.p();
    // `text_input` ignores group opacity: fade by hand over the backdrop.
    let fade = |rgb| {
        if disabled {
            c(cx.blend(rgb, DISABLED_OPACITY as f64))
        } else {
            c(rgb)
        }
    };
    TextColors {
        value: fade(p.text),
        placeholder: fade(p.muted),
        selection: rgba(p.accent, 0.35),
    }
}

// ── Field scaffold ──────────────────────────────────────────────────────

pub struct Field<'a, M> {
    cx: Ctx,
    control: El<'a, M>,
    label: Option<String>,
    required: bool,
    hint: Option<String>,
    error: Option<String>,
    counter: Option<String>,
}

/// The label / hint / error scaffold around `control` (build the control on
/// the same `cx`).
pub fn field<'a, M: 'a>(cx: &Ctx, control: impl Into<El<'a, M>>) -> Field<'a, M> {
    Field {
        cx: cx.clone(),
        control: control.into(),
        label: None,
        required: false,
        hint: None,
        error: None,
        counter: None,
    }
}

impl<'a, M: 'a> Field<'a, M> {
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    /// The red `*` after the label.
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
    pub fn hint(mut self, text: impl Into<String>) -> Self {
        self.hint = Some(text.into());
        self
    }
    /// Shown in red instead of the hint (React has no validation framework:
    /// an `error` prop, here an `Option`).
    pub fn error(mut self, text: Option<&str>) -> Self {
        self.error = text.map(str::to_string);
        self
    }
    /// Right-aligned under the control (`showCount`: "3/20").
    pub fn counter(mut self, text: impl Into<String>) -> Self {
        self.counter = Some(text.into());
        self
    }
}

impl<'a, M: 'a> From<Field<'a, M>> for El<'a, M> {
    fn from(f: Field<'a, M>) -> Self {
        let p = f.cx.p();
        let mut col = column![].spacing(6).width(Length::Fill);
        if let Some(text) = f.label {
            let l = label(text, 12.0, 16.0).medium().color(c(p.subtext));
            col = col.push(if f.required {
                El::from(row![
                    l,
                    container(label("*", 12.0, 16.0).medium().color(c(p.red)))
                        .padding(Padding::default().left(2))
                ])
            } else {
                l.into()
            });
        }
        col = col.push(f.control);
        if let Some(n) = f.counter {
            col = col.push(
                container(label(n, 12.0, 16.0).color(c(p.muted)))
                    .width(Length::Fill)
                    .align_x(Alignment::End),
            );
        }
        if let Some(e) = f.error {
            col = col.push(label(e, 12.0, 16.0).color(c(p.red)).wrap());
        } else if let Some(h) = f.hint {
            col = col.push(label(h, 12.0, 16.0).color(c(p.muted)).wrap());
        }
        col.into()
    }
}

// ── Text control ────────────────────────────────────────────────────────

pub struct Control<'a, M> {
    cx: Ctx,
    placeholder: String,
    value: String,
    on_input: Option<Box<dyn Fn(String) -> M + 'a>>,
    on_submit: Option<M>,
    id: Option<widget::Id>,
    font: iced::Font,
    size: FieldSize,
    error: bool,
    disabled: bool,
    focused: Option<bool>,
    secure: bool,
    left: Option<El<'a, M>>,
    right: Vec<El<'a, M>>,
    width: Length,
}

/// A text input in the control box: Inter, md (38 px), full width.
pub fn text_control<'a, M: Clone + 'a>(
    cx: &Ctx,
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> M + 'a,
) -> Control<'a, M> {
    Control {
        cx: cx.clone(),
        placeholder: placeholder.to_string(),
        value: value.to_string(),
        on_input: Some(Box::new(on_input)),
        on_submit: None,
        id: None,
        font: fonts::sans(),
        size: FieldSize::Md,
        error: false,
        disabled: false,
        focused: None,
        secure: false,
        left: None,
        right: Vec::new(),
        width: Length::Fill,
    }
}

impl<'a, M: Clone + 'a> Control<'a, M> {
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
        self
    }
    /// Red border and halo.
    pub fn error(mut self, on: bool) -> Self {
        self.error = on;
        self
    }
    /// `opacity .6`, `not-allowed`, not editable.
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// Force the focus look (the focus system's input); `None` follows the
    /// `text_input`'s own focus.
    pub fn focused(mut self, f: Option<bool>) -> Self {
        self.focused = f;
        self
    }
    pub fn mono(mut self) -> Self {
        self.font = fonts::mono();
        self
    }
    pub fn secure(mut self, on: bool) -> Self {
        self.secure = on;
        self
    }
    pub fn on_submit(mut self, m: M) -> Self {
        self.on_submit = Some(m);
        self
    }
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    /// The left slot (`absolute left-3`; the text starts at 36 px).
    pub fn left(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.left = Some(el.into());
        self
    }
    /// A 15 px `muted` icon in the left slot (`leftIcon`).
    pub fn left_icon(self, ico: Ico) -> Self {
        let m = c(self.cx.p().muted);
        self.left(icon(ico, 15.0).color(m))
    }
    /// `prefix`: 14/20 `muted` text in the left slot.
    pub fn prefix(self, text: &str) -> Self {
        let m = c(self.cx.p().muted);
        self.left(label(text.to_string(), 14.0, 20.0).color(m))
    }
    /// Append to the right group (`absolute right-2.5 gap-1`; the text ends
    /// 36 px from the right edge).
    pub fn right(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.right.push(el.into());
        self
    }
    /// A 15 px `muted` icon in the right group (`rightIcon`).
    pub fn right_icon(self, ico: Ico) -> Self {
        let m = c(self.cx.p().muted);
        self.right(icon(ico, 15.0).color(m))
    }
    /// `suffix`: 14/20 `muted` text in the right group.
    pub fn suffix(self, text: &str) -> Self {
        let m = c(self.cx.p().muted);
        self.right(label(text.to_string(), 14.0, 20.0).color(m))
    }
}

/// The text padding inside a control: `(left, right)` border-box px.
pub fn text_insets(size: FieldSize, left: bool, right: bool) -> (f32, f32) {
    let m = size.metrics();
    (
        1.0 + if left { 36.0 } else { m.pad_x },
        1.0 + if right { 36.0 } else { m.pad_x },
    )
}

impl<'a, M: Clone + 'a> From<Control<'a, M>> for El<'a, M> {
    fn from(f: Control<'a, M>) -> Self {
        let m = f.size.metrics();
        let h = m.height();
        let (pl, pr) = text_insets(f.size, f.left.is_some(), !f.right.is_empty());
        let colors = text_colors(&f.cx, f.disabled);
        let mut input = text_input(&f.placeholder, &f.value)
            .font(f.font)
            .size(m.text)
            .line_height(iced::widget::text::LineHeight::Absolute(m.line.into()))
            .padding(Padding {
                top: (h - m.line) / 2.0,
                bottom: (h - m.line) / 2.0,
                left: pl,
                right: pr,
            })
            .width(Length::Fill)
            .secure(f.secure)
            .style(move |_theme, _status| text_input::Style {
                background: Background::Color(Color::TRANSPARENT),
                border: Border::default(),
                icon: colors.placeholder,
                placeholder: colors.placeholder,
                value: colors.value,
                selection: colors.selection,
            });
        if !f.disabled
            && let Some(on_input) = f.on_input
        {
            input = input.on_input(on_input);
        }
        if let Some(msg) = f.on_submit {
            input = input.on_submit(msg);
        }
        if let Some(id) = f.id {
            input = input.id(id);
        }
        let right = (!f.right.is_empty()).then(|| {
            El::from(
                iced::widget::Row::with_children(f.right)
                    .spacing(4)
                    .align_y(Alignment::Center),
            )
        });
        Element::new(ControlBox {
            cx: f.cx,
            input: input.into(),
            left: f.left,
            right,
            height: h,
            width: f.width,
            error: f.error,
            disabled: f.disabled,
            focused: f.focused,
        })
    }
}

/// The 150 ms `transition-[border-color,box-shadow]`.
const TRANSITION: Duration = Duration::from_millis(150);

#[derive(Debug, Default)]
struct BoxState {
    focused: bool,
    from: f32,
    changed: Option<Instant>,
    /// Focus system identity (restore, Tab starting point).
    token: u64,
}

impl BoxState {
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

/// The control widget: box, focus transition, adornments, pixel snapping,
/// middle-press capture.
struct ControlBox<'a, M> {
    cx: Ctx,
    input: El<'a, M>,
    left: Option<El<'a, M>>,
    right: Option<El<'a, M>>,
    height: f32,
    width: Length,
    error: bool,
    disabled: bool,
    focused: Option<bool>,
}

impl<'a, M> ControlBox<'a, M> {
    /// (child index, element) of the adornments.
    fn slots(&self) -> impl Iterator<Item = (usize, &El<'a, M>)> {
        let l = self.left.iter().map(|e| (1, e));
        let r = self
            .right
            .iter()
            .map(|e| (if self.left.is_some() { 2 } else { 1 }, e));
        l.chain(r)
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for ControlBox<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Fixed(self.height))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<BoxState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(BoxState {
            token: crate::focus::next_token(),
            ..BoxState::default()
        })
    }
    fn children(&self) -> Vec<Tree> {
        let mut v = vec![Tree::new(&self.input)];
        v.extend(self.slots().map(|(_, e)| Tree::new(e)));
        v
    }
    fn diff(&self, tree: &mut Tree) {
        let mut els: Vec<&El<'a, M>> = vec![&self.input];
        els.extend(self.slots().map(|(_, e)| e));
        tree.diff_children(&els);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        let w = limits.max().width;
        let h = self.height;
        let exact = layout::Limits::new(Size::new(w, h), Size::new(w, h));
        let input = self
            .input
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &exact);
        let mut nodes = vec![input];
        let loose = layout::Limits::new(Size::ZERO, Size::new(w, h));
        let mut i = 1;
        if let Some(l) = &mut self.left {
            let n = l
                .as_widget_mut()
                .layout(&mut tree.children[i], renderer, &loose);
            let y = ((h - n.size().height) / 2.0).round();
            nodes.push(n.move_to(Point::new(12.0, y)));
            i += 1;
        }
        if let Some(r) = &mut self.right {
            let n = r
                .as_widget_mut()
                .layout(&mut tree.children[i], renderer, &loose);
            let y = ((h - n.size().height) / 2.0).round();
            let x = w - 10.0 - n.size().width;
            nodes.push(n.move_to(Point::new(x, y)));
        }
        layout::Node::with_children(Size::new(w, h), nodes)
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
        // `data-batoscroll-ignore`: a middle press on a text field is the
        // field's (primary paste), never an autoscroll.
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle))
        ) && cursor.is_over(layout.bounds())
        {
            shell.capture_event();
            return;
        }
        if crate::focus::is_pointer_press(event) && cursor.is_over(layout.bounds()) {
            crate::focus::note_press(tree.state.downcast_ref::<BoxState>().token);
        }
        // Esc never blurs a field (Chromium); it is left for the dialog.
        let esc = matches!(
            event,
            Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            })
        );
        // The right group's buttons (clear, reveal) first, then the input:
        // icons and prefixes are `pointer-events-none`, the input spans the box.
        let kids: Vec<_> = layout.children().collect();
        if let Some(r) = &mut self.right {
            let i = kids.len() - 1;
            r.as_widget_mut().update(
                &mut tree.children[i],
                event,
                kids[i],
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
        if !shell.is_event_captured() && !esc {
            self.input.as_widget_mut().update(
                &mut tree.children[0],
                event,
                kids[0],
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
        let focused = self.focused.unwrap_or_else(|| {
            tree.children[0]
                .state
                .downcast_ref::<text_input::State<P>>()
                .is_focused()
        });
        let now = Instant::now();
        let st = tree.state.downcast_mut::<BoxState>();
        if focused && !st.focused && !crate::focus::is_pointer_press(event) {
            crate::scroller::focus_into_view(layout.bounds());
        }
        if focused != st.focused {
            st.from = st.progress(now);
            st.focused = focused;
            st.changed = Some(now);
            shell.request_redraw();
        } else if let Event::Window(window::Event::RedrawRequested(at)) = event
            && st.running(*at)
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
        if !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        if self.disabled {
            return mouse::Interaction::NotAllowed;
        }
        let kids: Vec<_> = layout.children().collect();
        if let Some(r) = &self.right {
            let i = kids.len() - 1;
            let m = r.as_widget().mouse_interaction(
                &tree.children[i],
                kids[i],
                cursor,
                viewport,
                renderer,
            );
            if m != mouse::Interaction::None {
                return m;
            }
        }
        self.input.as_widget().mouse_interaction(
            &tree.children[0],
            kids[0],
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
        let kids: Vec<_> = layout.children().collect();
        let token = tree.state.downcast_ref::<BoxState>().token;
        operation.custom(None, layout.bounds(), &mut crate::focus::Anchor(token));
        operation.custom(
            None,
            layout.bounds(),
            &mut crate::focus::Meta {
                token,
                tab_stop: !self.disabled,
            },
        );
        self.input
            .as_widget_mut()
            .operate(&mut tree.children[0], kids[0], renderer, operation);
        if let Some(r) = &mut self.right {
            let i = kids.len() - 1;
            r.as_widget_mut()
                .operate(&mut tree.children[i], kids[i], renderer, operation);
        }
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
        let raw = layout.bounds();
        let b = crate::scale::snap_rect(raw);
        let shift = Vector::new(b.x - raw.x, b.y - raw.y);
        let st = tree.state.downcast_ref::<BoxState>();
        let focus = st.progress(Instant::now());
        let boxed = control_style(
            &self.cx,
            ControlState {
                error: self.error,
                disabled: self.disabled,
                focus,
            },
        );
        let kids: Vec<_> = layout.children().collect();
        let adornments = |renderer: &mut iced::Renderer| {
            for (i, el) in self.slots() {
                el.as_widget().draw(
                    &tree.children[i],
                    renderer,
                    theme,
                    style,
                    kids[i],
                    cursor - shift,
                    viewport,
                );
            }
        };
        let alpha = if self.disabled { DISABLED_OPACITY } else { 1.0 };
        renderer.with_translation(shift, |renderer| {
            // A disabled input is an opacity layer, painted after (over)
            // the absolutely positioned adornments; an enabled one is in
            // flow, under them.
            if self.disabled {
                adornments(renderer);
            }
            with_opacity(alpha, || paint(renderer, raw, &boxed));
            self.input.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                kids[0],
                cursor - shift,
                viewport,
            );
            if !self.disabled {
                adornments(renderer);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bato_theme::{Rgb, ThemeId, palette};

    fn cx() -> Ctx {
        Ctx::from_palette(palette(ThemeId::Dark, Rgb::hex("#8b5cf6").unwrap())).on_card()
    }

    #[test]
    fn sizes_match_size_classes() {
        assert_eq!(FieldSize::Sm.metrics().height(), 30.0);
        assert_eq!(FieldSize::Md.metrics().height(), 38.0);
        assert_eq!(FieldSize::Lg.metrics().height(), 42.0);
        // Text at border + px, or at pl-9 / pr-9 with adornments.
        assert_eq!(text_insets(FieldSize::Md, false, false), (13.0, 13.0));
        assert_eq!(text_insets(FieldSize::Sm, true, false), (37.0, 11.0));
        assert_eq!(text_insets(FieldSize::Lg, false, true), (15.0, 37.0));
    }

    #[test]
    fn control_box_follows_control_classes() {
        let cx = cx();
        let p = cx.p().clone();
        let rest = control_style(&cx, ControlState::default());
        assert_eq!(rest.bg, Some(c(cx.recessed())));
        assert_eq!(rest.border, Some(c(p.control)));
        assert_eq!(rest.radius, 8.0);
        assert!(rest.rings.is_empty());
        let f = control_style(
            &cx,
            ControlState {
                focus: 1.0,
                ..ControlState::default()
            },
        );
        assert_eq!(f.border, Some(c(p.accent_bright)));
        assert_eq!(f.rings, vec![(4.0, c(cx.tint(0.15)))]);
        let e = control_style(
            &cx,
            ControlState {
                error: true,
                focus: 1.0,
                ..ControlState::default()
            },
        );
        assert_eq!(e.border, Some(c(p.red)));
        assert_eq!(e.rings, vec![(4.0, c(cx.blend(p.red, 0.15)))]);
        let e_rest = control_style(
            &cx,
            ControlState {
                error: true,
                ..ControlState::default()
            },
        );
        assert_eq!(e_rest.border, Some(c(p.red)), "error border at rest too");
        // Half way through the transition: half the halo.
        let h = control_style(
            &cx,
            ControlState {
                focus: 0.5,
                ..ControlState::default()
            },
        );
        assert_eq!(h.rings[0].0, 2.0);
    }

    #[test]
    fn focus_transition_is_150ms_and_reversible() {
        let t = Instant::now();
        let mut s = BoxState::default();
        assert_eq!(s.progress(t), 0.0);
        s.from = 0.0;
        s.focused = true;
        s.changed = Some(t);
        assert_eq!(s.progress(t), 0.0);
        assert_eq!(s.progress(t + TRANSITION), 1.0);
        assert!(s.running(t + Duration::from_millis(100)));
        assert!(!s.running(t + TRANSITION));
        let mid = s.progress(t + Duration::from_millis(75));
        assert!(mid > 0.5 && mid < 1.0, "ease-out heavy curve: {mid}");
        // Blur mid-way: starts from where it was.
        s.from = mid;
        s.focused = false;
        s.changed = Some(t + Duration::from_millis(75));
        assert_eq!(s.progress(t + Duration::from_millis(75)), mid);
    }

    #[test]
    fn disabled_text_is_faded_by_hand() {
        let cx = cx();
        let on = text_colors(&cx, false);
        let off = text_colors(&cx, true);
        assert_eq!(on.value, c(cx.p().text));
        assert_eq!(off.value, c(cx.blend(cx.p().text, 0.6)));
        assert_ne!(on.placeholder, off.placeholder);
    }
}
