//! Clipboard and vim keymap tests: the widget's key decoding and the
//! document's vim layer, against a fake clipboard.

use iced::advanced::clipboard::{Clipboard, Kind};
use iced::advanced::graphics::text::cosmic_text as ct;
use iced::advanced::graphics::text::font_system;
use iced::keyboard::key::{Code, Named, Physical};
use iced::keyboard::{Key, Modifiers};

use super::Content;
use super::engine::{Cmd, Doc, Move, Outcome, Register, VimKey};
use super::syntax::Language;
use super::view::DocView;

/// The system clipboard and the primary selection, in memory.
#[derive(Default)]
struct Fake {
    std: Option<String>,
    primary: Option<String>,
    writes: Vec<(Kind, String)>,
}

impl Clipboard for Fake {
    fn read(&self, kind: Kind) -> Option<String> {
        match kind {
            Kind::Standard => self.std.clone(),
            Kind::Primary => self.primary.clone(),
        }
    }
    fn write(&mut self, kind: Kind, contents: String) {
        self.writes.push((kind, contents.clone()));
        match kind {
            Kind::Standard => self.std = Some(contents),
            Kind::Primary => self.primary = Some(contents),
        }
    }
}

fn palette() -> bato_theme::Palette {
    bato_theme::palette(
        bato_theme::ThemeId::Dark,
        bato_theme::Rgb::new(124, 58, 237),
    )
}

fn content(text: &str, vim: bool) -> Content {
    crate::fonts::setup();
    let c = Content::new(text);
    {
        let mut fs = font_system().write().unwrap();
        let mut d = c.0.borrow_mut();
        d.configure(fs.raw(), &palette(), Language::Text, vim);
        d.refresh(fs.raw());
    }
    c
}

fn view(c: &Content, vim: bool) -> DocView<'_, ()> {
    DocView {
        content: c,
        palette: palette(),
        lang: Language::Text,
        vim,
        line_numbers: true,
        read_only: false,
        placeholder: None,
        on_change: None,
        id: None,
        view_h: 200.0,
        round_bottom: true,
    }
}

fn ch(c: &str) -> Key {
    Key::Character(c.into())
}

fn code(c: char) -> Physical {
    Physical::Code(match c {
        'a' => Code::KeyA,
        'c' => Code::KeyC,
        'v' => Code::KeyV,
        'x' => Code::KeyX,
        _ => Code::KeyZ,
    })
}

/// A Ctrl(+Shift) chord through the widget's plain-mode keymap.
fn plain(v: &DocView<'_, ()>, k: &str, m: Modifiers, clip: &mut Fake) -> Option<Outcome> {
    let key = ch(k);
    let latin = key.to_latin(code(k.chars().next().unwrap()));
    v.plain_key(&key, latin, m, None, clip, 100.0)
}

fn cursor(c: &Content) -> (usize, usize) {
    let cur = c.0.borrow().cursor();
    (cur.line, cur.index)
}

// ── Plain mode (CodeMirror keys) ─────────────────────────────────────────

#[test]
fn plain_select_all_copy_paste() {
    let c = content("one\ntwo", false);
    let v = view(&c, false);
    let mut clip = Fake::default();
    plain(&v, "a", Modifiers::CTRL, &mut clip).unwrap();
    plain(&v, "c", Modifiers::CTRL, &mut clip).unwrap();
    assert_eq!(clip.std.as_deref(), Some("one\ntwo"));
    // Ctrl+V replaces the selection; Ctrl+Shift+V pastes too.
    clip.std = Some("X".into());
    plain(&v, "v", Modifiers::CTRL, &mut clip).unwrap();
    assert_eq!(c.text(), "X");
    plain(&v, "v", Modifiers::CTRL | Modifiers::SHIFT, &mut clip).unwrap();
    assert_eq!(c.text(), "XX");
    assert_eq!(cursor(&c), (0, 2));
}

