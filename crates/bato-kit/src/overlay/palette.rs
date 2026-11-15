//! `CommandPalette` (`overlay/CommandPalette.tsx`): the Ctrl K launcher.
//!
//! A [`Kind::Top`](crate::modal::Kind::Top) level of the overlay stack (top-aligned at 12 vh,
//! `max-w-lg`, scrim `#000@.5` + blur, z-50 over the toasts): a search field that takes focus
//! when the palette opens, then the commands, grouped under their `group` headers in list order.
//!
//! | React | Here |
//! |---|---|
//! | filter: `(label + ' ' + keywords + ' ' + group).toLowerCase().includes(q)` | [`filter`], the same substring test |
//! | ↑ ↓ move the active row, Enter runs it, Esc closes (on the input) | [`Msg::Up`] / [`Msg::Down`] / [`Msg::Enter`] while the search field has focus; Esc closes the level |
//! | `onMouseEnter` sets the active row | [`Msg::Hover`] when the pointer enters a row |
//! | `scrollIntoView({ block: 'nearest' })` on the active row | [`crate::scroller::jump_to`] from [`Palette::update`] |
//! | `run(c)`: close, then `c.onRun()` | [`Outcome::Run`] after closing |
//!
//! The app owns a [`Palette`] (open flag, query, active row) and a list of [`Command`]s whose
//! `run` is the app's own message. The frame has one built in ([`crate::frame`]: `mod+k`, the
//! sidebar search box, `frame::Msg::OpenPalette`); use this module directly for a page's own
//! palette (the Showcase's Triggers card).
//!
//! ```ignore
//! // state
//! palette: Palette, commands: Vec<Command<Msg>>,
//! // update
//! Msg::Palette(m) => match self.palette.update(m, &self.commands) {
//!     Some(Outcome::Run(run)) => return self.update(run),
//!     _ => {}
//! },
//! // view: one level of `Frame::view_with`
//! if self.palette.is_open() { levels.push(self.palette.level(cx, &self.commands, "Type a command…", Msg::Palette)) }
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, key::Named};
use iced::widget::{column, container, row, text_input};
use iced::{
    Alignment, Background, Border, Color, Event, Length, Padding, Rectangle, Size, Vector, mouse,
};

use crate::El;
use crate::ctx::Ctx;
use crate::icon::{Ico, Icon, icon};
use crate::label::label;
use crate::layout::hline;
use crate::modal::{Kind, Modal, level};
use crate::scroller::{Align, jump_to, scroll_area};
use crate::style::{c, rgba, shadows};
use crate::surface::{BoxStyle, surface};

/// One command (React `Command`). `run` is the message the app gets when it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Command<M> {
    pub id: String,
    pub label: String,
    pub icon: Option<Icon>,
    /// Shown right-aligned (a shortcut, `⌘N`).
    pub hint: Option<String>,
    /// Consecutive commands of one group share a header.
    pub group: Option<String>,
    /// Extra words the filter matches (`appearance`).
    pub keywords: Option<String>,
    pub run: M,
}

impl<M> Command<M> {
    pub fn new(id: impl Into<String>, label: impl Into<String>, run: M) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            hint: None,
            group: None,
            keywords: None,
            run,
        }
    }
    pub fn icon(mut self, ico: impl Into<Icon>) -> Self {
        self.icon = Some(ico.into());
        self
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
    pub fn keywords(mut self, words: impl Into<String>) -> Self {
        self.keywords = Some(words.into());
        self
    }
    /// The same command with another message type.
    pub fn map<N>(self, f: impl FnOnce(M) -> N) -> Command<N> {
        Command {
            id: self.id,
            label: self.label,
            icon: self.icon,
            hint: self.hint,
            group: self.group,
            keywords: self.keywords,
            run: f(self.run),
        }
    }

    /// What the filter searches: label, keywords and group, lowercased.
    fn haystack(&self) -> String {
        format!(
            "{} {} {}",
            self.label,
            self.keywords.as_deref().unwrap_or(""),
            self.group.as_deref().unwrap_or("")
        )
        .to_lowercase()
    }
}

