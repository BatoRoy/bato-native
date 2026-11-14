//! Syntax highlighting: the languages of `CodeEditor.tsx`, the colour rules
//! of `lib/cmTheme.ts` mapped onto syntect scopes, and the lazily loaded
//! syntax set (two-face's extra set: syntect's defaults plus TypeScript and
//! TSX).
//!
//! cosmic-text's `SyntaxEditor` takes its colours from a syntect theme that
//! lives as long as the syntax set, but the app's palette changes at run
//! time (theme, any accent). So the theme does not hold colours: each rule
//! writes a *token class* as a sentinel colour ([`sentinel`]), and the
//! editor swaps sentinels for the palette's colours right after it
//! highlights a line ([`resolve`]). A palette change re-highlights through
//! the second, identical theme ([`THEMES`]).

use std::str::FromStr;
use std::sync::OnceLock;

use iced::advanced::graphics::text::cosmic_text as ct;
use syntect::highlighting::{
    Color as SColor, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSet,
    ThemeSettings,
};

use bato_theme::{Palette, Rgb};

/// `CodeLanguage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    JavaScript,
    TypeScript,
    Jsx,
    Tsx,
    Json,
    Html,
    Css,
    Python,
    Markdown,
    #[default]
    Text,
}

impl Language {
    pub const ALL: [Language; 10] = [
        Language::JavaScript,
        Language::TypeScript,
        Language::Jsx,
        Language::Tsx,
        Language::Json,
        Language::Html,
        Language::Css,
        Language::Python,
        Language::Markdown,
        Language::Text,
    ];

    /// The `CodeLanguage` string.
    pub fn id(self) -> &'static str {
        match self {
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Jsx => "jsx",
            Language::Tsx => "tsx",
            Language::Json => "json",
            Language::Html => "html",
            Language::Css => "css",
            Language::Python => "python",
            Language::Markdown => "markdown",
            Language::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.id() == s)
    }

    /// The extension cosmic-text's `SyntaxEditor` looks the syntax up by.
    pub(crate) fn extension(self) -> &'static str {
        match self {
            // React's `javascript({ jsx: true })`: Babel's grammar does JSX.
            Language::JavaScript | Language::Jsx => "jsx",
            Language::TypeScript => "ts",
            Language::Tsx => "tsx",
            Language::Json => "json",
            Language::Html => "html",
            Language::Css => "css",
            Language::Python => "py",
            Language::Markdown => "md",
            Language::Text => "txt",
        }
    }

    /// Lines after which Enter indents one more unit (besides an open
    /// bracket).
    pub(crate) fn colon_indents(self) -> bool {
        self == Language::Python
    }
}

/// The token classes of `cmTheme.ts` (one per colour rule, plus the
/// default foreground).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Class {
    Default = 0,
    Comment,
    Keyword,
    String,
    Number,
    Function,
    Type,
    Tag,
    Attribute,
    Punctuation,
    Heading,
    Link,
}

impl Class {
    #[cfg(test)]
    pub(crate) const ALL_FOR_TESTS: [Class; 12] = Self::ALL;
    const ALL: [Class; 12] = [
        Class::Default,
        Class::Comment,
        Class::Keyword,
        Class::String,
        Class::Number,
        Class::Function,
        Class::Type,
        Class::Tag,
        Class::Attribute,
        Class::Punctuation,
        Class::Heading,
        Class::Link,
    ];

    /// The palette colour (`cmTheme.ts` `styles`).
    pub fn color(self, p: &Palette) -> Rgb {
        match self {
            Class::Default => p.text,
            Class::Comment => p.muted,
            Class::Keyword | Class::Function | Class::Heading | Class::Link => p.accent_bright,
            Class::String | Class::Attribute => p.green,
            Class::Number | Class::Type => p.yellow,
            Class::Tag => p.red,
            Class::Punctuation => p.subtext,
        }
    }
}

const TAG_G: u8 = 0x5b;
const TAG_B: u8 = 0xa7;
const TAG_A: u8 = 0x01;

/// The sentinel colour that stands for a class in the syntect theme.
pub fn sentinel(class: Class) -> ct::Color {
    ct::Color::rgba(class as u8, TAG_G, TAG_B, TAG_A)
}

/// The class a sentinel stands for (`None`: a real colour).
pub fn class_of(c: ct::Color) -> Option<Class> {
    (c.a() == TAG_A && c.g() == TAG_G && c.b() == TAG_B)
        .then(|| Class::ALL.get(c.r() as usize).copied())
        .flatten()
}