#[test]
fn plain_cut_and_linewise_copy() {
    let c = content("one\ntwo", false);
    let v = view(&c, false);
    let mut clip = Fake::default();
    // No selection: Ctrl+C copies the line (`lineWiseCopyCut`)…
    plain(&v, "c", Modifiers::CTRL, &mut clip).unwrap();
    assert_eq!(clip.std.as_deref(), Some("one\n"));
    // …and Ctrl+X cuts it.
    plain(&v, "x", Modifiers::CTRL, &mut clip).unwrap();
    assert_eq!(c.text(), "two");
    // Shift+Insert pastes, Ctrl+Insert copies.
    clip.std = Some("A".into());
    v.plain_key(
        &Key::Named(Named::Insert),
        None,
        Modifiers::SHIFT,
        None,
        &mut clip,
        100.0,
    )
    .unwrap();
    assert_eq!(c.text(), "Atwo");
}

#[test]
fn plain_shortcuts_follow_the_physical_key_on_other_layouts() {
    let c = content("abc", false);
    let v = view(&c, false);
    let mut clip = Fake::default();
    // Cyrillic layout: Ctrl+С (the C key) selects… Ctrl+Ф (A) all, then copies.
    let k = ch("ф");
    v.plain_key(
        &k,
        k.to_latin(code('a')),
        Modifiers::CTRL,
        None,
        &mut clip,
        100.0,
    )
    .unwrap();
    let k = ch("с");
    v.plain_key(
        &k,
        k.to_latin(code('c')),
        Modifiers::CTRL,
        None,
        &mut clip,
        100.0,
    )
    .unwrap();
    assert_eq!(clip.std.as_deref(), Some("abc"));
}

#[test]
fn middle_click_pastes_the_primary_selection() {
    let c = content("ab", false);
    let mut clip = Fake::default();
    let mut fs = font_system().write().unwrap();
    let mut d = c.0.borrow_mut();
    d.run(fs.raw(), &Cmd::Move(Move::CharRight, false), 100.0);
    d.paste_primary(fs.raw(), "XY", &mut clip);
    assert_eq!(d.text(), "aXYb");
}

// ── Vim mode ─────────────────────────────────────────────────────────────

struct Vim {
    c: Content,
    clip: Fake,
    read_only: bool,
}

impl Vim {
    fn new(text: &str) -> Self {
        Self {
            c: content(text, true),
            clip: Fake::default(),
            read_only: false,
        }
    }
    fn doc(&self) -> std::cell::RefMut<'_, Doc> {
        self.c.0.borrow_mut()
    }
    fn key(&mut self, k: VimKey) -> Outcome {
        let mut fs = font_system().write().unwrap();
        let mut d = self.c.0.borrow_mut();
        d.vim_input(fs.raw(), k, &mut self.clip, 10, self.read_only)
            .unwrap_or_default()
    }
    /// Typed keys; `␣` is Space.
    fn keys(&mut self, s: &str) -> Outcome {
        let mut out = Outcome::default();
        for c in s.chars() {
            out |= self.key(VimKey::Char(if c == '␣' { ' ' } else { c }));
        }
        out
    }
    fn esc(&mut self) {
        self.key(VimKey::Action(ct::Action::Escape));
    }
    fn text(&self) -> String {
        self.c.text()
    }
    fn cur(&self) -> (usize, usize) {
        cursor(&self.c)
    }
    fn mode(&self) -> ct::ViMode {
        self.c.0.borrow().vi_mode().unwrap()
    }
    fn reg(&self, name: char) -> Option<(String, bool)> {
        self.c
            .0
            .borrow()
            .vi_register(name)
            .map(|Register { text, linewise }| (text, linewise))
    }
    fn std_writes(&self) -> Vec<String> {
        self.clip
            .writes
            .iter()
            .filter(|(k, _)| *k == Kind::Standard)
            .map(|(_, t)| t.clone())
            .collect()
    }
}

const TEXT: &str = "one two\nthree\n  four";

