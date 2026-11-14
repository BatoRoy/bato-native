//! Pure text logic of CodeMirror's commands (`@codemirror/commands`,
//! `closeBrackets`, `bracketMatching`): character groups, indentation,
//! bracket pairs and line comments. Positions are byte offsets in a line.

use super::syntax::Language;

/// `indentUnit` (two spaces).
pub const INDENT_UNIT: usize = 2;
/// `tabSize`.
pub const TAB_SIZE: usize = 4;

/// CodeMirror's `CharCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cat {
    Word,
    Space,
    Other,
}

pub fn cat(c: char) -> Cat {
    if c.is_alphanumeric() || c == '_' || c == '$' {
        Cat::Word
    } else if c.is_whitespace() {
        Cat::Space
    } else {
        Cat::Other
    }
}

/// `moveByGroup` within a line: from `at`, skip spaces, then one group of
/// the same category. `None` at the line edge (the caller moves to the
/// neighbouring line).
pub fn group_end(line: &str, at: usize, forward: bool) -> Option<usize> {
    let mut pos = at;
    let mut group: Option<Cat> = None;
    let mut moved = false;
    loop {
        let next = if forward {
            line[pos..].chars().next()
        } else {
            line[..pos].chars().next_back()
        };
        let Some(ch) = next else { break };
        let c = cat(ch);
        match group {
            Some(g) if g != c => break,
            None if c != Cat::Space => group = Some(c),
            _ => {}
        }
        moved = true;
        pos = if forward {
            pos + ch.len_utf8()
        } else {
            pos - ch.len_utf8()
        };
    }
    moved.then_some(pos)
}

/// The previous / next character boundary (`None` at the edge).
pub fn char_step(line: &str, at: usize, forward: bool) -> Option<usize> {
    if forward {
        line[at..].chars().next().map(|c| at + c.len_utf8())
    } else {
        line[..at].chars().next_back().map(|c| at - c.len_utf8())
    }
}

