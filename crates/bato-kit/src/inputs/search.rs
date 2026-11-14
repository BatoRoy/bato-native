//! `SearchInput` (`inputs/SearchInput.tsx`): a search field with an
//! optional suggestions dropdown and optional inline autocomplete.
//!
//! The box is a control (`px-3 py-2`) with a 15 px `muted` Search icon, the
//! input (`pl-2 pr-2`) and, when there is a value, a 14 px clear × that
//! empties it and puts focus back in the input. Suggestions come from a
//! static list (case-insensitive substring of the query; all of them for an
//! empty query) or from `on_search`, at most `max_results` (8). The dropdown
//! is a `matchWidth` popover under the box, open while the input has been
//! focused or typed in and there are results: ↑ ↓ (clamped) and hover move
//! the active row, Enter or a press picks it (without taking focus from the
//! input), Esc closes it. With `auto_complete`, the rest of the top result
//! shows as `muted` ghost text after the query when it starts with it; Tab,
//! or → with the caret at the end, accepts it. The input is controlled like
//! React's: a value the app sets (an accepted completion, a picked row)
//! puts the caret at the end.
//!
//! ```ignore
//! search_input(&on, &state.q, Msg::Q).suggestions(FRAMEWORKS).auto_complete(state.auto)
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, key::Named};
use iced::widget::{column, container, row, text_input};
use iced::{Alignment, Color, Element, Event, Length, Padding, Rectangle, Size, Vector, mouse};

use super::combobox::{bare_input, control_frame, option_row};
use super::text_field::controlled;
use crate::ctx::Ctx;
use crate::field::field;
use crate::focus::{Target, request};
use crate::icon::{Ico, icon};
use crate::label::{Span, label, rich};
use crate::overlay::popover::{keys, local_many, min_fill, popover, popover_panel};
use crate::scroller::scroll_area;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;
type P = <iced::Renderer as TextRenderer>::Paragraph;

/// A dynamic suggestion source.
type Source<'a> = Box<dyn Fn(&str) -> Vec<String> + 'a>;

pub struct SearchInput<'a, M> {
    cx: Ctx,
    value: String,
    on_change: Box<dyn Fn(String) -> M + 'a>,
    suggestions: Vec<String>,
    on_search: Option<Source<'a>>,
    show_suggestions: bool,
    auto_complete: bool,
    on_select: Option<Box<dyn Fn(String) -> M + 'a>>,
    max_results: usize,
    placeholder: String,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    id: Option<widget::Id>,
}

/// `<SearchInput value onChange />` on `cx` (the field's backdrop).
pub fn search_input<'a, M>(
    cx: &Ctx,
    value: &str,
    on_change: impl Fn(String) -> M + 'a,
) -> SearchInput<'a, M> {
    SearchInput {
        cx: cx.clone(),
        value: value.to_string(),
        on_change: Box::new(on_change),
        suggestions: Vec::new(),
        on_search: None,
        show_suggestions: true,
        auto_complete: false,
        on_select: None,
        max_results: 8,
        placeholder: "Search…".into(),
        label: None,
        hint: None,
        error: None,
        id: None,
    }
}