#[test]
fn vim_plain_yank_and_delete_stay_inside() {
    let mut v = Vim::new(TEXT);
    v.keys("yy");
    assert_eq!(v.reg('"'), Some(("one two".into(), true)));
    v.keys("jdd");
    assert_eq!(v.text(), "one two\n  four");
    assert_eq!(v.reg('"'), Some(("three".into(), true)));
    assert_eq!(v.cur(), (1, 2), "dd: to the first non-blank");
    v.keys("p");
    assert_eq!(v.text(), "one two\n  four\nthree");
    assert!(v.clip.writes.is_empty(), "no `clipboard=unnamedplus`");
}

#[test]
fn vim_operators_fixed_from_upstream() {
    let mut v = Vim::new(TEXT);
    v.keys("2dd");
    assert_eq!(v.text(), "  four");
    let mut v = Vim::new(TEXT);
    v.keys("dj");
    assert_eq!(v.text(), "  four");
    assert_eq!(v.reg('"'), Some(("one two\nthree".into(), true)));
    let mut v = Vim::new(TEXT);
    v.keys("Gdd");
    assert_eq!(v.text(), "one two\nthree");
    // `x` yanks, so `xp` swaps two characters.
    let mut v = Vim::new(TEXT);
    v.keys("xp");
    assert_eq!(v.text(), "noe two\nthree\n  four");
    // Visual mode includes the cursor's character; the cursor goes back.
    let mut v = Vim::new(TEXT);
    v.keys("vey");
    assert_eq!(v.reg('"'), Some(("one".into(), false)));
    assert_eq!((v.cur(), v.mode()), ((0, 0), ct::ViMode::Normal));
    // Visual `p` replaces the selection and takes the replaced text.
    v.keys("wviwp");
    assert_eq!(v.text(), "one one\nthree\n  four");
    assert_eq!(v.reg('"'), Some(("two".into(), false)));
    assert_eq!(v.mode(), ct::ViMode::Normal);
}

#[test]
fn vim_leader_y_yanks_to_the_clipboard() {
    let mut v = Vim::new(TEXT);
    v.keys("j␣yy");
    assert_eq!(v.std_writes(), vec!["three\n".to_string()]);
    assert_eq!(v.reg('+'), Some(("three".into(), true)));
    assert_eq!(v.reg('"'), Some(("three".into(), true)), "\"\" follows");
    assert_eq!(v.cur(), (1, 0));
    // `<leader>yiw`: charwise, the cursor at the word's start.
    v.keys("k$␣yiw");
    assert_eq!(v.clip.std.as_deref(), Some("two"));
    assert_eq!(v.cur(), (0, 4));
    assert_eq!(
        v.std_writes(),
        vec!["three\n".to_string(), "two".to_string()]
    );
}

#[test]
fn vim_leader_y_in_visual_modes() {
    let mut v = Vim::new(TEXT);
    v.keys("wve␣y");
    assert_eq!(v.clip.std.as_deref(), Some("two"));
    assert_eq!((v.cur(), v.mode()), ((0, 4), ct::ViMode::Normal));
    v.keys("Vj␣y");
    assert_eq!(v.clip.std.as_deref(), Some("one two\nthree\n"));
    assert_eq!(v.cur(), (0, 4));
}

#[test]
fn vim_leader_pending_shows_and_times_out() {
    let mut v = Vim::new(TEXT);
    v.keys("␣");
    assert_eq!(v.doc().vi_pending(), "<20>");
    assert!(v.doc().vim_deadline().is_some());
    v.keys("y");
    assert_eq!(v.doc().vi_pending(), "\"+y", "showcmd after the mapping");
    v.esc();
    // Timeout: Space as typed (`l`).
    v.keys("␣");
    {
        let mut fs = font_system().write().unwrap();
        let mut d = v.c.0.borrow_mut();
        d.vim_timeout(fs.raw(), &mut v.clip);
    }
    assert_eq!(v.cur(), (0, 1));
    assert_eq!(v.doc().vi_pending(), "");
    // An unmapped key after the leader: Space, then the key.
    v.keys("␣j");
    assert_eq!(v.cur(), (1, 2));
}