/// Byte length of the leading whitespace.
pub fn indent_len(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// The visual column of byte `at` (tabs to `TAB_SIZE`).
pub fn column(line: &str, at: usize) -> usize {
    let mut col = 0;
    for c in line[..at].chars() {
        col = if c == '\t' {
            col + TAB_SIZE - col % TAB_SIZE
        } else {
            col + 1
        };
    }
    col
}

/// The indentation string for a column.
pub fn indent_string(col: usize) -> String {
    " ".repeat(col)
}

fn opener(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn closer(c: char) -> Option<char> {
    match c {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// `insertNewlineAndIndent`: the indentation of the new line after `before`
/// (the text left of the cursor), and whether the text right of the cursor
/// starts with the matching closer (it then goes on its own line, back at
/// the old indentation).
pub fn newline_indent(lang: Language, line: &str, at: usize) -> (usize, bool) {
    let base = column(line, indent_len(line).min(at));
    let before = line[..at].trim_end();
    let after = line[at..].trim_start();
    let last = before.chars().next_back();
    let opens = last.and_then(opener);
    let deeper = opens.is_some() || (lang.colon_indents() && last == Some(':'));
    let split = opens.is_some_and(|close| after.starts_with(close));
    (if deeper { base + INDENT_UNIT } else { base }, split)
}

/// `closeBrackets`: the closer to insert after a typed `c`, if any. `next`
/// is the character after the cursor, `prev` the one before.
pub fn auto_close(c: char, prev: Option<char>, next: Option<char>) -> Option<char> {
    // closeBrackets' `before`: ")]}:;>" and whitespace (or the line end).
    let free = next.is_none_or(|n| n.is_whitespace() || ")]}:;>".contains(n));
    if let Some(close) = opener(c) {
        return free.then_some(close);
    }
    if matches!(c, '"' | '\'' | '`') {
        // Not right after a word character (an apostrophe in `don't`).
        let after_word = prev.is_some_and(|p| cat(p) == Cat::Word);
        return (free && !after_word).then_some(c);
    }
    None
}

/// Typing `c` with `next` after the cursor just steps over it.
pub fn steps_over(c: char, next: Option<char>) -> bool {
    next == Some(c) && (closer(c).is_some() || matches!(c, '"' | '\'' | '`'))
}

/// Backspace between an auto-closed pair deletes both.
pub fn is_pair(prev: Option<char>, next: Option<char>) -> bool {
    match (prev, next) {
        (Some(p), Some(n)) => opener(p) == Some(n) || (p == n && matches!(p, '"' | '\'' | '`')),
        _ => false,
    }
}

/// `deleteCharBackward` in leading whitespace: the byte to delete back to
/// (the previous indent unit column). `None` outside leading whitespace.
pub fn backspace_indent(line: &str, at: usize) -> Option<usize> {
    if at == 0 || at > indent_len(line) || line[..at].ends_with('\t') {
        return None;
    }
    let col = column(line, at);
    let target = (col - 1) / INDENT_UNIT * INDENT_UNIT;
    // Walk back over spaces to the target column.
    let mut pos = at;
    while pos > 0 && column(line, pos) > target && line.as_bytes()[pos - 1] == b' ' {
        pos -= 1;
    }
    Some(pos)
}

/// The matching bracket for a bracket at `(line, at)`: scans forward for
/// an opener, backward for a closer, across lines (CodeMirror's
/// `bracketMatching` without the syntax tree; `maxScanDistance` 10 000).
pub fn matching_bracket(lines: &[&str], line: usize, at: usize) -> Option<(usize, usize)> {
    let c = lines.get(line)?[at..].chars().next()?;
    let (forward, want, me) = match (opener(c), closer(c)) {
        (Some(close), _) => (true, close, c),
        (_, Some(open)) => (false, open, c),
        _ => return None,
    };
    let mut depth = 0usize;
    let mut scanned = 0usize;
    let mut l = line;
    let mut pos = if forward { at + c.len_utf8() } else { at };
    loop {
        let text = lines[l];
        if forward {
            for (i, ch) in text[pos..].char_indices() {
                scanned += 1;
                if ch == me {
                    depth += 1;
                } else if ch == want {
                    if depth == 0 {
                        return Some((l, pos + i));
                    }
                    depth -= 1;
                }
            }
            l += 1;
            if l >= lines.len() || scanned > 10_000 {
                return None;
            }
            pos = 0;
        } else {
            for (i, ch) in text[..pos].char_indices().rev() {
                scanned += 1;
                if ch == me {
                    depth += 1;
                } else if ch == want {
                    if depth == 0 {
                        return Some((l, i));
                    }
                    depth -= 1;
                }
            }
            if l == 0 || scanned > 10_000 {
                return None;
            }
            l -= 1;
            pos = lines[l].len();
        }
    }
}

/// The line-comment token (`toggleComment`); languages without one use a
/// block comment around the line.
pub fn comment_tokens(lang: Language) -> Option<(&'static str, &'static str)> {
    match lang {
        Language::JavaScript | Language::TypeScript | Language::Jsx | Language::Tsx => {
            Some(("//", ""))
        }
        Language::Python => Some(("#", "")),
        Language::Css => Some(("/*", "*/")),
        Language::Html | Language::Markdown => Some(("<!--", "-->")),
        Language::Json | Language::Text => None,
    }
}

/// `toggleLineComment` over `lines`: comment them all unless every
/// non-blank one is commented already (then uncomment). Returns the new
/// lines.
pub fn toggle_comment(lines: &[&str], open: &str, close: &str) -> Vec<String> {
    let blank = |l: &str| l.trim().is_empty();
    let commented = |l: &str| {
        let t = l.trim_start();
        t.starts_with(open) && (close.is_empty() || t.trim_end().ends_with(close))
    };
    let all =
        lines.iter().filter(|l| !blank(l)).all(|l| commented(l)) && lines.iter().any(|l| !blank(l));
    // Comment at the smallest indentation of the block (CodeMirror does).
    let min_indent = lines
        .iter()
        .filter(|l| !blank(l))
        .map(|l| indent_len(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if blank(l) {
                return l.to_string();
            }
            if all {
                let ind = indent_len(l);
                let mut rest = &l[ind + open.len()..];
                rest = rest.strip_prefix(' ').unwrap_or(rest);
                if !close.is_empty() {
                    rest = rest.trim_end();
                    rest = rest.strip_suffix(close).unwrap_or(rest);
                    rest = rest.strip_suffix(' ').unwrap_or(rest);
                }
                format!("{}{}", &l[..ind], rest)
            } else if close.is_empty() {
                format!("{}{} {}", &l[..min_indent], open, &l[min_indent..])
            } else {
                format!(
                    "{}{} {} {}",
                    &l[..min_indent],
                    open,
                    &l[min_indent..],
                    close
                )
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_skip_space_then_one_category() {
        let l = "function greet(name) {";
        assert_eq!(group_end(l, 0, true), Some(8));
        assert_eq!(group_end(l, 8, true), Some(14)); // " greet"
        assert_eq!(group_end(l, 14, true), Some(15)); // "("
        assert_eq!(group_end(l, 14, false), Some(9)); // back over "greet"
        assert_eq!(group_end(l, 9, false), Some(0)); // " " then "function"
        assert_eq!(group_end(l, l.len(), true), None);
        assert_eq!(group_end("  ", 0, true), Some(2));
        assert_eq!(group_end("a == b", 1, true), Some(4));
    }

    #[test]
    fn enter_indents_after_openers() {
        let js = Language::JavaScript;
        assert_eq!(newline_indent(js, "function f() {", 14), (2, false));
        assert_eq!(newline_indent(js, "  if (x) {}", 10), (4, true));
        assert_eq!(newline_indent(js, "  return 1", 10), (2, false));
        assert_eq!(newline_indent(Language::Python, "def f():", 8), (2, false));
        assert_eq!(newline_indent(js, "    ", 2), (2, false));
    }

    #[test]
    fn close_brackets_rules() {
        assert_eq!(auto_close('(', None, None), Some(')'));
        assert_eq!(auto_close('(', None, Some('x')), None);
        assert_eq!(auto_close('{', Some('a'), Some(' ')), Some('}'));
        assert_eq!(auto_close('\'', Some('n'), None), None);
        assert_eq!(auto_close('"', Some(' '), Some(')')), Some('"'));
        assert!(steps_over(')', Some(')')));
        assert!(!steps_over('a', Some('a')));
        assert!(is_pair(Some('('), Some(')')));
        assert!(is_pair(Some('`'), Some('`')));
        assert!(!is_pair(Some('('), Some(']')));
    }

    #[test]
    fn backspace_in_indentation_deletes_a_unit() {
        assert_eq!(backspace_indent("    x", 4), Some(2));
        assert_eq!(backspace_indent("   x", 3), Some(2));
        assert_eq!(backspace_indent("  x", 2), Some(0));
        assert_eq!(backspace_indent("  x", 3), None);
        assert_eq!(backspace_indent("x", 0), None);
    }

    #[test]
    fn brackets_match_across_lines() {
        let lines = ["function greet(name) {", "  return `${name}`", "}"];
        assert_eq!(matching_bracket(&lines, 0, 14), Some((0, 19)));
        assert_eq!(matching_bracket(&lines, 0, 21), Some((2, 0)));
        assert_eq!(matching_bracket(&lines, 2, 0), Some((0, 21)));
        assert_eq!(matching_bracket(&lines, 1, 11), Some((1, 16)));
        assert_eq!(matching_bracket(&lines, 0, 0), None);
    }

    #[test]
    fn comments_toggle() {
        let on = toggle_comment(&["  a", "", "    b"], "//", "");
        assert_eq!(on, ["  // a", "", "  //   b"]);
        let off = toggle_comment(&["  // a", "", "  //   b"], "//", "");
        assert_eq!(off, ["  a", "", "    b"]);
        let css = toggle_comment(&["a {}"], "/*", "*/");
        assert_eq!(css, ["/* a {} */"]);
        assert_eq!(toggle_comment(&["/* a {} */"], "/*", "*/"), ["a {}"]);
    }

    #[test]
    fn columns_expand_tabs() {
        assert_eq!(column("\tx", 1), 4);
        assert_eq!(column("ab\tx", 3), 4);
        assert_eq!(column("  x", 2), 2);
    }
}
