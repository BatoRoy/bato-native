//! `TagsInput` (`inputs/TagsInput.tsx`): chips and a text input wrapping in
//! the control box (`flex flex-wrap items-center gap-1.5 px-2 py-1.5`,
//! `focus-within:border-app-accent`, no halo). Enter or `,` adds the draft,
//! Backspace on an empty draft removes the last tag, a paste holding `,` or a
//! newline adds every part, leaving the field commits the draft, a chip's ×
//! removes it. `unique` (default) skips duplicates; `max` caps the count.
//!
//! The app owns the tags and a [`TagsEdit`] (React's `draft`):
//!
//! ```ignore
//! tags_input(&cx, &self.tags_edit, &self.tags, Msg::Tags).label("Tags")
//! // update: Msg::Tags(ev) => self.tags_edit.update(ev, &mut self.tags),
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{Key, key::Named};
use iced::widget::row;
use iced::{Alignment, Element, Event, Length, Padding, Point, Rectangle, Size, Vector, mouse};

use super::frame::{Look, control_frame, plain_input};
use crate::El;
use crate::ctx::Ctx;
use crate::field::field;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// React's `add`: split on `,` / newline, trim, drop empties, skip
/// duplicates when `unique`, stop at `max`.
pub fn add_tags(tags: &[String], raw: &str, unique: bool, max: Option<usize>) -> Vec<String> {
    let mut next = tags.to_vec();
    for part in raw
        .split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if unique && next.iter().any(|t| t == part) {
            continue;
        }
        if max.is_some_and(|m| next.len() >= m) {
            break;
        }
        next.push(part.to_string());
    }
    next
}

/// What the field reports.
#[derive(Debug, Clone, PartialEq)]
pub enum TagsEvent {
    /// The draft changed (typed or pasted).
    Draft(String),
    /// Enter, or focus left the field.
    Commit,
    Remove(usize),
    /// Backspace on an empty draft.
    RemoveLast,
}

/// The field's own state: the draft and the rules.
#[derive(Debug, Clone)]
pub struct TagsEdit {
    pub draft: String,
    pub unique: bool,
    pub max: Option<usize>,
}

impl Default for TagsEdit {
    fn default() -> Self {
        Self {
            draft: String::new(),
            unique: true,
            max: None,
        }
    }
}

impl TagsEdit {
    /// Apply an event to the draft and `tags`.
    pub fn update(&mut self, ev: TagsEvent, tags: &mut Vec<String>) {
        match ev {
            TagsEvent::Draft(s) => {
                if s.contains([',', '\n']) {
                    // `,` typed (React adds the draft) or a multi-part paste.
                    *tags = add_tags(tags, &s, self.unique, self.max);
                    self.draft.clear();
                } else {
                    self.draft = s;
                }
            }
            TagsEvent::Commit => {
                if !self.draft.trim().is_empty() {
                    *tags = add_tags(tags, &self.draft, self.unique, self.max);
                    self.draft.clear();
                }
            }
            TagsEvent::Remove(i) => {
                if i < tags.len() {
                    tags.remove(i);
                }
            }
            TagsEvent::RemoveLast => {
                if self.draft.is_empty() {
                    tags.pop();
                }
            }
        }
    }
}

pub struct TagsInput<'a, M> {
    cx: Ctx,
    tags: Vec<String>,
    draft: String,
    on: Box<dyn Fn(TagsEvent) -> M + 'a>,
    placeholder: String,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
}

pub fn tags_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    edit: &TagsEdit,
    tags: &[String],
    on: impl Fn(TagsEvent) -> M + 'a,
) -> TagsInput<'a, M> {
    TagsInput {
        cx: cx.clone(),
        tags: tags.to_vec(),
        draft: edit.draft.clone(),
        on: Box::new(on),
        placeholder: String::new(),
        label: None,
        hint: None,
        error: None,
        required: false,
    }
}

impl<'a, M: Clone + 'a> TagsInput<'a, M> {
    /// Shown while there are no tags.
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = Some(l.into());
        self
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    pub fn error(mut self, e: Option<&str>) -> Self {
        self.error = e.map(str::to_string);
        self
    }
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
}