/// The scope rules. syntect picks the most specific matching selector, so
/// the narrower rules undo the broad ones where CodeMirror's Lezer tags
/// differ from the Sublime scopes (quotes belong to the string, operators
/// and CSS units are plain text, JSON keys are property names, the inside
/// of a template interpolation is code).
const RULES: &[(&str, Class)] = &[
    ("comment", Class::Comment),
    ("comment punctuation", Class::Comment),
    (
        "keyword, storage.type, storage.modifier, keyword.operator.word, \
         keyword.operator.new, keyword.operator.expression, \
         keyword.operator.logical.python, variable.language.this, \
         constant.language.import-export-all",
        Class::Keyword,
    ),
    ("keyword.operator, keyword.other.unit", Class::Default),
    ("variable.language", Class::Default),
    (
        "string, string punctuation.definition.string, \
         string punctuation.definition.string.begin, \
         string punctuation.definition.string.end, \
         string punctuation.definition.template, \
         string punctuation.quasi, string keyword.other.template, \
         storage.type.string",
        Class::String,
    ),
    // Babel's `${` / `}` in a template literal.
    ("keyword.other.substitution", Class::Punctuation),
    ("constant.numeric, constant.language", Class::Number),
    (
        "entity.name.function, variable.function, support.function, \
         meta.function-call entity.name.function, \
         meta.function-call variable.function",
        Class::Function,
    ),
    (
        "entity.name.type, entity.name.class, support.type.primitive, \
         entity.name.namespace, entity.other.attribute-name.class.css",
        Class::Type,
    ),
    ("support.type.property-name", Class::Default),
    ("entity.name.tag", Class::Tag),
    ("entity.other.attribute-name", Class::Attribute),
    ("entity.other.attribute-name.id.css", Class::Default),
    ("punctuation, meta.brace", Class::Punctuation),
    (
        "markup.heading, entity.name.section, markup.heading punctuation",
        Class::Heading,
    ),
    (
        "markup.underline.link, string.other.link, meta.link, markup.link",
        Class::Link,
    ),
    // JSON keys are property names (text), with their quotes.
    (
        "meta.mapping.key string, meta.mapping.key punctuation.definition.string, \
         meta.structure.dictionary.key string, \
         meta.structure.dictionary.key punctuation.definition.string, \
         string.json support.type.property-name",
        Class::Default,
    ),
    // Inside `${…}`: code again.
    (
        "meta.interpolation, meta.template.expression, source.js.embedded, \
         meta.embedded.line",
        Class::Default,
    ),
];

fn theme(name: &str) -> Theme {
    let fg = |class: Class| {
        let c = sentinel(class);
        SColor {
            r: c.r(),
            g: c.g(),
            b: c.b(),
            a: c.a(),
        }
    };
    let mut scopes = Vec::new();
    for (sel, class) in RULES {
        scopes.push(ThemeItem {
            scope: ScopeSelectors::from_str(sel).expect("scope selector"),
            style: StyleModifier {
                foreground: Some(fg(*class)),
                background: None,
                font_style: Some(if *class == Class::Comment {
                    FontStyle::ITALIC
                } else {
                    FontStyle::empty()
                }),
            },
        });
    }
    Theme {
        name: Some(name.to_string()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(fg(Class::Default)),
            ..ThemeSettings::default()
        },
        scopes,
    }
}

/// The two (identical) themes: switching between them makes
/// `SyntaxEditor` drop its highlight cache.
pub const THEMES: [&str; 2] = ["bato-a", "bato-b"];

/// The syntax set and the class themes, loaded on first use (the first
/// editor that lays out), never at start-up.
pub fn system() -> &'static ct::SyntaxSystem {
    static SYSTEM: OnceLock<ct::SyntaxSystem> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let mut theme_set = ThemeSet::default();
        for name in THEMES {
            theme_set.themes.insert(name.to_string(), theme(name));
        }
        ct::SyntaxSystem {
            syntax_set: two_face::syntax::extra_no_newlines(),
            theme_set,
        }
    })
}

/// Swap the sentinel colours of `buffer`'s lines for `palette`'s. Returns
/// whether any line changed (it then needs shaping again).
pub fn resolve(buffer: &mut ct::Buffer, palette: &Palette) -> bool {
    let mut any = false;
    for line in buffer.lines.iter_mut() {
        let list = line.attrs_list();
        let defaults = list.defaults();
        let needs = defaults.color_opt.and_then(class_of).is_some()
            || list
                .spans_iter()
                .any(|(_, a)| a.color_opt.and_then(class_of).is_some());
        if !needs {
            continue;
        }
        let real = |a: ct::Attrs<'_>| -> ct::AttrsOwned {
            let class = a.color_opt.and_then(class_of);
            let mut a = a.clone();
            if let Some(class) = class {
                let rgb = class.color(palette);
                a.color_opt = Some(ct::Color::rgb(rgb.r, rgb.g, rgb.b));
            }
            // `fontStyle: italic` on a monospace font without an italic
            // face: Chromium (Skia) slants the upright one.
            if a.style == ct::Style::Italic {
                a.style = ct::Style::Normal;
                a.cache_key_flags |= ct::CacheKeyFlags::FAKE_ITALIC;
            }
            ct::AttrsOwned::new(&a)
        };
        let mut out = ct::AttrsList::new(&real(defaults).as_attrs());
        for (range, a) in list.spans_iter() {
            out.add_span(range.clone(), &real(a.as_attrs()).as_attrs());
        }
        line.set_attrs_list(out);
        any = true;
    }
    any
}