#[test]
fn vim_ctrl_shift_v_pastes_the_clipboard() {
    let mut v = Vim::new(TEXT);
    v.clip.std = Some("XY".into());
    // Normal: after the cursor (`"+p`), the cursor on its last character.
    v.key(VimKey::Ctrl('v', true));
    assert_eq!(v.text(), "oXYne two\nthree\n  four");
    assert_eq!(v.cur(), (0, 2));
    // A trailing newline: linewise, below.
    v.clip.std = Some("new\n".into());
    v.key(VimKey::Ctrl('v', true));
    assert_eq!(v.text(), "oXYne two\nnew\nthree\n  four");
    assert_eq!(v.cur(), (1, 0));
    // Insert mode: at the cursor, staying in insert mode.
    v.clip.std = Some("ZZ".into());
    v.keys("A");
    v.key(VimKey::Ctrl('v', true));
    assert_eq!(v.text(), "oXYne two\nnewZZ\nthree\n  four");
    assert_eq!(v.mode(), ct::ViMode::Insert);
    // A typed `"+p` reads the clipboard too.
    v.esc();
    v.clip.std = Some("Q".into());
    v.keys("0\"+P");
    assert_eq!(v.text(), "oXYne two\nQnewZZ\nthree\n  four");
    // and `p` afterwards uses `""` again (modit kept `"+` pending).
    assert!(v.std_writes().is_empty());
}

#[test]
fn vim_ctrl_c_is_escape_and_never_copies() {
    let mut v = Vim::new(TEXT);
    v.keys("wi");
    assert_eq!(v.mode(), ct::ViMode::Insert);
    v.key(VimKey::Ctrl('c', false));
    assert_eq!(v.mode(), ct::ViMode::Normal);
    assert_eq!(v.cur(), (0, 3), "like Esc, one to the left");
    v.keys("ve");
    v.key(VimKey::Ctrl('c', false));
    assert_eq!(v.mode(), ct::ViMode::Normal);
    assert!(v.doc().selection_bounds().is_none());
    v.keys("Vj");
    v.key(VimKey::Ctrl('c', false));
    assert_eq!(v.mode(), ct::ViMode::Normal);
    // A pending operator or leader is cancelled too.
    v.keys("d");
    v.key(VimKey::Ctrl('c', false));
    v.keys("␣");
    v.key(VimKey::Ctrl('c', false));
    assert_eq!(v.doc().vi_pending(), "");
    assert_eq!(v.text(), TEXT);
    assert!(v.clip.writes.is_empty());
}

#[test]
fn vim_visual_leader_p_keeps_the_register() {
    let mut v = Vim::new("one two three");
    v.keys("yiw");
    v.keys("wviw␣p");
    assert_eq!(v.text(), "one one three");
    assert_eq!(v.reg('"'), Some(("one".into(), false)));
    assert_eq!(v.mode(), ct::ViMode::Normal);
    v.keys("wwviw␣p");
    assert_eq!(v.text(), "one one one");
    // Linewise: `V<leader>p` replaces the line.
    let mut v = Vim::new(TEXT);
    v.keys("yyjV␣p");
    assert_eq!(v.text(), "one two\none two\n  four");
    assert_eq!(v.reg('"'), Some(("one two".into(), true)));
}

#[test]
fn vim_leader_d_deletes_to_the_black_hole() {
    let mut v = Vim::new(TEXT);
    v.keys("yiw");
    v.keys("j␣dd");
    assert_eq!(v.text(), "one two\n  four");
    assert_eq!(v.reg('"'), Some(("one".into(), false)));
    v.keys("0␣dw");
    assert_eq!(v.text(), "one two\nfour");
    v.keys("kve␣d");
    assert_eq!(v.text(), " two\nfour");
    assert_eq!(v.reg('"'), Some(("one".into(), false)));
    assert!(v.clip.writes.is_empty());
}

