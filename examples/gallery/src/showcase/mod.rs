//! The Showcase ("Examples"): `ShowcasePage.tsx`. A title, the full-width
//! `Tabs` bar and the cards of the selected tab (docs/CARDS.md).
//!
//! Every card is a small component in `showcase/<tab>/<card>.rs` with its own
//! `State`, `Msg`, `update` and `view`; the tab's `mod.rs` lists the cards
//! once ([`tab_cards!`]). Each card is wrapped in a container with the id
//! [`card_id`], which `BATO_VIEW` `card=` scrolls to.

pub mod dashboard;
pub mod data;
pub mod date;
pub mod feedback;
pub mod inputs;
pub mod layout;
pub mod overlays;

use bato_kit::Tone;
use bato_kit::iced::widget::Id;
use bato_kit::iced::widget::column;
use bato_kit::prelude::*;

/// What a card asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Toast(String, Tone),
    /// A FileDropzone click: open the portal file chooser and send the
    /// chosen paths back as `reply(paths)` (React's `onPaths`).
    OpenFiles {
        multiple: bool,
        reply: Reply,
    },
}

/// Where the chosen paths of [`Effect::OpenFiles`] go.
#[derive(Clone, Copy)]
pub struct Reply(pub fn(Vec<std::path::PathBuf>) -> Msg);

impl PartialEq for Reply {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

impl std::fmt::Debug for Reply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reply")
    }
}

/// The overlay levels the Showcase has open, bottom first (the Triggers
/// card's ConfirmDialog, Drawer and CommandPalette).
pub fn levels<'a>(state: &'a State, cx: &Ctx) -> Vec<bato_kit::modal::Modal<'a, Msg>> {
    if state.tab != Tab::Overlays {
        return Vec::new();
    }
    overlays::triggers::levels(&state.overlays.triggers, cx)
        .into_iter()
        .map(|l| l.map(|m| Msg::Overlays(overlays::Msg::Triggers(m))))
        .collect()
}

/// Declares a tab's cards: `module: Variant => "card-id"`, in page order.
/// Generates the tab's `State`, `Msg`, `IDS`, `update` and `cards`.
macro_rules! tab_cards {
    ($($m:ident: $v:ident => $id:literal),* $(,)?) => {
        $(pub mod $m;)*

        #[derive(Debug, Default)]
        pub struct State {
            $(pub $m: $m::State,)*
        }

        #[derive(Debug, Clone)]
        pub enum Msg {
            $($v($m::Msg),)*
        }

        /// Card ids in page order.
        pub const IDS: &[&str] = &[$($id),*];

        pub fn update(state: &mut State, msg: Msg) -> Option<$crate::showcase::Effect> {
            match msg {
                $(Msg::$v(m) => $m::update(&mut state.$m, m),)*
            }
        }

        /// `(card id, view)` in page order.
        pub fn cards<'a>(state: &'a State, cx: &bato_kit::Ctx) -> Vec<(&'static str, bato_kit::El<'a, Msg>)> {
            vec![$(($id, $m::view(&state.$m, cx).map(Msg::$v)),)*]
        }
    };
}
pub(crate) use tab_cards;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Inputs,
    Date,
    Feedback,
    Layout,
    Dashboard,
    Overlays,
    Data,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Inputs,
        Tab::Date,
        Tab::Feedback,
        Tab::Layout,
        Tab::Dashboard,
        Tab::Overlays,
        Tab::Data,
    ];

    /// The id in `BATO_VIEW` `tab=`.
    pub fn id(self) -> &'static str {
        match self {
            Tab::Inputs => "inputs",
            Tab::Date => "date",
            Tab::Feedback => "feedback",
            Tab::Layout => "layout",
            Tab::Dashboard => "dashboard",
            Tab::Overlays => "overlays",
            Tab::Data => "data",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Data => "Data & Viz",
            Tab::Inputs => "Inputs",
            Tab::Date => "Date",
            Tab::Feedback => "Feedback",
            Tab::Layout => "Layout",
            Tab::Dashboard => "Dashboard",
            Tab::Overlays => "Overlays",
        }
    }

    pub fn parse(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The card ids of this tab, in page order.
    pub fn cards(self) -> &'static [&'static str] {
        match self {
            Tab::Inputs => inputs::IDS,
            Tab::Date => date::IDS,
            Tab::Feedback => feedback::IDS,
            Tab::Layout => layout::IDS,
            Tab::Dashboard => dashboard::IDS,
            Tab::Overlays => overlays::IDS,
            Tab::Data => data::IDS,
        }
    }

    /// The tab a card id belongs to (`card=` without `tab=`).
    pub fn of_card(card: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.cards().contains(&card))
    }
}

/// The container id of a card, for `card=` and `scroll_to`.
pub fn card_id(tab: Tab, card: &str) -> Id {
    Id::from(format!("card:{}/{card}", tab.id()))
}

#[derive(Debug, Default)]
pub struct State {
    pub tab: Tab,
    pub inputs: inputs::State,
    pub date: date::State,
    pub feedback: feedback::State,
    pub layout: layout::State,
    pub dashboard: dashboard::State,
    pub overlays: overlays::State,
    pub data: data::State,
}

