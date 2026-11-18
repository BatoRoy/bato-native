//! Bundled fonts: Inter 4.001 (wght axis, opsz pinned to 14; upright and
//! italic) and JetBrains Mono 400/500/600/700 (upright and italic), compiled
//! in from `assets/fonts`. Call [`setup`] once before
//! the first frame (the frame's `run` helper does).
//!
//! The family names are pinned: a machine may also have a system `Inter`
//! (static instances) and `JetBrains Mono` variants, and fontdb would happily
//! resolve "Inter" weight 600 to the system SemiBold. So every system face of
//! those families is taken out of the "Inter" family before anything is
//! shaped.
//!
//! Inter is fontsource's split, as Electron loads it: the Latin subset is the
//! `Inter` face, the other six `unicode-range` subsets are fallback families
//! tried in Chromium's order, then the system's static Inter faces (renamed
//! to [`SYSTEM_INTER`], so a weight request never resolves to them), the next
//! family of the CSS stack. Only after those come cosmic-text's script and
//! common fallbacks.

use std::borrow::Cow;
use std::collections::HashSet;

use iced::advanced::graphics::text::cosmic_text::fontdb;
use iced::advanced::graphics::text::font_system;

pub const INTER: &[u8] = include_bytes!("../../../assets/fonts/InterVariable-latin.ttf");
/// The other fontsource Inter 4.001 subsets (`woff2_decompress`ed), in the
/// order Chromium tries them: the last `@font-face` of fontsource's CSS whose
/// `unicode-range` covers a character wins.
pub const INTER_SUBSETS: [(&str, &[u8]); 6] = [
    (
        "Inter latin-ext",
        include_bytes!("../../../assets/fonts/InterVariable-latin-ext.ttf"),
    ),
    (
        "Inter vietnamese",
        include_bytes!("../../../assets/fonts/InterVariable-vietnamese.ttf"),
    ),
    (
        "Inter greek",
        include_bytes!("../../../assets/fonts/InterVariable-greek.ttf"),
    ),
    (
        "Inter greek-ext",
        include_bytes!("../../../assets/fonts/InterVariable-greek-ext.ttf"),
    ),
    (
        "Inter cyrillic",
        include_bytes!("../../../assets/fonts/InterVariable-cyrillic.ttf"),
    ),
    (
        "Inter cyrillic-ext",
        include_bytes!("../../../assets/fonts/InterVariable-cyrillic-ext.ttf"),
    ),
];
/// Inter Italic (fontsource `wght-italic`): the Latin subset is the `Inter`
/// italic face, Latin Extended joins the `Inter latin-ext` family. The other
/// subsets have no italic here: their upright faces stand in (registered as
/// italic too), so an italic run never leaves the Inter families.
pub const INTER_ITALIC: &[u8] =
    include_bytes!("../../../assets/fonts/InterVariable-Italic-latin.ttf");
pub const INTER_ITALIC_LATIN_EXT: &[u8] =
    include_bytes!("../../../assets/fonts/InterVariable-Italic-latin-ext.ttf");
/// JetBrains Mono's other static faces (fontsource `latin`): italic 400/500,
/// SemiBold and Bold, upright and italic.
pub const JBM_FACES: [&[u8]; 6] = [
    include_bytes!("../../../assets/fonts/JetBrainsMono-Italic-latin.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-MediumItalic-latin.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-SemiBold-latin.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-SemiBoldItalic-latin.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Bold-latin.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-BoldItalic-latin.ttf"),
];
pub const JBM_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular-latin.ttf");
pub const JBM_MEDIUM: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Medium-latin.ttf");

pub const SANS: &str = "Inter";
pub const MONO: &str = "JetBrains Mono";
/// The system's own `Inter` faces, renamed: fallback only, after the bundled
/// subsets (Electron's stack is `"Inter Variable", Inter, …`).
pub const SYSTEM_INTER: &str = "Inter (system)";

/// hhea ascent / descent in em (spec §2.1), for the CSS line-box model.
pub const INTER_ASC: f32 = 1984.0 / 2048.0;
pub const INTER_DESC: f32 = 494.0 / 2048.0;
pub const JBM_ASC: f32 = 1020.0 / 1000.0;
pub const JBM_DESC: f32 = 300.0 / 1000.0;

/// Load the bundled fonts into iced's font system. Idempotent.
pub fn setup() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(setup_once);
}