#[test]
fn vim_visual_j_k_move_lines() {
    let mut v = Vim::new("a\nb\nc\nd");
    v.keys("Vj");
    v.keys("J");
    assert_eq!(v.text(), "c\na\nb\nd");
    assert_eq!(v.mode(), ct::ViMode::VisualLine);
    assert_eq!(v.cur(), (2, 0));
    v.keys("J");
    assert_eq!(v.text(), "c\nd\na\nb");
    v.keys("J");
    assert_eq!(v.text(), "c\nd\na\nb", "at the end: nothing");
    v.keys("KK");
    assert_eq!(v.text(), "a\nb\nc\nd");
    // One undo step each.
    v.esc();
    v.keys("u");
    assert_eq!(v.text(), "c\na\nb\nd");
}

#[test]
fn vim_j_joins_keeping_the_cursor() {
    let mut v = Vim::new("  if (x) {\n    y\n  }\n)");
    v.keys("ll");
    v.keys("J");
    assert_eq!(v.text(), "  if (x) { y\n  }\n)");
    assert_eq!(v.cur(), (0, 2));
    v.keys("3J");
    assert_eq!(v.text(), "  if (x) { y })");
    assert_eq!(v.cur(), (0, 2));
    v.keys("u");
    assert_eq!(v.text(), "  if (x) { y\n  }\n)");
}

#[test]
fn vim_half_pages_and_search_centre() {
    let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let mut v = Vim::new(text.trim_end());
    let o = v.key(VimKey::Ctrl('d', false));
    assert_eq!(v.cur().0, 5, "half of 10 lines");
    assert!(o.center);
    v.key(VimKey::Ctrl('d', false));
    v.key(VimKey::Ctrl('u', false));
    assert_eq!(v.cur().0, 5);
    v.keys("/line 3");
    v.key(VimKey::Action(ct::Action::Enter));
    assert_eq!(v.cur(), (30, 0));
    let o = v.keys("n");
    assert!(o.center);
    assert_eq!(v.cur(), (31, 0));
    let o = v.keys("N");
    assert!(o.center);
    assert_eq!(v.cur(), (30, 0));
}

#[test]
fn vim_read_only_yanks_but_never_edits() {
    let mut v = Vim::new(TEXT);
    v.read_only = true;
    v.keys("␣yy");
    assert_eq!(v.clip.std.as_deref(), Some("one two\n"));
    v.keys("␣ddJxp");
    v.keys("Vj␣pJ");
    v.esc();
    v.clip.std = Some("Z".into());
    v.key(VimKey::Ctrl('v', true));
    assert_eq!(v.text(), TEXT);
}

#[test]
fn vim_widget_decodes_chords() {
    let c = content(TEXT, true);
    let view = view(&c, true);
    let mut clip = Fake {
        std: Some("XY".into()),
        ..Fake::default()
    };
    // Ctrl+Shift+V: iced reports the unshifted key with Ctrl held.
    let k = ch("v");
    view.vim_key(
        &k,
        k.to_latin(code('v')),
        Modifiers::CTRL | Modifiers::SHIFT,
        None,
        &mut clip,
        10,
    )
    .unwrap();
    assert_eq!(c.text(), "oXYne two\nthree\n  four");
    // Space is a named key: the leader.
    view.vim_key(
        &Key::Named(Named::Space),
        None,
        Modifiers::empty(),
        None,
        &mut clip,
        10,
    )
    .unwrap();
    assert_eq!(c.0.borrow().vi_pending(), "<20>");
    // Ctrl+C: Escape, nothing copied.
    let k = ch("c");
    view.vim_key(
        &k,
        k.to_latin(code('c')),
        Modifiers::CTRL,
        None,
        &mut clip,
        10,
    )
    .unwrap();
    assert_eq!(clip.writes.len(), 0);
    assert_eq!(c.0.borrow().vi_pending(), "");
}