impl<'a, M: Clone + 'a> From<TagsInput<'a, M>> for El<'a, M> {
    fn from(t: TagsInput<'a, M>) -> Self {
        let cx = t.cx.clone();
        let inside = cx.on(cx.recessed());
        let p = inside.p();
        let on = std::rc::Rc::new(t.on);
        let mut items: Vec<El<'a, M>> = Vec::new();
        for (i, tag) in t.tags.iter().enumerate() {
            let (rest, hover) = (c(p.accent_bright), c(p.text));
            let x = surface(icon(Ico::X, 12.0))
                .style(move |s| {
                    BoxStyle::default().fg(if s == Status::Hover { hover } else { rest })
                })
                .transition_ms(0)
                .focus_ring(Ring::plain(&inside))
                .on_press(on(TagsEvent::Remove(i)));
            items.push(
                surface(
                    row![label(tag.clone(), 12.0, 16.0).color(c(p.accent_bright)), x]
                        .spacing(4)
                        .align_y(Alignment::Center),
                )
                .padding(Padding {
                    top: 2.0,
                    bottom: 2.0,
                    left: 8.0,
                    right: 4.0,
                })
                .fixed(BoxStyle::default().bg(c(inside.tint(0.15))).radius(6.0))
                .into(),
            );
        }
        let placeholder = if t.tags.is_empty() {
            t.placeholder.as_str()
        } else {
            ""
        };
        let (a, b) = (on.clone(), on.clone());
        let input = plain_input(&cx, placeholder, &t.draft, 14.0, 20.0, false)
            .padding([2.0, 0.0])
            .width(Length::Fill)
            .on_input(move |s| a(TagsEvent::Draft(s)))
            .on_paste(move |s| b(TagsEvent::Draft(s)))
            .on_submit(on(TagsEvent::Commit));
        items.push(input.into());
        let empty = t.draft.is_empty() && !t.tags.is_empty();
        let (k, f) = (on.clone(), on.clone());
        let frame = control_frame(
            &cx,
            Element::new(FlexWrap {
                items,
                gap: 6.0,
                min_last: 96.0,
            }),
        )
        .look(Look::Within)
        .error(t.error.is_some())
        .padding(Padding {
            top: 7.0,
            bottom: 7.0,
            left: 9.0,
            right: 9.0,
        })
        .on_key(move |key, _mods, at| {
            (empty && at.is_last() && *key == Key::Named(Named::Backspace))
                .then(|| k(TagsEvent::RemoveLast))
        })
        .on_focus(move |focused| (!focused).then(|| f(TagsEvent::Commit)));
        let mut fld = field(&cx, frame)
            .required(t.required)
            .error(t.error.as_deref());
        if let Some(l) = t.label {
            fld = fld.label(l);
        }
        if let Some(h) = t.hint {
            fld = fld.hint(h);
        }
        fld.into()
    }
}

// ── flex-wrap row whose last item fills its line ────────────────────────

/// `flex flex-wrap items-center gap-G`, the last child `flex-1 min-w-…`.
struct FlexWrap<'a, M> {
    items: Vec<El<'a, M>>,
    gap: f32,
    min_last: f32,
}

