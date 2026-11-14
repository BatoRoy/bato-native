//! Engine tests: CodeMirror's commands on a real cosmic-text buffer.

use iced::advanced::graphics::text::font_system;

use super::engine::{Cmd, Doc, Move};
use super::syntax::{self, Class, Language};

fn doc(text: &str, lang: Language) -> Doc {
    crate::fonts::setup();
    let mut d = Doc::new(text);
    let mut fs = font_system().write().unwrap();
    let p = bato_theme::palette(
        bato_theme::ThemeId::Dark,
        bato_theme::Rgb::new(124, 58, 237),
    );
    d.configure(fs.raw(), &p, lang, false);
    d.refresh(fs.raw());
    d
}

fn run(d: &mut Doc, cmds: &[Cmd]) {
    let mut fs = font_system().write().unwrap();
    for c in cmds {
        d.run(fs.raw(), c, 100.0);
    }
}

fn typed(s: &str) -> Vec<Cmd> {
    s.chars().map(|c| Cmd::Type(c.to_string())).collect()
}

#[test]
fn typing_closes_brackets_and_steps_over() {
    let mut d = doc("", Language::JavaScript);
    run(&mut d, &typed("f(x"));
    assert_eq!(d.text(), "f(x)");
    run(&mut d, &typed(")"));
    assert_eq!(d.text(), "f(x)");
    assert_eq!(d.cursor().index, 4);
    // Backspace between a pair deletes both.
    run(&mut d, &typed(" ["));
    assert_eq!(d.text(), "f(x) []");
    run(&mut d, &[Cmd::Backspace]);
    assert_eq!(d.text(), "f(x) ");
}

#[test]
fn enter_indents_and_splits_braces() {
    let mut d = doc("function f() {}", Language::JavaScript);
    run(
        &mut d,
        &[
            Cmd::Move(Move::LineEnd, false),
            Cmd::Move(Move::CharLeft, false),
            Cmd::Newline,
        ],
    );
    assert_eq!(d.text(), "function f() {\n  \n}");
    assert_eq!((d.cursor().line, d.cursor().index), (1, 2));
    run(&mut d, &typed("x"));
    run(&mut d, &[Cmd::Newline]);
    assert_eq!(d.text(), "function f() {\n  x\n  \n}");
    // A closer on a blank line dedents.
    run(&mut d, &[Cmd::Move(Move::DocEnd, false)]);
    assert_eq!(d.cursor().line, 3);
}

#[test]
fn undo_groups_typing_and_restores_selection() {
    let mut d = doc("abc", Language::Text);
    run(&mut d, &[Cmd::Move(Move::LineEnd, false)]);
    run(&mut d, &typed("def"));
    assert_eq!(d.text(), "abcdef");
    run(&mut d, &[Cmd::Backspace]);
    assert_eq!(d.text(), "abcde");
    assert!(d.undo());
    assert_eq!(d.text(), "abcdef");
    assert!(d.undo());
    assert_eq!(d.text(), "abc");
    assert!(d.redo());
    assert_eq!(d.text(), "abcdef");
    // A move breaks the group.
    run(&mut d, &[Cmd::Move(Move::CharLeft, false)]);
    run(&mut d, &typed("X"));
    run(&mut d, &[Cmd::Move(Move::LineEnd, false)]);
    run(&mut d, &typed("Y"));
    assert!(d.undo());
    assert_eq!(d.text(), "abcdeXf");
}

#[test]
fn moves_follow_codemirror() {
    let mut d = doc("  let a = 1\nb", Language::JavaScript);
    // Home: indentation first, then the line start.
    run(
        &mut d,
        &[
            Cmd::Move(Move::LineEnd, false),
            Cmd::Move(Move::LineStart, false),
        ],
    );
    assert_eq!(d.cursor().index, 2);
    run(&mut d, &[Cmd::Move(Move::LineStart, false)]);
    assert_eq!(d.cursor().index, 0);
    // Ctrl+Right: spaces, then a group.
    run(&mut d, &[Cmd::Move(Move::GroupRight, false)]);
    assert_eq!(d.cursor().index, 5);
    // Shift+Ctrl+Right extends; Left collapses to the start.
    run(&mut d, &[Cmd::Move(Move::GroupRight, true)]);
    assert_eq!(d.selected_text().as_deref(), Some(" a"));
    run(&mut d, &[Cmd::Move(Move::CharLeft, false)]);
    assert_eq!(d.cursor().index, 5);
    assert!(d.selection_bounds().is_none());
    // Up on the first line goes to its start; Down on the last to its end.
    run(&mut d, &[Cmd::Move(Move::LineUp, false)]);
    assert_eq!((d.cursor().line, d.cursor().index), (0, 0));
    run(
        &mut d,
        &[
            Cmd::Move(Move::LineDown, false),
            Cmd::Move(Move::LineDown, false),
        ],
    );
    assert_eq!((d.cursor().line, d.cursor().index), (1, 1));
}