impl State {
    pub fn new(tab: Tab) -> Self {
        Self {
            tab,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Tab(Tab),
    Inputs(inputs::Msg),
    Date(date::Msg),
    Feedback(feedback::Msg),
    Layout(layout::Msg),
    Dashboard(dashboard::Msg),
    Overlays(overlays::Msg),
    Data(data::Msg),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Tab(t) => {
            state.tab = t;
            None
        }
        Msg::Inputs(m) => inputs::update(&mut state.inputs, m),
        Msg::Date(m) => date::update(&mut state.date, m),
        Msg::Feedback(m) => feedback::update(&mut state.feedback, m),
        Msg::Layout(m) => layout::update(&mut state.layout, m),
        Msg::Dashboard(m) => dashboard::update(&mut state.dashboard, m),
        Msg::Overlays(m) => overlays::update(&mut state.overlays, m),
        Msg::Data(m) => data::update(&mut state.data, m),
    }
}

fn mapped<'a, M: 'a>(
    cards: Vec<(&'static str, El<'a, M>)>,
    f: fn(M) -> Msg,
) -> Vec<(&'static str, El<'a, Msg>)> {
    cards.into_iter().map(|(id, el)| (id, el.map(f))).collect()
}

/// The page: `mx-auto p-6 space-y-6`, content width unless `wide`.
pub fn view<'a>(state: &'a State, cx: &Ctx, wide: bool) -> El<'a, Msg> {
    let p = cx.p();
    let header = column![
        label("Component kit", 20.0, 28.0)
            .semibold()
            .color(c(p.text)),
        vgap(4.0),
        label(
            "A library for building almost any app — themed by the active accent.",
            14.0,
            20.0
        )
        .color(c(p.subtext)),
    ];
    let bar = tabs(
        cx,
        state.tab,
        Tab::ALL.map(|t| Segment::new(t, t.label())),
        Msg::Tab,
    );
    let cards = match state.tab {
        Tab::Inputs => mapped(inputs::cards(&state.inputs, cx), Msg::Inputs),
        Tab::Date => mapped(date::cards(&state.date, cx), Msg::Date),
        Tab::Feedback => mapped(feedback::cards(&state.feedback, cx), Msg::Feedback),
        Tab::Layout => mapped(layout::cards(&state.layout, cx), Msg::Layout),
        Tab::Dashboard => mapped(dashboard::cards(&state.dashboard, cx), Msg::Dashboard),
        Tab::Overlays => mapped(overlays::cards(&state.overlays, cx), Msg::Overlays),
        Tab::Data => mapped(data::cards(&state.data, cx), Msg::Data),
    };
    let mut col = column![header, bar].spacing(24).width(Length::Fill);
    for (id, el) in cards {
        col = col.push(container(el).id(card_id(state.tab, id)).width(Length::Fill));
    }
    page(col, wide, 24)
}

/// `BATO_SELFTEST=<name>` scripts of the Showcase tabs (open the tab with
/// `BATO_VIEW`).
pub fn selftest(name: &str) -> Option<Vec<bato_kit::overlay::script::Step<Msg>>> {
    use bato_kit::overlay::script::Step;
    fn map<M>(steps: Vec<Step<M>>, f: fn(M) -> Msg) -> Vec<Step<Msg>> {
        steps.into_iter().map(|s| s.map(f)).collect()
    }
    match name {
        "feedback" => Some(map(feedback::selftest::steps(), Msg::Feedback)),
        "pickers" => Some(overlays::triggers::selftest()),
        "layout" => Some(map(layout::selftest::steps(), Msg::Layout)),
        "graph" => Some(data::node_graph::selftest()),
        "editor" => Some(map(inputs::code_editor::selftest(), |m| {
            Msg::Inputs(inputs::Msg::CodeEditor(m))
        })),
        "dnd" => Some(data::sortable::selftest()),
        "dnd-tabs" => Some(layout::editor_tabs::selftest()),
        n if n.starts_with("data:") => data::table::b6_selftest(&n[5..]).map(|v| map(v, Msg::Data)),
        _ => None,
    }
}

/// A card that is not ported yet: the real Card look, its title, and
/// "Not ported yet".
pub fn stub<'a, M: Clone + 'a>(cx: &Ctx, title: &str) -> El<'a, M> {
    let p = cx.p();
    card(cx, title)
        .body(label("Not ported yet", 13.0, 19.5).color(c(p.muted)))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The card ids and their order are docs/CARDS.md's.
    #[test]
    fn cards_match_the_spec() {
        let spec = include_str!("../../../../docs/CARDS.md");
        for tab in Tab::ALL {
            let line = spec
                .lines()
                .find(|l| l.starts_with(&format!("| `{}`", tab.id())))
                .expect("tab row");
            let ids: Vec<&str> = line
                .split('|')
                .nth(2)
                .unwrap()
                .split('`')
                .skip(1)
                .step_by(2)
                .collect();
            assert_eq!(ids, tab.cards(), "{}", tab.id());
        }
        assert_eq!(Tab::of_card("charts"), Some(Tab::Data));
        assert_eq!(Tab::parse("overlays"), Some(Tab::Overlays));
        let n: usize = Tab::ALL.iter().map(|t| t.cards().len()).sum();
        assert_eq!(n, 32);
    }
}
