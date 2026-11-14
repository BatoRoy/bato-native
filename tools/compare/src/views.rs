//! What can be compared: Showcase tabs and cards from docs/CARDS.md, plus the
//! PoC slice views that already have references in docs/reference/.

use std::path::Path;

#[derive(Clone, Debug)]
pub struct View {
    /// `data/charts`, `data`, `home`, or the raw BATO_VIEW for anything else.
    pub id: String,
    /// The BATO_VIEW value both clients get.
    pub bato_view: String,
    /// File-name stem for outputs: `data-charts`, `data`, `home`, …
    pub stem: String,
    /// Stored reference, relative to the repo root (may not exist yet).
    pub reference: String,
    /// Full-card / full-page reference (Showcase only).
    pub full_reference: Option<String>,
    /// Shows today's date or the current time, so a stored reference goes stale.
    pub clock: bool,
}

/// The PoC slice references in docs/reference/ (see docs/poc-iced/README.md).
const SLICE: &[(&str, &str)] = &[
    ("home", "view=home"),
    ("scroll-lab", "view=scroll-lab"),
    ("scroll-lab-rows", "view=scroll-lab&scroll=1500"),
    ("options", "view=home&modal=options"),
    ("about", "view=home&modal=about"),
];

/// Ids whose capture (1280×800 from the card's top) shows a date or the time:
/// the whole Date tab, and the Data cards that show or scroll into the Timeline.
fn shows_clock(id: &str) -> bool {
    id == "date" || id.starts_with("date/") || matches!(id, "data/pagination" | "data/charts" | "data/timeline")
}

/// `(tab, [cards])` in page order, parsed from the table in docs/CARDS.md.
pub fn parse_cards(root: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let path = root.join("docs/CARDS.md");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut tabs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("| `") {
            continue;
        }
        let cols: Vec<&str> = line.trim_matches('|').split('|').collect();
        if cols.len() < 2 {
            continue;
        }
        let ticks = |s: &str| -> Vec<String> {
            s.split('`').skip(1).step_by(2).map(str::to_string).collect()
        };
        let (tab, cards) = (ticks(cols[0]), ticks(cols[1]));
        if tab.len() == 1 && !cards.is_empty() {
            tabs.push((tab[0].clone(), cards));
        }
    }
    if tabs.is_empty() {
        return Err(format!("{}: no card table found", path.display()));
    }
    Ok(tabs)
}

fn showcase(tab: &str, card: Option<&str>) -> View {
    let (id, stem, bato_view) = match card {
        Some(c) => (format!("{tab}/{c}"), format!("{tab}-{c}"), format!("view=examples&tab={tab}&card={c}")),
        None => (tab.to_string(), tab.to_string(), format!("view=examples&tab={tab}")),
    };
    View {
        reference: format!("docs/reference/examples/{stem}.png"),
        full_reference: Some(format!("docs/reference/examples/full/{stem}.png")),
        clock: shows_clock(&id),
        id,
        stem,
        bato_view,
    }
}

/// Every Showcase id: each tab (scrolled to the top), then its cards.
pub fn all_showcase(root: &Path) -> Result<Vec<View>, String> {
    let mut out = Vec::new();
    for (tab, cards) in parse_cards(root)? {
        out.push(showcase(&tab, None));
        out.extend(cards.iter().map(|c| showcase(&tab, Some(c))));
    }
    Ok(out)
}

/// Resolves a user-given id: `data/charts`, `data`, a slice name (`home`, …),
/// or a raw BATO_VIEW (anything containing `=`; matched back to an id when it
/// names a known view).
pub fn resolve(root: &Path, arg: &str) -> Result<View, String> {
    let showcase_ids = all_showcase(root)?;
    if arg.contains('=') {
        let q: Vec<(String, String)> = arg
            .split('&')
            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let get = |k: &str| q.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
        if get("view") == Some("examples") && q.iter().all(|(k, _)| matches!(k.as_str(), "view" | "tab" | "card")) {
            let id = match (get("tab").unwrap_or("inputs"), get("card")) {
                (t, Some(c)) => format!("{t}/{c}"),
                (t, None) => t.to_string(),
            };
            if let Some(v) = showcase_ids.iter().find(|v| v.id == id) {
                return Ok(v.clone());
            }
        }
        if let Some((name, _)) = SLICE.iter().find(|(_, bv)| *bv == arg) {
            return resolve(root, name);
        }
        let stem: String = arg.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        return Ok(View {
            id: arg.to_string(),
            bato_view: arg.to_string(),
            reference: format!("docs/reference/other/{stem}.png"),
            full_reference: None,
            clock: false,
            stem,
        });
    }
    if let Some((name, bv)) = SLICE.iter().find(|(n, _)| *n == arg) {
        return Ok(View {
            id: name.to_string(),
            bato_view: bv.to_string(),
            stem: name.to_string(),
            reference: format!("docs/reference/electron-{name}.png"),
            full_reference: None,
            clock: false,
        });
    }
    showcase_ids.into_iter().find(|v| v.id == arg).ok_or_else(|| {
        format!("unknown id `{arg}` (see `compare list`; ids come from docs/CARDS.md, or pass a raw BATO_VIEW)")
    })
}

/// Expands batch selectors: `all`, a tab (`data` → the tab and its cards), or ids.
pub fn select(root: &Path, args: &[String]) -> Result<Vec<View>, String> {
    let all = all_showcase(root)?;
    if args.is_empty() || args.iter().any(|a| a == "all") {
        return Ok(all);
    }
    let mut out: Vec<View> = Vec::new();
    for a in args {
        let tab_members: Vec<View> = all.iter().filter(|v| v.id == *a || v.id.starts_with(&format!("{a}/"))).cloned().collect();
        let add = if tab_members.len() > 1 { tab_members } else { vec![resolve(root, a)?] };
        for v in add {
            if !out.iter().any(|o| o.id == v.id) {
                out.push(v);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn cards_md_parses() {
        let tabs = parse_cards(&root()).unwrap();
        let names: Vec<&str> = tabs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(names, ["inputs", "date", "feedback", "layout", "dashboard", "overlays", "data"]);
        assert_eq!(tabs.iter().map(|(_, c)| c.len()).sum::<usize>(), 32);
        assert_eq!(tabs[1].1, ["pickers", "calendar-time", "calendar-view"]);
    }

    #[test]
    fn ids_resolve_both_ways() {
        let r = root();
        let v = resolve(&r, "data/charts").unwrap();
        assert_eq!(v.bato_view, "view=examples&tab=data&card=charts");
        assert_eq!(v.reference, "docs/reference/examples/data-charts.png");
        assert!(v.clock);
        assert_eq!(resolve(&r, "view=examples&tab=data&card=charts").unwrap().id, "data/charts");
        assert_eq!(resolve(&r, "view=home").unwrap().reference, "docs/reference/electron-home.png");
        assert_eq!(resolve(&r, "layout").unwrap().bato_view, "view=examples&tab=layout");
        assert!(resolve(&r, "data/nope").is_err());
        assert_eq!(select(&r, &["overlays".into()]).unwrap().len(), 3);
        assert_eq!(select(&r, &[]).unwrap().len(), 39);
    }
}