#[test]
fn tab_indents_lines_and_comments_toggle() {
    let mut d = doc("a\nb", Language::JavaScript);
    run(&mut d, &[Cmd::SelectAll, Cmd::IndentMore]);
    assert_eq!(d.text(), "  a\n  b");
    run(&mut d, &[Cmd::IndentLess]);
    assert_eq!(d.text(), "a\nb");
    run(&mut d, &[Cmd::ToggleComment]);
    assert_eq!(d.text(), "// a\n// b");
    run(&mut d, &[Cmd::ToggleComment]);
    assert_eq!(d.text(), "a\nb");
    run(
        &mut d,
        &[Cmd::Move(Move::DocStart, false), Cmd::MoveLine(false)],
    );
    assert_eq!(d.text(), "b\na");
    run(&mut d, &[Cmd::DeleteLine]);
    assert_eq!(d.text(), "b");
}

#[test]
fn linewise_copy_pastes_above() {
    let mut d = doc("one\ntwo", Language::Text);
    let mut fs = font_system().write().unwrap();
    let copied = d.copy(fs.raw(), false).unwrap();
    assert_eq!(copied, "one\n");
    d.run(fs.raw(), &Cmd::Move(Move::LineDown, false), 100.0);
    d.run(fs.raw(), &Cmd::Paste(copied), 100.0);
    assert_eq!(d.text(), "one\none\ntwo");
}

/// The classes of each token of the Showcase sample, in JavaScript.
#[test]
fn sample_highlights_like_codemirror() {
    let src =
        "function greet(name) {\n  // say hello\n  return `Hello, ${name}!`\n}\n\ngreet('world')";
    let d = doc(src, Language::JavaScript);
    let p = bato_theme::palette(
        bato_theme::ThemeId::Dark,
        bato_theme::Rgb::new(124, 58, 237),
    );
    let buf = d.buffer().unwrap();
    let class_at = |line: usize, at: usize| {
        let attrs = buf.lines[line].attrs_list().get_span(at);
        let col = attrs.color_opt.unwrap();
        Class::ALL_FOR_TESTS
            .iter()
            .copied()
            .find(|c| {
                let rgb = c.color(&p);
                (col.r(), col.g(), col.b()) == (rgb.r, rgb.g, rgb.b)
            })
            .unwrap_or(Class::Default)
    };
    let tok = |line: usize, s: &str| {
        let l = src.lines().nth(line).unwrap();
        class_at(line, l.find(s).unwrap())
    };
    assert_eq!(tok(0, "function"), Class::Keyword);
    assert_eq!(tok(0, "greet"), Class::Keyword); // accent-bright (function)
    assert_eq!(tok(0, "("), Class::Punctuation);
    assert_eq!(tok(0, "name"), Class::Default);
    assert_eq!(tok(0, "{"), Class::Punctuation);
    assert_eq!(tok(1, "//"), Class::Comment);
    assert_eq!(tok(1, "say"), Class::Comment);
    assert_eq!(tok(2, "return"), Class::Keyword);
    assert_eq!(tok(2, "`"), Class::String);
    assert_eq!(tok(2, "Hello"), Class::String);
    assert_eq!(tok(2, "${"), Class::Punctuation);
    assert_eq!(tok(2, "name}"), Class::Default);
    assert_eq!(tok(2, "!"), Class::String);
    assert_eq!(tok(5, "greet"), Class::Keyword);
    assert_eq!(tok(5, "'"), Class::String);
    assert_eq!(tok(5, "world"), Class::String);
    let _ = syntax::THEMES;
}

