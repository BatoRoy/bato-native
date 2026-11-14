//! Keyboard shortcuts (INF-5): `useHotkeys` on `iced::event::listen_with`.
//!
//! Combos are written like React's: `mod+k`, `shift+?`, `alt+n`, `escape`.
//! `mod` is Ctrl on Linux/Windows and ⌘ on macOS (`ctrl`/`cmd` are accepted
//! as spellings of it); the key is the produced character (`?`, not `/`) or
//! a key name (`escape`, `enter`, `arrowdown`, `f2`, `space`).
//!
//! **Ignored while typing**: like React, a press is skipped when the focused
//! widget is a text field ([`KeyPress::typing`]), and also when a widget
//! consumed it (the topmost dialog took Escape, a menu took an arrow), unless
//! the binding says [`Hotkeys::in_inputs`]. "Typing" is kept up to date by
//! [`probe`], which the frame wraps around the whole window; it recognises
//! iced's `text_input` and any widget that reports [`TypingTarget`] through
//! `Operation::custom` before its `focusable` (a code editor, an OTP field).
//!
//! **Scoped vs global**: a global binding is always live; a scoped one only
//! while its scope is active ([`Hotkeys::enter`] / [`Hotkeys::leave`], e.g.
//! while the command palette is open). The innermost active scope wins over
//! the outer ones, and scopes win over global bindings.
//!
//! ```ignore
//! // state
//! keys: Hotkeys<Msg> = Hotkeys::new()
//!     .bind("mod+k", Msg::OpenPalette)
//!     .bind_in("palette", "escape", Msg::ClosePalette).in_inputs(),
//! // subscription: only the bound combos ever produce a message
//! self.keys.subscription().map(Msg::Key)
//! // update
//! Msg::Key(press) => if let Some(m) = self.keys.resolve(&press) { return self.update(m) }
//! ```

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, operation};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Element, Event, Length, Rectangle, Size, Subscription, Vector, event, mouse, window};

const MOD: u8 = 1;
const ALT: u8 = 2;
const SHIFT: u8 = 4;

/// A normalised key combination (`mod+shift+k`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Combo {
    mods: u8,
    /// `KeyboardEvent.key`, lowercased (`k`, `?`, `escape`, `arrowdown`, ` `).
    key: String,
}

impl Combo {
    /// Parse `mod+k`, `shift+?`, `Escape`, `mod++`. `None` without a key.
    pub fn parse(s: &str) -> Option<Combo> {
        let s = s.trim().to_lowercase();
        let (head, plus) = if s == "+" {
            ("", true)
        } else if let Some(h) = s.strip_suffix("++") {
            (h, true)
        } else {
            (s.as_str(), false)
        };
        let mut mods = 0;
        let mut key = plus.then(|| "+".to_string());
        for part in head.split('+') {
            match part {
                "" => {}
                "mod" | "ctrl" | "control" | "cmd" | "meta" => mods |= MOD,
                "alt" | "option" => mods |= ALT,
                "shift" => mods |= SHIFT,
                k => key = Some(alias(k).to_string()),
            }
        }
        key.map(|key| Combo { mods, key })
    }

    /// The combo a key press produces (React's `onKey`): `None` for a bare
    /// modifier press.
    pub fn from_key(modified_key: &Key, modifiers: Modifiers) -> Option<Combo> {
        let key = match modified_key.as_ref() {
            Key::Character(c) => c.to_lowercase(),
            Key::Named(
                Named::Control
                | Named::Shift
                | Named::Alt
                | Named::Meta
                | Named::Super
                | Named::Hyper
                | Named::AltGraph,
            ) => return None,
            Key::Named(Named::Space) => " ".to_string(),
            Key::Named(n) => format!("{n:?}").to_lowercase(),
            Key::Unidentified => return None,
        };
        let mut mods = 0;
        if modifiers.command() {
            mods |= MOD;
        }
        if modifiers.alt() {
            mods |= ALT;
        }
        if modifiers.shift() {
            mods |= SHIFT;
        }
        Some(Combo { mods, key })
    }
}

fn alias(k: &str) -> &str {
    match k {
        "esc" => "escape",
        "space" | "spacebar" => " ",
        "up" => "arrowup",
        "down" => "arrowdown",
        "left" => "arrowleft",
        "right" => "arrowright",
        "return" => "enter",
        "del" => "delete",
        k => k,
    }
}

impl fmt::Display for Combo {
    /// React's spelling: `mod+alt+shift+key`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods & MOD != 0 {
            f.write_str("mod+")?;
        }
        if self.mods & ALT != 0 {
            f.write_str("alt+")?;
        }
        if self.mods & SHIFT != 0 {
            f.write_str("shift+")?;
        }
        f.write_str(if self.key == " " { "space" } else { &self.key })
    }
}

/// One key press, as the subscription reports it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub combo: Combo,
    /// A text field had focus ([`probe`]).
    pub typing: bool,
    /// A widget consumed the press.
    pub captured: bool,
}