fn setup_once() {
    let mut fs = font_system().write().expect("font system");
    // Take system faces that share our family names out of those families:
    // static `Inter` faces become the fallback-only SYSTEM_INTER family, every
    // other `Inter …` / `JetBrains Mono …` face is dropped.
    {
        let db = fs.raw().db_mut();
        let shadowing: Vec<fontdb::FaceInfo> = db
            .faces()
            .filter(|f| !matches!(f.source, fontdb::Source::Binary(_)))
            .filter(|f| {
                f.families.iter().any(|(name, _)| {
                    name == "Inter"
                        || name.starts_with("Inter ")
                        || name.starts_with("JetBrains Mono")
                })
            })
            .cloned()
            .collect();
        for face in shadowing {
            db.remove_face(face.id);
            if let Some((_, lang)) = face.families.first().filter(|(n, _)| n == SANS) {
                let mut renamed = face.clone();
                renamed.families = vec![(SYSTEM_INTER.into(), *lang)];
                db.push_face_info(renamed);
            }
        }
    }
    fs.load_font(Cow::Borrowed(INTER));
    fs.load_font(Cow::Borrowed(JBM_REGULAR));
    fs.load_font(Cow::Borrowed(JBM_MEDIUM));
    fs.load_font(Cow::Borrowed(INTER_ITALIC));
    for bytes in JBM_FACES {
        fs.load_font(Cow::Borrowed(bytes));
    }
    let subsets = INTER_SUBSETS
        .iter()
        .copied()
        .chain([("Inter latin-ext", INTER_ITALIC_LATIN_EXT)]);
    for (family, bytes) in subsets {
        let before: HashSet<fontdb::ID> = fs.raw().db().faces().map(|f| f.id).collect();
        fs.load_font(Cow::Borrowed(bytes));
        let db = fs.raw().db_mut();
        let added: Vec<fontdb::FaceInfo> = db
            .faces()
            .filter(|f| !before.contains(&f.id))
            .cloned()
            .collect();
        for mut face in added {
            db.remove_face(face.id);
            let lang = face
                .families
                .first()
                .map_or(fontdb::Language::English_UnitedStates, |f| f.1);
            face.families = vec![(family.into(), lang)];
            db.push_face_info(face);
        }
    }
    // The subsets without an italic file: their upright faces also serve
    // italic runs (cosmic-text only matches faces of the requested style).
    {
        let db = fs.raw().db_mut();
        let has_italic = |db: &fontdb::Database, family: &str| {
            db.faces().any(|f| {
                f.style == fontdb::Style::Italic && f.families.iter().any(|(n, _)| n == family)
            })
        };
        for (family, _) in INTER_SUBSETS {
            if has_italic(db, family) {
                continue;
            }
            let upright: Vec<fontdb::FaceInfo> = db
                .faces()
                .filter(|f| {
                    matches!(f.source, fontdb::Source::Binary(_))
                        && f.style == fontdb::Style::Normal
                        && f.families.iter().any(|(n, _)| n == family)
                })
                .cloned()
                .collect();
            for mut face in upright {
                face.style = fontdb::Style::Italic;
                db.push_face_info(face);
            }
        }
    }
    let mut chain: Vec<String> = INTER_SUBSETS.iter().map(|(f, _)| f.to_string()).collect();
    chain.push(SYSTEM_INTER.into());
    fs.raw().set_family_fallbacks(SANS, chain);

    // fontdb registers a variable font once, at its default weight (400).
    // cosmic-text only accepts exact weight matches inside a family, so a
    // request for Inter 500 or 600 silently fell back to *another family*
    // (NotoSans-Medium, FreeSans Bold). Register each bundled Inter face once
    // per CSS weight; cosmic-text then applies `wght` = the requested weight
    // for both shaping (harfrust) and rasterization.
    let db = fs.raw().db_mut();
    let inter: Vec<fontdb::FaceInfo> = db
        .faces()
        .filter(|f| {
            matches!(f.source, fontdb::Source::Binary(_))
                && f.families
                    .iter()
                    .any(|(n, _)| n == SANS || INTER_SUBSETS.iter().any(|(s, _)| n == s))
        })
        .cloned()
        .collect();
    for face in inter {
        for w in (100..=900).step_by(100) {
            if w != face.weight.0 {
                let mut copy = face.clone();
                copy.weight = fontdb::Weight(w);
                db.push_face_info(copy);
            }
        }
    }
}

pub fn sans() -> iced::Font {
    iced::Font::with_name(SANS)
}