#[test]
fn every_language_has_a_syntax() {
    let ss = &syntax::system().syntax_set;
    for l in Language::ALL {
        let found = ss.find_syntax_by_extension(l.extension());
        assert!(found.is_some() || l == Language::Text, "{l:?}");
        eprintln!("{l:?} -> {:?}", found.map(|s| &s.name));
    }
}

#[test]
#[ignore]
fn dump_scopes() {
    use syntect::parsing::{ParseState, ScopeStack};
    let ss = &syntax::system().syntax_set;
    let ext = std::env::var("EXT").unwrap_or("jsx".into());
    let src = std::env::var("SRC").unwrap_or(
        "function greet(name) {\n  // say hello\n  return `Hello, ${name}!`\n}\n\ngreet('world')"
            .into(),
    );
    let syn = match std::env::var("NAME") {
        Ok(n) => ss.find_syntax_by_name(&n).unwrap(),
        Err(_) => ss.find_syntax_by_extension(&ext).unwrap(),
    };
    for e in ["js", "jsx", "ts", "tsx", "mjs"] {
        eprintln!(
            "ext {e} -> {:?}",
            ss.find_syntax_by_extension(e).map(|s| &s.name)
        );
    }
    eprintln!("syntax: {}", syn.name);
    let mut ps = ParseState::new(syn);
    let mut stack = ScopeStack::new();
    for line in src.lines() {
        let ops = ps.parse_line(line, ss).unwrap();
        let mut last = 0;
        for (i, op) in ops {
            if i > last {
                eprintln!(
                    "{:?} => {}",
                    &line[last..i],
                    stack
                        .as_slice()
                        .iter()
                        .map(|s| s.build_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            last = i;
            stack.apply(&op).unwrap();
        }
        if last < line.len() {
            eprintln!(
                "{:?} => {}",
                &line[last..],
                stack
                    .as_slice()
                    .iter()
                    .map(|s| s.build_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
}

#[test]
fn vim_modes_and_motions() {
    crate::fonts::setup();
    let mut d = Doc::new("  return `Hello, x`\nnext");
    let mut fs = font_system().write().unwrap();
    let p = bato_theme::palette(
        bato_theme::ThemeId::Dark,
        bato_theme::Rgb::new(124, 58, 237),
    );
    d.configure(fs.raw(), &p, Language::JavaScript, true);
    d.refresh(fs.raw());
    use iced::advanced::graphics::text::cosmic_text as ct;
    let key = |d: &mut Doc, fs: &mut ct::FontSystem, s: &str| {
        for c in s.chars() {
            d.vi_key(fs, ct::Action::Insert(c));
        }
    };
    d.pointer(fs.raw(), super::engine::Pointer::Click, 16.0, 5.0);
    assert_eq!(d.vi_mode(), Some(ct::ViMode::Normal));
    key(&mut d, fs.raw(), "dw");
    assert_eq!(d.text(), "  `Hello, x`\nnext");
    key(&mut d, fs.raw(), "iyield ");
    assert_eq!(d.vi_mode(), Some(ct::ViMode::Insert));
    assert_eq!(d.text(), "  yield `Hello, x`\nnext");
    d.vi_key(fs.raw(), ct::Action::Escape);
    assert_eq!(d.vi_mode(), Some(ct::ViMode::Normal));
    key(&mut d, fs.raw(), "ve");
    assert_eq!(d.vi_mode(), Some(ct::ViMode::Visual));
    eprintln!(
        "cursor {:?} sel {:?} bounds {:?}",
        d.cursor(),
        d.selection_bounds(),
        d.selected_text()
    );
    key(&mut d, fs.raw(), "2");
    assert_eq!(d.vi_pending(), "2");
    d.vi_key(fs.raw(), ct::Action::Escape);
    key(&mut d, fs.raw(), "u");
    eprintln!("after u: {:?}", d.text());
}

/// `cargo test -p bato-kit --features editor load_cost -- --ignored --nocapture`
/// (dependencies are built at opt-level 2 in dev).
#[test]
#[ignore]
fn load_cost() {
    let t = std::time::Instant::now();
    let _ = syntax::system();
    eprintln!("syntax set: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let _ = doc(
        "function f() {}\n".repeat(200).as_str(),
        Language::TypeScript,
    );
    eprintln!("200 TS lines highlighted + shaped: {:?}", t.elapsed());
}