/// The indices of the commands matching `query`, in list order (React's
/// `filtered`): every command for a blank query, else those whose label,
/// keywords or group contain the trimmed, lowercased query.
pub fn filter<M>(commands: &[Command<M>], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    commands
        .iter()
        .enumerate()
        .filter(|(_, c)| q.is_empty() || c.haystack().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// Where a group header goes: for each filtered row, the group to show
/// above it (React: `c.group && c.group !== lastGroup`).
pub fn headers<'c, M>(commands: &'c [Command<M>], rows: &[usize]) -> Vec<Option<&'c str>> {
    let mut last: Option<&str> = None;
    rows.iter()
        .map(|&i| {
            let g = commands[i].group.as_deref();
            let show = g.is_some() && g != last;
            last = g;
            if show { g } else { None }
        })
        .collect()
}

/// The palette's messages; route them back into [`Palette::update`].
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Query(String),
    Up,
    Down,
    /// The pointer entered the row at this filtered position.
    Hover(usize),
    /// Run the active row.
    Enter,
    /// Run the row at this filtered position (a click).
    Run(usize),
    Close,
}

/// What [`Palette::update`] asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome<M> {
    /// The palette closed and this command runs.
    Run(M),
    /// Closed without running anything.
    Closed,
}

/// The palette's state (the app owns one per palette).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Palette {
    open: bool,
    query: String,
    active: usize,
}

/// The id of the container around the filtered row `i` (scroll-into-view).
pub fn row_id(i: usize) -> widget::Id {
    widget::Id::from(format!("bato-palette-row-{i}"))
}

impl Palette {
    /// Open with an empty query and the first row active (React resets both
    /// when `open` turns true).
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.active = 0;
    }
    pub fn close(&mut self) {
        self.open = false;
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    /// The active row's position among the filtered rows.
    pub fn active(&self) -> usize {
        self.active
    }

    pub fn update<M: Clone>(&mut self, msg: Msg, commands: &[Command<M>]) -> Option<Outcome<M>> {
        let rows = filter(commands, &self.query);
        match msg {
            Msg::Query(q) => {
                self.query = q;
                self.active = 0;
                jump_to(row_id(0), Align::Nearest);
            }
            Msg::Down => {
                self.active = (self.active + 1).min(rows.len().saturating_sub(1));
                jump_to(row_id(self.active), Align::Nearest);
            }
            Msg::Up => {
                self.active = self.active.saturating_sub(1);
                jump_to(row_id(self.active), Align::Nearest);
            }
            Msg::Hover(i) => self.active = i,
            Msg::Enter => return self.run(rows.get(self.active).map(|&i| &commands[i])),
            Msg::Run(i) => return self.run(rows.get(i).map(|&i| &commands[i])),
            Msg::Close => {
                if self.open {
                    self.open = false;
                    return Some(Outcome::Closed);
                }
            }
        }
        None
    }

    fn run<M: Clone>(&mut self, cmd: Option<&Command<M>>) -> Option<Outcome<M>> {
        let cmd = cmd?;
        self.open = false;
        Some(Outcome::Run(cmd.run.clone()))
    }

    /// The palette as a level of the overlay stack ([`crate::frame::Frame::view_with`]
    /// stacks it over the toasts). `wrap` lifts the palette's messages into the app's.
    pub fn level<'a, M, N: Clone + 'a>(
        &self,
        cx: &Ctx,
        commands: &[Command<M>],
        placeholder: &str,
        wrap: impl Fn(Msg) -> N + 'a,
    ) -> Modal<'a, N> {
        let wrap = std::rc::Rc::new(wrap);
        let on = cx.on_card();
        let p = on.p();
        let rows = filter(commands, &self.query);
        let heads = headers(commands, &rows);

        let colors = (c(p.text), c(p.muted), rgba(p.accent, 0.35));
        let search = text_input(placeholder, &self.query)
            .on_input({
                let wrap = wrap.clone();
                move |q| wrap(Msg::Query(q))
            })
            .font(crate::fonts::sans())
            .size(14.0)
            .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
            .padding(Padding::from([14.0, 0.0]))
            .width(Length::Fill)
            .style(move |_, _| text_input::Style {
                background: Background::Color(Color::TRANSPARENT),
                border: Border::default(),
                icon: colors.1,
                placeholder: colors.1,
                value: colors.0,
                selection: colors.2,
            });
        let head = container(
            row![icon(Ico::Search, 16.0).color(c(p.muted)), search]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .padding(Padding::from([0.0, 16.0]));

        let mut list: Vec<El<'a, N>> = Vec::new();
        if rows.is_empty() {
            list.push(
                container(
                    label("No commands found", 14.0, 20.0)
                        .color(c(p.muted))
                        .align(crate::label::Align::Center)
                        .width(Length::Fill),
                )
                .padding(Padding::from([24.0, 16.0]))
                .into(),
            );
        }
        for (pos, (&i, head)) in rows.iter().zip(&heads).enumerate() {
            let cmd = &commands[i];
            if let Some(g) = head {
                list.push(
                    container(label(g.to_string(), 11.0, 16.5).medium().color(c(p.muted)))
                        .padding(Padding {
                            top: 10.0,
                            right: 10.0,
                            bottom: 4.0,
                            left: 10.0,
                        })
                        .into(),
                );
            }
            let active = pos == self.active;
            let mut items: Vec<El<'a, N>> = Vec::new();
            if let Some(ico) = cmd.icon {
                items.push(
                    icon(ico, 15.0)
                        .color(c(if active { p.accent_bright } else { p.muted }))
                        .into(),
                );
            }
            items.push(
                label(cmd.label.clone(), 13.0, 19.5)
                    .ellipsis()
                    .color(c(if active { p.text } else { p.subtext }))
                    .width(Length::Fill)
                    .into(),
            );
            if let Some(h) = &cmd.hint {
                items.push(label(h.clone(), 12.0, 16.0).color(c(p.muted)).into());
            }
            let bg = if active { Some(c(on.wash(0.07))) } else { None };
            let r = surface(row(items).spacing(12).align_y(Alignment::Center))
                .width(Length::Fill)
                .height(36)
                .padding(Padding::from([0.0, 10.0]))
                .center_y()
                .on_press(wrap(Msg::Run(pos)))
                .focus_ring(crate::focus::Ring::plain(&on))
                .fixed(BoxStyle {
                    bg,
                    radius: 8.0,
                    ..BoxStyle::default()
                });
            list.push(
                container(on_enter(r, wrap(Msg::Hover(pos))))
                    .id(row_id(pos))
                    .into(),
            );
        }
        let body = scroll_area(&on, container(column(list)).padding(8)).max_height(320.0);

        let panel = surface(column![head, hline(c(on.line())), body])
            .width(Length::Fill)
            .clip()
            .fixed(BoxStyle {
                bg: Some(c(p.card)),
                radius: 16.0,
                shadows: shadows(&p.shadow_lg),
                ..BoxStyle::default()
            });
        let keys = Keys {
            content: panel.into(),
            up: wrap(Msg::Up),
            down: wrap(Msg::Down),
            enter: wrap(Msg::Enter),
        };
        level(Kind::Top, keys, wrap(Msg::Close))
    }
}