pub fn mono() -> iced::Font {
    iced::Font::with_name(MONO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::graphics::text::cosmic_text::{
        Attrs, Buffer, Family, Metrics, Shaping, Weight,
    };

    /// The family of the face each character of `text` shaped with.
    fn faces(text: &str, weight: u16) -> Vec<(char, String)> {
        setup();
        let mut guard = font_system().write().expect("font system");
        let fs = guard.raw();
        let mut buffer = Buffer::new(fs, Metrics::new(14.0, 20.0));
        let attrs = Attrs::new()
            .family(Family::Name(SANS))
            .weight(Weight(weight));
        buffer.set_text(fs, text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(fs, false);
        let mut out = Vec::new();
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                let face = fs.db().face(g.font_id).expect("face");
                let ch = text[g.start..g.end].chars().next().expect("char");
                out.push((ch, face.families[0].0.clone()));
            }
        }
        out
    }

    /// (family, italic, weight of the face) per character.
    fn styled(
        text: &str,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> Vec<(char, String, bool, u16)> {
        use iced::advanced::graphics::text::cosmic_text::Style;
        setup();
        let mut guard = font_system().write().expect("font system");
        let fs = guard.raw();
        let mut buffer = Buffer::new(fs, Metrics::new(14.0, 20.0));
        let mut attrs = Attrs::new()
            .family(Family::Name(family))
            .weight(Weight(weight));
        if italic {
            attrs = attrs.style(Style::Italic);
        }
        buffer.set_text(fs, text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(fs, false);
        let mut out = Vec::new();
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                let face = fs.db().face(g.font_id).expect("face");
                let ch = text[g.start..g.end].chars().next().expect("char");
                // The real face behind a copy: its PostScript name.
                let real_italic = face.post_script_name.contains("Italic");
                out.push((ch, face.families[0].0.clone(), real_italic, face.weight.0));
            }
        }
        out
    }

    #[test]
    fn italic_and_bold_faces() {
        // Inter italic: the real italic face, at the requested weight.
        for w in [400, 700] {
            let got = styled("Ag", SANS, w, true);
            assert!(got.iter().all(|(_, f, it, _)| f == SANS && *it), "{got:?}");
        }
        // Latin Extended italic is real too; Cyrillic stays in Inter (upright).
        let got = styled("ĂЖ", SANS, 400, true);
        assert_eq!(got[0].1, "Inter latin-ext");
        assert!(got[0].2, "{got:?}");
        assert_eq!(got[1].1, "Inter cyrillic");
        // Upright text is unchanged.
        let got = styled("Ag", SANS, 400, false);
        assert!(got.iter().all(|(_, f, it, _)| f == SANS && !*it), "{got:?}");
        // JetBrains Mono: every weight 400–700 its own face, italic or not.
        for (w, italic) in [
            (400, false),
            (500, false),
            (600, false),
            (700, false),
            (400, true),
            (500, true),
            (600, true),
            (700, true),
        ] {
            let got = styled("m", MONO, w, italic);
            assert_eq!(got[0].1, MONO, "{w} {italic}");
            assert_eq!(got[0].2, italic, "{w} {italic}");
            assert_eq!(got[0].3, w, "{w} {italic}");
        }
    }

    #[test]
    fn inter_subsets_cover_like_electron() {
        for weight in [400, 600] {
            let got = faces("Aé Ă Ж Ѣ Ω ἀ Ơ", weight);
            let family = |c: char| got.iter().find(|(g, _)| *g == c).map(|(_, f)| f.as_str());
            assert_eq!(family('A'), Some(SANS));
            assert_eq!(family('é'), Some(SANS));
            assert_eq!(family('Ă'), Some("Inter latin-ext"));
            assert_eq!(family('Ж'), Some("Inter cyrillic"));
            assert_eq!(family('Ѣ'), Some("Inter cyrillic-ext"));
            assert_eq!(family('Ω'), Some("Inter greek"));
            assert_eq!(family('ἀ'), Some("Inter greek-ext"));
            assert_eq!(family('Ơ'), Some("Inter latin-ext"));
        }
    }

    #[test]
    fn arrows_fall_back_to_system_inter_only_after_the_subsets() {
        setup();
        let has_system = {
            let mut guard = font_system().write().expect("font system");
            guard
                .raw()
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == SYSTEM_INTER))
        };
        let got = faces("a←", 500);
        assert_eq!(got[0].1, SANS);
        if has_system {
            assert_eq!(got[1].1, SYSTEM_INTER);
        } else {
            assert_ne!(got[1].1, SANS);
        }
        // A weight request never resolves to the system faces.
        let db_has_sans_system = {
            let mut guard = font_system().write().expect("font system");
            guard.raw().db().faces().any(|f| {
                !matches!(f.source, fontdb::Source::Binary(_))
                    && f.families.iter().any(|(n, _)| n == SANS)
            })
        };
        assert!(!db_has_sans_system);
    }
}