impl KeyPress {
    /// A press of `combo` nobody consumed, outside text fields (tests,
    /// scripted runs).
    pub fn of(combo: &str) -> Option<KeyPress> {
        Combo::parse(combo).map(|combo| KeyPress {
            combo,
            typing: false,
            captured: false,
        })
    }
}

static TYPING: AtomicBool = AtomicBool::new(false);

/// Whether a text field had focus at the last key press.
pub fn typing() -> bool {
    TYPING.load(Ordering::Relaxed)
}

fn on_event(event: Event, status: event::Status, _: window::Id) -> Option<KeyPress> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key,
            modifiers,
            ..
        }) => Combo::from_key(&modified_key, modifiers).map(|combo| KeyPress {
            combo,
            typing: typing(),
            captured: status == event::Status::Captured,
        }),
        _ => None,
    }
}

/// Every key press in the window (prefer [`Hotkeys::subscription`], which
/// only reports bound combos).
pub fn listen() -> Subscription<KeyPress> {
    iced::event::listen_with(on_event)
}

#[derive(Debug, Clone)]
struct Binding<M> {
    combo: Combo,
    msg: M,
    scope: Option<&'static str>,
    in_inputs: bool,
}

/// A hotkey registry.
#[derive(Debug, Clone)]
pub struct Hotkeys<M> {
    bindings: Vec<Binding<M>>,
    scopes: Vec<&'static str>,
}

impl<M> Default for Hotkeys<M> {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            scopes: Vec::new(),
        }
    }
}

impl<M: Clone> Hotkeys<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A global binding. Panics on an unparsable combo (a programming error).
    pub fn bind(self, combo: &str, msg: M) -> Self {
        self.push(None, combo, msg)
    }

    /// A binding live only while `scope` is active.
    pub fn bind_in(self, scope: &'static str, combo: &str, msg: M) -> Self {
        self.push(Some(scope), combo, msg)
    }

    fn push(mut self, scope: Option<&'static str>, combo: &str, msg: M) -> Self {
        let combo = Combo::parse(combo).unwrap_or_else(|| panic!("hotkey {combo:?}"));
        self.bindings.push(Binding {
            combo,
            msg,
            scope,
            in_inputs: false,
        });
        self
    }

    /// The last binding also fires while typing in a field (and when a
    /// widget consumed the key).
    pub fn in_inputs(mut self) -> Self {
        if let Some(b) = self.bindings.last_mut() {
            b.in_inputs = true;
        }
        self
    }

    /// Activate a scope (innermost last).
    pub fn enter(&mut self, scope: &'static str) {
        self.scopes.retain(|s| *s != scope);
        self.scopes.push(scope);
    }

    pub fn leave(&mut self, scope: &'static str) {
        self.scopes.retain(|s| *s != scope);
    }

    pub fn is_active(&self, scope: &'static str) -> bool {
        self.scopes.contains(&scope)
    }

    /// The bound combos (what the subscription listens for), each once.
    pub fn combos(&self) -> Vec<Combo> {
        let mut v: Vec<Combo> = Vec::new();
        for b in &self.bindings {
            if !v.contains(&b.combo) {
                v.push(b.combo.clone());
            }
        }
        v
    }

    /// The message for `press`, if a live binding matches.
    pub fn resolve(&self, press: &KeyPress) -> Option<M> {
        let usable = |b: &&Binding<M>| {
            b.combo == press.combo && (b.in_inputs || (!press.typing && !press.captured))
        };
        for scope in self.scopes.iter().rev() {
            if let Some(b) = self
                .bindings
                .iter()
                .filter(|b| b.scope == Some(*scope))
                .find(usable)
            {
                return Some(b.msg.clone());
            }
        }
        self.bindings
            .iter()
            .filter(|b| b.scope.is_none())
            .find(usable)
            .map(|b| b.msg.clone())
    }

    /// Key presses of the bound combos only; map them into the app's
    /// message and [`Hotkeys::resolve`] them in `update`.
    pub fn subscription(&self) -> Subscription<KeyPress> {
        if self.bindings.is_empty() {
            return Subscription::none();
        }
        listen()
            .with(self.combos())
            .filter_map(|(combos, press)| combos.contains(&press.combo).then_some(press))
    }
}

/// A marker a typing widget (code editor, OTP field) reports through
/// `Operation::custom` right before its `focusable`, so the [`probe`] counts
/// it as a text field.
pub struct TypingTarget;

/// Finds whether a text field has focus.
#[derive(Default)]
struct FocusedText {
    text_next: bool,
    typing: bool,
}

impl widget::Operation for FocusedText {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn text_input(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        _state: &mut dyn operation::TextInput,
    ) {
        self.text_next = true;
    }
    fn custom(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn std::any::Any,
    ) {
        if state.is::<TypingTarget>() {
            self.text_next = true;
        }
    }
    fn focusable(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn operation::Focusable,
    ) {
        if self.text_next && state.is_focused() {
            self.typing = true;
        }
        self.text_next = false;
    }
}

/// Keeps [`typing`] current: wrap the whole window in it (the frame does).
/// On each key press it checks, before the widgets see the key, whether a
/// text field has focus (the browser's `e.target`).
pub struct Probe<'a, M> {
    content: Element<'a, M, iced::Theme, iced::Renderer>,
}