impl<'a, M> SearchInput<'a, M> {
    /// The static source, filtered by the query.
    pub fn suggestions<S: ToString>(mut self, s: &[S]) -> Self {
        self.suggestions = s.iter().map(ToString::to_string).collect();
        self
    }
    /// The dynamic source (overrides `suggestions`). Synchronous: React's
    /// may be async and is debounced (150 ms); run slow lookups in the app
    /// and pass their results as `suggestions` instead.
    pub fn on_search(mut self, f: impl Fn(&str) -> Vec<String> + 'a) -> Self {
        self.on_search = Some(Box::new(f));
        self
    }
    /// The suggestions dropdown (default on).
    pub fn show_suggestions(mut self, on: bool) -> Self {
        self.show_suggestions = on;
        self
    }
    /// Inline ghost completion of the top match (default off).
    pub fn auto_complete(mut self, on: bool) -> Self {
        self.auto_complete = on;
        self
    }
    /// Published after `on_change` when a suggestion is chosen.
    pub fn on_select(mut self, f: impl Fn(String) -> M + 'a) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }
    pub fn max_results(mut self, n: usize) -> Self {
        self.max_results = n;
        self
    }
    pub fn placeholder(mut self, s: impl Into<String>) -> Self {
        self.placeholder = s.into();
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
    /// The input's id (`ref` + `focus()`): `bato_kit::focus::focus(id)`.
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// The static source's results for `value`.
pub fn filter_suggestions(suggestions: &[String], value: &str, max: usize) -> Vec<String> {
    let q = value.to_lowercase();
    suggestions
        .iter()
        .filter(|s| q.is_empty() || s.to_lowercase().contains(&q))
        .take(max)
        .cloned()
        .collect()
}

/// The ghost text: the rest of `top` when it starts with `value`
/// (case-insensitive) and is longer.
pub fn completion(value: &str, top: Option<&str>) -> String {
    let Some(top) = top else {
        return String::new();
    };
    if value.is_empty()
        || !top.to_lowercase().starts_with(&value.to_lowercase())
        || top.chars().count() <= value.chars().count()
    {
        return String::new();
    }
    top.chars().skip(value.chars().count()).collect()
}

#[derive(Debug)]
pub struct SearchState {
    pub open: bool,
    pub active: usize,
    /// The input's id (the clear button focuses it).
    id: widget::Id,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            open: false,
            active: 0,
            id: widget::Id::unique(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SearchEvent {
    Input(String),
    Focus,
    Close,
    Active(usize),
    Choose(String),
    /// ↓ ↑ in the open dropdown (the result count rides along).
    Move(i32, usize),
    Clear,
}

/// → at the end of the input accepts the completion; any other key, or →
/// mid-text, is the input's.
struct EndKey<'a, E> {
    input: El<'a, E>,
    value: String,
    accept: Option<E>,
}

impl<'a, E: Clone + 'a> Widget<E, iced::Theme, iced::Renderer> for EndKey<'a, E> {
    fn size(&self) -> Size<Length> {
        self.input.as_widget().size()
    }
    fn tag(&self) -> widget::tree::Tag {
        self.input.as_widget().tag()
    }
    fn state(&self) -> widget::tree::State {
        self.input.as_widget().state()
    }
    fn children(&self) -> Vec<Tree> {
        self.input.as_widget().children()
    }
    fn diff(&self, tree: &mut Tree) {
        self.input.as_widget().diff(tree);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.input.as_widget_mut().layout(tree, renderer, limits)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, E>,
        viewport: &Rectangle,
    ) {
        if let (
            Some(m),
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::ArrowRight),
                ..
            }),
        ) = (&self.accept, event)
            && tree.tag == widget::tree::Tag::of::<text_input::State<P>>()
        {
            let st = tree.state.downcast_ref::<text_input::State<P>>();
            let value = text_input::Value::new(&self.value);
            let at_end = matches!(
                st.cursor().state(&value),
                text_input::cursor::State::Index(i) if i == value.len()
            );
            if st.is_focused() && at_end {
                shell.publish(m.clone());
                shell.capture_event();
                return;
            }
        }
        self.input.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.input
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.input
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
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
        self.input
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, E, iced::Theme, iced::Renderer>> {
        self.input
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a, M: Clone + 'a> From<SearchInput<'a, M>> for El<'a, M> {
    fn from(si: SearchInput<'a, M>) -> Self {
        let SearchInput {
            cx,
            value,
            on_change,
            suggestions,
            on_search,
            show_suggestions,
            auto_complete,
            on_select,
            max_results,
            placeholder,
            label: lab,
            hint,
            error,
            id: user_id,
        } = si;
        let clear_id = user_id.clone();
        let results = match &on_search {
            Some(f) => {
                let mut r = f(&value);
                r.truncate(max_results);
                r
            }
            None => filter_suggestions(&suggestions, &value, max_results),
        };
        let ghost = if auto_complete {
            completion(&value, results.first().map(String::as_str))
        } else {
            String::new()
        };
        let fcx = cx.clone();
        let has_error = error.is_some();
        let control = local_many(
            move |s: &SearchState| {
                let p = cx.p();
                let dropdown_open = s.open && show_suggestions && !results.is_empty();
                let n = results.len();
                let accept =
                    (!ghost.is_empty()).then(|| SearchEvent::Choose(format!("{value}{ghost}")));
                let enter = results.get(s.active).cloned().map(SearchEvent::Choose);
                // Controlled like React's input: an accepted completion or a
                // picked row puts the caret at the end of the new value.
                let typed: Rc<RefCell<Option<String>>> = Rc::default();
                let on_input = {
                    let typed = typed.clone();
                    move |v: String| {
                        *typed.borrow_mut() = Some(v.clone());
                        SearchEvent::Input(v)
                    }
                };
                let input = controlled(
                    Element::new(EndKey {
                        input: bare_input(&cx, &placeholder, &value, on_input)
                            .id(user_id.clone().unwrap_or_else(|| s.id.clone()))
                            .padding(Padding::from([0.0, 8.0]))
                            .into(),
                        value: value.clone(),
                        accept: accept.clone(),
                    }),
                    value.clone(),
                    typed,
                );
                // The ghost layer is always there (empty without a completion)
                // so the input keeps its place in the tree, and its focus.
                // `pl-9` from the box's padding edge: 1 px right of the
                // input's own text (React's geometry).
                let ghost_layer = container(rich(
                    vec![
                        Span::new(value.clone()).color(Color::TRANSPARENT),
                        Span::new(ghost.clone()).color(c(p.muted)),
                    ],
                    14.0,
                    20.0,
                ))
                .padding(Padding::default().left(9));
                let field_layers: Vec<El<'_, SearchEvent>> = vec![input, ghost_layer.into()];
                let mut items: Vec<El<'_, SearchEvent>> = vec![
                    icon(Ico::Search, 15.0).color(c(p.muted)).into(),
                    iced::widget::Stack::with_children(field_layers)
                        .width(Length::Fill)
                        .into(),
                ];
                if !value.is_empty() {
                    let pp = p.clone();
                    items.push(
                        surface(icon(Ico::X, 14.0))
                            .focusable(false)
                            .style(move |st| {
                                BoxStyle::default().fg(c(if st == Status::Hover {
                                    pp.text
                                } else {
                                    pp.muted
                                }))
                            })
                            .on_press(SearchEvent::Clear)
                            .into(),
                    );
                }
                let boxed = control_frame(
                    &cx,
                    container(row(items).align_y(Alignment::Center))
                        .padding(Padding::from([9.0, 13.0]))
                        .width(Length::Fill),
                )
                .text(true)
                .error(has_error)
                .on_focus(SearchEvent::Focus);
                let boxed = keys(boxed, move |k, _| match k {
                    Named::ArrowDown if dropdown_open => Some(SearchEvent::Move(1, n)),
                    Named::ArrowUp if dropdown_open => Some(SearchEvent::Move(-1, n)),
                    Named::Enter if dropdown_open => enter.clone(),
                    Named::Escape => Some(SearchEvent::Close),
                    Named::Tab => accept.clone(),
                    _ => None,
                });
                let panel = dropdown_open.then(|| {
                    let on = cx.on_card();
                    let rows: Vec<El<'_, SearchEvent>> = results
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            option_row(
                                &on,
                                vec![
                                    icon(Ico::Search, 13.0).color(c(on.p().muted)).into(),
                                    label(r.clone(), 14.0, 20.0)
                                        .ellipsis()
                                        .width(Length::Fill)
                                        .into(),
                                ],
                                i == s.active,
                                false,
                                false,
                                SearchEvent::Choose(r.clone()),
                                Some(SearchEvent::Active(i)),
                            )
                        })
                        .collect();
                    min_fill(
                        popover_panel(
                            &cx,
                            scroll_area(
                                &on,
                                column(rows)
                                    .width(Length::Fill)
                                    .padding(Padding::from([4.0, 0.0])),
                            )
                            .max_height(240.0),
                        )
                        .padding(1)
                        .width(Length::Fill),
                    )
                    .into()
                });
                popover(boxed, panel)
                    .match_width(true)
                    .on_dismiss(|_| SearchEvent::Close)
                    .into()
            },
            move |s: &mut SearchState, e| match e {
                SearchEvent::Input(v) => {
                    s.open = true;
                    s.active = 0;
                    vec![on_change(v)]
                }
                SearchEvent::Focus => {
                    s.open = true;
                    vec![]
                }
                SearchEvent::Close => {
                    s.open = false;
                    vec![]
                }
                SearchEvent::Active(i) => {
                    s.active = i;
                    vec![]
                }
                SearchEvent::Move(d, n) => {
                    s.active = (s.active as i32 + d).clamp(0, n as i32 - 1).max(0) as usize;
                    vec![]
                }
                SearchEvent::Choose(v) => {
                    s.open = false;
                    s.active = 0;
                    let mut out = vec![on_change(v.clone())];
                    if let Some(f) = &on_select {
                        out.push(f(v));
                    }
                    out
                }
                SearchEvent::Clear => {
                    s.active = 0;
                    request(Target::Id(clear_id.clone().unwrap_or_else(|| s.id.clone())));
                    vec![on_change(String::new())]
                }
            },
        );
        let mut f = field(&fcx, control).error(error.as_deref());
        if let Some(l) = lab {
            f = f.label(l);
        }
        if let Some(h) = hint {
            f = f.hint(h);
        }
        f.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fw() -> Vec<String> {
        [
            "React", "Redux", "Recoil", "Remix", "Svelte", "Solid", "Vue", "Vite", "Vitest",
            "Angular", "Astro", "Next.js",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn results_are_filtered_and_capped() {
        let s = fw();
        assert_eq!(filter_suggestions(&s, "", 8).len(), 8);
        assert_eq!(filter_suggestions(&s, "", 8)[7], "Vite");
        assert_eq!(
            filter_suggestions(&s, "re", 8),
            vec!["React", "Redux", "Recoil", "Remix"]
        );
        assert_eq!(filter_suggestions(&s, "VIT", 8), vec!["Vite", "Vitest"]);
        assert!(filter_suggestions(&s, "zz", 8).is_empty());
    }

    #[test]
    fn ghost_completion() {
        assert_eq!(completion("re", Some("React")), "act");
        assert_eq!(completion("RE", Some("React")), "act", "case-insensitive");
        assert_eq!(completion("", Some("React")), "");
        assert_eq!(completion("React", Some("React")), "", "nothing left");
        assert_eq!(completion("ea", Some("React")), "", "not a prefix");
        assert_eq!(completion("re", None), "");
    }
}