/// Place items of `sizes` in lines of `width` with `gap`; the last item is
/// at least `min_last` wide and stretches to the end of its line. Returns
/// each item's rect and the total height.
fn flow(sizes: &[Size], width: f32, gap: f32, min_last: f32) -> (Vec<Rectangle>, f32) {
    let n = sizes.len();
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0.0;
    for (i, s) in sizes.iter().enumerate() {
        let w = if i + 1 == n {
            min_last.max(s.width.min(min_last))
        } else {
            s.width
        };
        let line = lines.last_mut().unwrap();
        if !line.is_empty() && x + gap + w > width {
            lines.push(vec![i]);
            x = w;
        } else {
            x += if line.is_empty() { w } else { gap + w };
            lines.last_mut().unwrap().push(i);
        }
    }
    let mut rects = vec![Rectangle::default(); n];
    let mut y = 0.0;
    for (li, line) in lines.iter().enumerate() {
        let h = line.iter().map(|&i| sizes[i].height).fold(0.0, f32::max);
        let mut x = 0.0;
        for &i in line {
            let w = if i + 1 == n {
                (width - x).max(min_last.min(width))
            } else {
                sizes[i].width
            };
            rects[i] = Rectangle {
                x,
                y: y + ((h - sizes[i].height) / 2.0).round(),
                width: w,
                height: sizes[i].height,
            };
            x += w + gap;
        }
        y += h;
        if li + 1 < lines.len() {
            y += gap;
        }
    }
    (rects, y)
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for FlexWrap<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn children(&self) -> Vec<Tree> {
        self.items.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.items);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let loose = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        let n = self.items.len();
        // First pass: natural sizes (the last item at its minimum).
        let mut sizes = Vec::with_capacity(n);
        for (i, (el, t)) in self
            .items
            .iter_mut()
            .zip(tree.children.iter_mut())
            .enumerate()
        {
            let l = if i + 1 == n {
                layout::Limits::new(
                    Size::ZERO,
                    Size::new(self.min_last.min(width), f32::INFINITY),
                )
            } else {
                loose
            };
            sizes.push(el.as_widget_mut().layout(t, renderer, &l).size());
        }
        let (rects, h) = flow(&sizes, width, self.gap, self.min_last);
        let nodes = self
            .items
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(rects.iter())
            .map(|((el, t), r)| {
                let exact =
                    layout::Limits::new(Size::new(r.width, 0.0), Size::new(r.width, f32::INFINITY));
                el.as_widget_mut()
                    .layout(t, renderer, &exact)
                    .move_to(Point::new(r.x, r.y))
            })
            .collect();
        layout::Node::with_children(Size::new(width, h), nodes)
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
        for ((el, t), l) in self
            .items
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
        {
            el.as_widget_mut()
                .update(t, event, l, cursor, renderer, clipboard, shell, viewport);
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
        let mut out = mouse::Interaction::None;
        for ((el, t), l) in self
            .items
            .iter()
            .zip(tree.children.iter())
            .zip(layout.children())
        {
            let m = el
                .as_widget()
                .mouse_interaction(t, l, cursor, viewport, renderer);
            if m != mouse::Interaction::None {
                out = m;
            }
        }
        // The empty space of the box is the input's (a click there focuses
        // nothing in React, but the caret cursor reads as a field).
        if out == mouse::Interaction::None && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            out
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        for ((el, t), l) in self
            .items
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
        {
            el.as_widget_mut().operate(t, l, renderer, operation);
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
        for ((el, t), l) in self
            .items
            .iter()
            .zip(tree.children.iter())
            .zip(layout.children())
        {
            el.as_widget()
                .draw(t, renderer, theme, style, l, cursor, viewport);
        }
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        iced::advanced::overlay::from_children(
            &mut self.items,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn add_splits_trims_and_dedupes() {
        let t = s(&["react", "electron"]);
        assert_eq!(
            add_tags(&t, " vue , react,\nsvelte", true, None),
            s(&["react", "electron", "vue", "svelte"])
        );
        assert_eq!(
            add_tags(&t, "react", false, None),
            s(&["react", "electron", "react"])
        );
        assert_eq!(
            add_tags(&t, "a,b,c", true, Some(3)),
            s(&["react", "electron", "a"])
        );
    }

    #[test]
    fn keyboard_flow() {
        let mut tags = s(&["react"]);
        let mut e = TagsEdit::default();
        e.update(TagsEvent::Draft("vue".into()), &mut tags);
        assert_eq!(e.draft, "vue");
        e.update(TagsEvent::Commit, &mut tags);
        assert_eq!(tags, s(&["react", "vue"]));
        assert!(e.draft.is_empty());
        e.update(TagsEvent::Draft("solid,".into()), &mut tags);
        assert_eq!(tags, s(&["react", "vue", "solid"]), "comma adds");
        e.update(TagsEvent::RemoveLast, &mut tags);
        assert_eq!(tags, s(&["react", "vue"]));
        e.update(TagsEvent::Draft("x".into()), &mut tags);
        e.update(TagsEvent::RemoveLast, &mut tags);
        assert_eq!(tags.len(), 2, "only with an empty draft");
        e.update(TagsEvent::Remove(0), &mut tags);
        assert_eq!(tags, s(&["vue"]));
    }

    #[test]
    fn flow_wraps_and_stretches_the_last() {
        let sz = |w: f32, h: f32| Size::new(w, h);
        let (r, h) = flow(
            &[sz(50.0, 20.0), sz(60.0, 20.0), sz(10.0, 24.0)],
            300.0,
            6.0,
            96.0,
        );
        assert_eq!(h, 24.0);
        assert_eq!(r[1].x, 56.0);
        assert_eq!(r[2].x, 122.0);
        assert_eq!(r[2].width, 178.0, "fills the line");
        assert_eq!(r[0].y, 2.0, "items-center");
        let (r, h) = flow(
            &[sz(150.0, 20.0), sz(100.0, 20.0), sz(10.0, 24.0)],
            300.0,
            6.0,
            96.0,
        );
        assert_eq!(r[2].y, 26.0, "wraps when under min width");
        assert_eq!(r[2].x, 0.0);
        assert_eq!(h, 50.0);
    }
}