// ── Widgets ─────────────────────────────────────────────────────────────

/// ↑ ↓ Enter on the search field (React's `onKeyDown` on the input): only
/// while a text field has focus ([`crate::hotkeys::typing`]); a focused row
/// keeps Enter for itself (a click), like a focused `<button>`. Esc goes
/// past the panel to the level.
struct Keys<'a, M> {
    content: El<'a, M>,
    up: M,
    down: M,
    enter: M,
}

/// Publishes `msg` when the pointer enters `content` (`onMouseEnter`).
pub fn on_enter<'a, M: Clone + 'a>(content: impl Into<El<'a, M>>, msg: M) -> El<'a, M> {
    El::new(Enter {
        content: content.into(),
        msg,
    })
}

struct Enter<'a, M> {
    content: El<'a, M>,
    msg: M,
}

#[derive(Default)]
struct Inside(bool);

macro_rules! delegate {
    () => {
        fn size(&self) -> Size<Length> {
            self.content.as_widget().size()
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
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits)
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
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
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
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
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
                layout,
                renderer,
                operation,
            );
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
                layout,
                renderer,
                viewport,
                translation,
            )
        }
    };
}
pub(crate) use delegate;

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Enter<'a, M> {
    delegate!();
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Inside>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Inside::default())
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
        if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            let over = cursor.is_over(layout.bounds());
            let st = tree.state.downcast_mut::<Inside>();
            if over && !st.0 {
                shell.publish(self.msg.clone());
            }
            st.0 = over;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Keys<'a, M> {
    delegate!();
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
        // Esc must reach the level (which closes the palette): iced's text
        // input would take it to blur itself (`input::Halo` does the same).
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(Named::Escape),
            ..
        }) = event
        {
            return;
        }
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modifiers,
            ..
        }) = event
            && crate::hotkeys::typing()
            && !modifiers.control()
            && !modifiers.alt()
            && !modifiers.logo()
        {
            let msg = match named {
                Named::ArrowDown => Some(&self.down),
                Named::ArrowUp => Some(&self.up),
                Named::Enter => Some(&self.enter),
                _ => None,
            };
            if let Some(m) = msg {
                shell.publish(m.clone());
                shell.capture_event();
                return;
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
}

impl<'a, M: Clone + 'a> From<Keys<'a, M>> for El<'a, M> {
    fn from(k: Keys<'a, M>) -> Self {
        El::new(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds() -> Vec<Command<&'static str>> {
        vec![
            Command::new("home", "Go to Home", "home")
                .icon(Ico::Home)
                .group("Navigation"),
            Command::new("settings", "Open Settings", "settings")
                .icon(Ico::Settings)
                .group("Navigation"),
            Command::new("new", "New File", "new")
                .group("Actions")
                .hint("⌘N"),
            Command::new("add", "Add Item", "add").group("Actions"),
            Command::new("dark", "Dark theme", "dark")
                .group("Theme")
                .keywords("appearance"),
            Command::new("loose", "Ungrouped", "loose"),
        ]
    }

    #[test]
    fn filter_matches_label_keywords_and_group_like_react() {
        let c = cmds();
        assert_eq!(filter(&c, ""), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(filter(&c, "   "), vec![0, 1, 2, 3, 4, 5], "blank = all");
        assert_eq!(filter(&c, "SETTINGS"), vec![1], "case-insensitive");
        assert_eq!(filter(&c, "appear"), vec![4], "keywords");
        assert_eq!(filter(&c, "actions"), vec![2, 3], "group");
        assert_eq!(filter(&c, " file "), vec![2], "trimmed");
        assert_eq!(filter(&c, "d i"), vec![3], "a substring with a space");
        assert!(filter(&c, "zzz").is_empty());
    }

    #[test]
    fn headers_follow_filtered_order() {
        let c = cmds();
        let all = filter(&c, "");
        assert_eq!(
            headers(&c, &all),
            vec![
                Some("Navigation"),
                None,
                Some("Actions"),
                None,
                Some("Theme"),
                None
            ]
        );
        let rows = filter(&c, "new");
        assert_eq!(headers(&c, &rows), vec![Some("Actions")]);
    }

    #[test]
    fn keyboard_navigation_clamps_and_runs() {
        let c = cmds();
        let mut p = Palette::default();
        p.open();
        assert_eq!(p.update(Msg::Up, &c), None);
        assert_eq!(p.active(), 0, "↑ stops at the first row");
        for _ in 0..10 {
            p.update(Msg::Down, &c);
        }
        assert_eq!(p.active(), 5, "↓ stops at the last row");
        p.update(Msg::Hover(2), &c);
        assert_eq!(p.active(), 2);
        assert_eq!(p.update(Msg::Enter, &c), Some(Outcome::Run("new")));
        assert!(!p.is_open(), "running closes");
    }

    #[test]
    fn query_resets_active_and_enter_on_nothing_is_a_no_op() {
        let c = cmds();
        let mut p = Palette::default();
        p.open();
        p.update(Msg::Down, &c);
        p.update(Msg::Query("theme".into()), &c);
        assert_eq!(p.active(), 0);
        assert_eq!(p.update(Msg::Enter, &c), Some(Outcome::Run("dark")));
        p.open();
        assert_eq!(p.query(), "", "reopening clears the query");
        p.update(Msg::Query("zzz".into()), &c);
        p.update(Msg::Down, &c);
        assert_eq!(p.active(), 0);
        assert_eq!(p.update(Msg::Enter, &c), None);
        assert!(p.is_open());
        assert_eq!(p.update(Msg::Close, &c), Some(Outcome::Closed));
        assert_eq!(p.update(Msg::Close, &c), None, "already closed");
    }

    #[test]
    fn click_runs_the_filtered_position() {
        let c = cmds();
        let mut p = Palette::default();
        p.open();
        p.update(Msg::Query("actions".into()), &c);
        assert_eq!(p.update(Msg::Run(1), &c), Some(Outcome::Run("add")));
        assert_eq!(c[2].clone().map(|s| s.len()).run, 3);
    }
}