pub fn probe<'a, M>(
    content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>,
) -> Probe<'a, M> {
    Probe {
        content: content.into(),
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Probe<'a, M> {
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
        if let Event::Keyboard(keyboard::Event::KeyPressed { .. }) = event {
            let mut op = FocusedText::default();
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], layout, renderer, &mut op);
            let mut typing = op.typing;
            if !typing
                && let Some(ov) = self.content.as_widget_mut().overlay(
                    &mut tree.children[0],
                    layout,
                    renderer,
                    viewport,
                    Vector::ZERO,
                )
            {
                let mut nested = overlay::Nested::new(ov);
                let node = nested.layout(renderer, layout.bounds().size());
                let mut op = FocusedText::default();
                nested.operate(Layout::new(&node), renderer, &mut op);
                typing = op.typing;
            }
            TYPING.store(typing, Ordering::Relaxed);
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
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
}

impl<'a, M: 'a> From<Probe<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(p: Probe<'a, M>) -> Self {
        Element::new(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(s: &str) -> KeyPress {
        KeyPress::of(s).unwrap()
    }

    #[test]
    fn parse_and_print() {
        assert_eq!(Combo::parse("Mod+K").unwrap().to_string(), "mod+k");
        assert_eq!(Combo::parse("ctrl+s"), Combo::parse("mod+s"));
        assert_eq!(
            Combo::parse("shift+alt+mod+p").unwrap().to_string(),
            "mod+alt+shift+p"
        );
        assert_eq!(Combo::parse("esc"), Combo::parse("Escape"));
        assert_eq!(Combo::parse("space").unwrap().to_string(), "space");
        assert_eq!(Combo::parse("mod++").unwrap().to_string(), "mod++");
        assert_eq!(Combo::parse("+").unwrap().to_string(), "+");
        assert_eq!(Combo::parse("mod+"), None);
    }

    #[test]
    fn from_key_like_keyboard_event_key() {
        let ctrl = Modifiers::CTRL;
        let k = Combo::from_key(&Key::Character("k".into()), ctrl).unwrap();
        assert_eq!(k.to_string(), "mod+k");
        // Shift is applied to the character: shift+/ is "shift+?".
        let q = Combo::from_key(&Key::Character("?".into()), Modifiers::SHIFT).unwrap();
        assert_eq!(Some(q), Combo::parse("shift+?"));
        // Ctrl+Shift+K reports "K"; lowercased like React.
        let k = Combo::from_key(&Key::Character("K".into()), ctrl | Modifiers::SHIFT).unwrap();
        assert_eq!(Some(k), Combo::parse("mod+shift+k"));
        let esc = Combo::from_key(&Key::Named(Named::Escape), Modifiers::default());
        assert_eq!(esc, Combo::parse("escape"));
        let down = Combo::from_key(&Key::Named(Named::ArrowDown), Modifiers::default());
        assert_eq!(down, Combo::parse("down"));
        let sp = Combo::from_key(&Key::Named(Named::Space), Modifiers::default());
        assert_eq!(sp, Combo::parse("space"));
        assert_eq!(Combo::from_key(&Key::Named(Named::Control), ctrl), None);
        let f2 = Combo::from_key(&Key::Named(Named::F2), Modifiers::default());
        assert_eq!(f2, Combo::parse("f2"));
    }

    #[test]
    fn typing_and_captured_presses_are_ignored_unless_allowed() {
        let keys = Hotkeys::new().bind("mod+k", 1).bind("mod+s", 2).in_inputs();
        assert_eq!(keys.resolve(&press("mod+k")), Some(1));
        let typing = KeyPress {
            typing: true,
            ..press("mod+k")
        };
        assert_eq!(keys.resolve(&typing), None);
        let typing_s = KeyPress {
            typing: true,
            ..press("mod+s")
        };
        assert_eq!(keys.resolve(&typing_s), Some(2));
        let captured = KeyPress {
            captured: true,
            ..press("mod+k")
        };
        assert_eq!(keys.resolve(&captured), None);
        assert_eq!(keys.resolve(&press("k")), None);
    }

    #[test]
    fn scopes_win_innermost_first() {
        let mut keys = Hotkeys::new()
            .bind("escape", "global")
            .bind_in("drawer", "escape", "drawer")
            .bind_in("palette", "escape", "palette")
            .bind_in("palette", "enter", "run");
        assert_eq!(keys.resolve(&press("escape")), Some("global"));
        assert_eq!(keys.resolve(&press("enter")), None);
        keys.enter("drawer");
        assert_eq!(keys.resolve(&press("escape")), Some("drawer"));
        keys.enter("palette");
        assert_eq!(keys.resolve(&press("escape")), Some("palette"));
        assert_eq!(keys.resolve(&press("enter")), Some("run"));
        keys.leave("palette");
        assert_eq!(keys.resolve(&press("escape")), Some("drawer"));
        assert!(!keys.is_active("palette"));
        assert_eq!(keys.combos().len(), 2);
    }
}
