//! `compare`: the fidelity harness. Captures the Electron reference (or reuses
//! the stored PNG) and a native window off-screen with the same BATO_VIEW, and
//! reports how far apart they are. See tools/README.md.

mod capture;
mod image;
mod views;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use capture::{Client, Session, Shot};
use image::{Metrics, Rgb};
use views::View;

const USAGE: &str = "\
compare: Electron vs native pixel diff on the off-screen output (tools/README.md)

  compare list                          all ids (docs/CARDS.md) and their BATO_VIEW
  compare one <id> --native <bin>       capture native (+ Electron if needed), diff
  compare batch [<id|tab>...] --native <bin>
                                        the same over many ids (default: all 39), table
  compare diff <reference.png> <candidate.png>
                                        diff two existing images
  compare shot <id> --native <bin> -o <out.png>
                                        capture the native window only
  compare reference [<id|tab>...] [--full]
                                        (re)capture the stored Electron references

  <id>  data/charts · data (tab, scrolled to the top) · home|scroll-lab|scroll-lab-rows|
        options|about (PoC slice) · or a raw BATO_VIEW like 'view=examples&tab=data&card=charts'

options
  --native <bin>        native client; gets BATO_VIEW, BATO_APP_ID, XDG_CONFIG_HOME=<private>
  --app-id <id>         BATO_APP_ID for it (default batopoc-compare; must match ^batopoc-)
  --env K=V             extra env for the native client (repeatable)
  --arg <a>             extra argument for the native client (repeatable)
  --electron <app>      packaged Electron build (default $BATO_ELECTRON, else
                        ../bato-template/dist/electron/linux-unpacked/app)
  --fresh               always capture Electron now instead of using the stored PNG
  --stored              never launch Electron (use stored PNGs even when stale)
  --out <dir>           where images go (default target/compare)
  --threshold <n>       per-channel difference that counts a pixel as differing (default 8)
  --fail-above <mean>   exit 1 if any mean difference is above this
  --full                (reference) also write full-card / full-page images
";

struct Opts {
    pos: Vec<String>,
    native: Option<PathBuf>,
    app_id: String,
    env: Vec<(String, String)>,
    args: Vec<String>,
    electron: Option<PathBuf>,
    fresh: bool,
    stored: bool,
    out: Option<PathBuf>,
    threshold: u8,
    fail_above: Option<f64>,
    full: bool,
    output: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        pos: vec![],
        native: None,
        app_id: "batopoc-compare".into(),
        env: vec![],
        args: vec![],
        electron: None,
        fresh: false,
        stored: false,
        out: None,
        threshold: 8,
        fail_above: None,
        full: false,
        output: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--native" => o.native = Some(val()?.into()),
            "--app-id" => o.app_id = val()?,
            "--env" => {
                let kv = val()?;
                let (k, v) = kv.split_once('=').ok_or("--env takes K=V")?;
                o.env.push((k.into(), v.into()));
            }
            "--arg" => o.args.push(val()?),
            "--electron" => o.electron = Some(val()?.into()),
            "--fresh" => o.fresh = true,
            "--stored" => o.stored = true,
            "--out" => o.out = Some(val()?.into()),
            "-o" => o.output = Some(val()?.into()),
            "--threshold" => o.threshold = val()?.parse().map_err(|_| "--threshold: 0–255")?,
            "--fail-above" => o.fail_above = Some(val()?.parse().map_err(|_| "--fail-above: a number")?),
            "--full" => o.full = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => o.pos.push(a.clone()),
        }
    }
    if !o.app_id.starts_with("batopoc-") {
        return Err("--app-id must start with batopoc- (only those are routed off-screen)".into());
    }
    Ok(o)
}

fn repo_root() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    p.canonicalize().unwrap_or(p)
}

fn electron_app(o: &Opts, root: &Path) -> Result<PathBuf, String> {
    let p = o
        .electron
        .clone()
        .or_else(|| std::env::var_os("BATO_ELECTRON").map(PathBuf::from))
        .unwrap_or_else(|| root.join("../bato-template/dist/electron/linux-unpacked/app"));
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!(
            "no packaged Electron build at {} (build one: cd bato-template/app-client && npm run package:linux; \
             then pass --electron or set BATO_ELECTRON)",
            p.display()
        ))
    }
}

fn native_client(o: &Opts) -> Result<Client, String> {
    let bin = o.native.clone().ok_or("--native <bin> is required")?;
    if !bin.is_file() {
        return Err(format!("{}: not a file", bin.display()));
    }
    Ok(Client::Native { bin, app_id: o.app_id.clone(), env: o.env.clone(), args: o.args.clone() })
}

fn now_iso() -> String {
    Command::new("date")
        .arg("-Iseconds")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join("docs/reference/examples/manifest.json")
}

fn load_manifest(root: &Path) -> serde_json::Value {
    std::fs::read(manifest_path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({ "shots": {} }))
}

/// Where the reference for `v` comes from this time.
enum RefSource {
    Stored(PathBuf, String),
    Capture(String),
}

fn reference_source(o: &Opts, root: &Path, v: &View, manifest: &serde_json::Value) -> RefSource {
    let stored = root.join(&v.reference);
    if o.fresh || !stored.is_file() {
        return RefSource::Capture(if o.fresh { "fresh".into() } else { "no stored reference".into() });
    }
    let date: String = manifest["shots"][&v.id]["captured"].as_str().unwrap_or("").chars().take(10).collect();
    if v.clock && !o.stored {
        return RefSource::Capture("shows the date/time".into());
    }
    let note = if v.clock { format!("stored {date}, shows the date/time: stale") } else { "stored".into() };
    RefSource::Stored(stored, note)
}

struct Row {
    id: String,
    m: Metrics,
    notes: Vec<String>,
}

fn print_header() {
    println!("{:<26} {:>6} {:>8} {:>8}  {:<22} notes", "id", "mean", ">8 %", "px>24Σ", "bbox (x0,y0–x1,y1)");
}

fn print_row(r: &Row) {
    let bbox = r.m.bbox.map_or("-".into(), |b| format!("{},{}–{},{}", b.x0, b.y0, b.x1, b.y1));
    let mut notes = r.notes.clone();
    if let Some(s) = &r.m.size_note {
        notes.push(s.clone());
    }
    println!(
        "{:<26} {:>6.3} {:>7.2}% {:>8}  {:<22} {}",
        r.id,
        r.m.mean,
        r.m.over_pct,
        r.m.over24_sum,
        bbox,
        notes.join(" ")
    );
}

/// Diffs and writes `<stem>.{diff,side}.png` (+ the inputs) into `out`.
fn diff_and_write(reference: &Rgb, cand: &Rgb, mask: &[image::Rect], threshold: u8, out: &Path, stem: &str) -> Result<Metrics, String> {
    let (m, d) = image::diff(reference, cand, threshold, mask);
    d.save(&out.join(format!("{stem}.diff.png")))?;
    image::side_by_side(&[reference, cand, &d]).save(&out.join(format!("{stem}.side.png")))?;
    Ok(m)
}

fn compare_one(o: &Opts, root: &Path, s: &mut Session, v: &View, manifest: &serde_json::Value, out: &Path) -> Result<Row, String> {
    let native = native_client(o)?;
    let mut notes = Vec::new();
    // Clock-dependent views: capture both within seconds of each other.
    let (reference, ref_mask) = match reference_source(o, root, v, manifest) {
        RefSource::Stored(p, note) => {
            notes.push(format!("ref:{note}"));
            (Rgb::load(&p)?, vec![])
        }
        RefSource::Capture(why) => {
            let app = electron_app(o, root)?;
            notes.push(format!("ref:captured ({why})"));
            let shot = s.capture_electron(&app, &v.bato_view, false)?;
            shot.img.save(&out.join(format!("{}.electron.png", v.stem)))?;
            notes.extend(shot.notes.iter().map(|n| format!("electron:{n}")));
            (shot.img, shot.mask)
        }
    };
    let Shot { img, mask, notes: nn, .. } = s.capture_native(&native, &v.bato_view)?;
    img.save(&out.join(format!("{}.native.png", v.stem)))?;
    notes.extend(nn.iter().map(|n| format!("native:{n}")));
    let all_mask: Vec<_> = ref_mask.into_iter().chain(mask).collect();
    let m = diff_and_write(&reference, &img, &all_mask, o.threshold, out, &v.stem)?;
    Ok(Row { id: v.id.clone(), m, notes })
}

fn cmd_reference(o: &Opts, root: &Path) -> Result<(), String> {
    let app = electron_app(o, root)?;
    let list = views::select(root, &o.pos)?;
    if let Some(v) = list.iter().find(|v| v.full_reference.is_none()) {
        return Err(format!("`reference` only (re)captures Showcase ids; `{}` is not one", v.id));
    }
    let mut manifest = load_manifest(root);
    let mut s = Session::new(root)?;
    let commit = template_commit(&app);
    for v in &list {
        let shot = s.capture_electron(&app, &v.bato_view, false)?;
        let path = root.join(&v.reference);
        shot.img.save(&path)?;
        let mut entry = serde_json::json!({
            "file": v.reference,
            "bato_view": v.bato_view,
            "captured": now_iso(),
            "size": format!("{}x{}", shot.img.w, shot.img.h),
            "stable": shot.stable,
            "clock": v.clock,
            "notes": shot.notes,
        });
        let mut line = format!("{:<26} {}x{} {}", v.id, shot.img.w, shot.img.h, shot.notes.join(" "));
        if o.full {
            if let Some(full) = &v.full_reference {
                let fp = root.join(full);
                let img = s.capture_electron(&app, &v.bato_view, true)?.img;
                if img.h <= shot.img.h {
                    // Fits the window: the 1280×800 shot already shows all of it.
                    std::fs::remove_file(&fp).ok();
                    entry["full"] = serde_json::Value::Null;
                } else {
                    img.save(&fp)?;
                    entry["full"] = serde_json::json!({ "file": full, "size": format!("{}x{}", img.w, img.h) });
                    line.push_str(&format!("  full {}x{}", img.w, img.h));
                }
            }
        }
        println!("{line}");
        manifest["shots"][&v.id] = entry;
    }
    manifest["electron"] = serde_json::json!({
        "build": "app-client npm run package:linux (dist/electron/linux-unpacked/app)",
        "template_commit": commit,
        "settings": "fresh --user-data-dir: dark theme, brand accent, 100 %, comfortable width",
        "window": "1280x800 on the off-screen output; Chromium's own capture (BATO_CAPTURE), equal to a tools/shot.sh capture except the 4x4 compositor-rounded corners",
        "full": "BATO_CAPTURE_FULL=1: viewport emulated to card height + 48 (or the page height for a tab)",
    });
    let mp = manifest_path(root);
    std::fs::create_dir_all(mp.parent().unwrap()).ok();
    std::fs::write(&mp, serde_json::to_string_pretty(&manifest).unwrap() + "\n").map_err(|e| e.to_string())?;
    println!("wrote {} reference(s) and {}", list.len(), mp.display());
    Ok(())
}

fn template_commit(app: &Path) -> String {
    let mut d = app.parent();
    while let Some(dir) = d {
        if dir.join(".git").exists() {
            let git = |args: &[&str]| {
                Command::new("git")
                    .arg("-C")
                    .arg(dir)
                    .args(args)
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            };
            let head = git(&["rev-parse", "--short", "HEAD"]);
            let dirty = !git(&["status", "--porcelain", "--", "app-client"]).is_empty();
            return if dirty { format!("{head}+dirty") } else { head };
        }
        d = dir.parent();
    }
    "unknown".into()
}

fn run(args: &[String]) -> Result<bool, String> {
    let Some((cmd, rest)) = args.split_first() else { return Err(String::new()) };
    let o = parse(rest)?;
    let root = repo_root();
    let out = o.out.clone().unwrap_or_else(|| root.join("target/compare"));
    let over = |m: &Metrics| o.fail_above.is_some_and(|f| m.mean > f);
    match cmd.as_str() {
        "list" => {
            for v in views::all_showcase(&root)? {
                let stored = if root.join(&v.reference).is_file() { "" } else { "  (no reference yet)" };
                println!("{:<26} {}{}", v.id, v.bato_view, stored);
            }
            Ok(true)
        }
        "diff" => {
            let [a, b] = o.pos.as_slice() else { return Err("diff <reference.png> <candidate.png>".into()) };
            let (ra, rb) = (Rgb::load(Path::new(a))?, Rgb::load(Path::new(b))?);
            let stem = Path::new(b).file_stem().map_or("diff".into(), |s| s.to_string_lossy().to_string());
            let m = diff_and_write(&ra, &rb, &[], o.threshold, &out, &stem)?;
            print_header();
            let ok = !over(&m);
            print_row(&Row { id: stem.clone(), m, notes: vec![] });
            println!("images: {}/{stem}.{{diff,side}}.png", out.display());
            Ok(ok)
        }
        "shot" => {
            let [id] = o.pos.as_slice() else { return Err("shot <id> --native <bin> -o <out.png>".into()) };
            let v = views::resolve(&root, id)?;
            let dest = o.output.clone().ok_or("-o <out.png> is required")?;
            let mut s = Session::new(&root)?;
            let shot = s.capture_native(&native_client(&o)?, &v.bato_view)?;
            shot.img.save(&dest)?;
            println!("{} ({}x{}) {}", dest.display(), shot.img.w, shot.img.h, shot.notes.join(" "));
            Ok(true)
        }
        "one" => {
            let [id] = o.pos.as_slice() else { return Err("one <id> --native <bin>".into()) };
            let v = views::resolve(&root, id)?;
            native_client(&o)?;
            let manifest = load_manifest(&root);
            let mut s = Session::new(&root)?;
            let row = compare_one(&o, &root, &mut s, &v, &manifest, &out)?;
            print_header();
            print_row(&row);
            println!("images: {}/{}.{{native,diff,side}}.png  (side = electron | native | diff)", out.display(), v.stem);
            Ok(!over(&row.m))
        }
        "batch" => {
            let list = views::select(&root, &o.pos)?;
            native_client(&o)?;
            let manifest = load_manifest(&root);
            let mut s = Session::new(&root)?;
            print_header();
            let mut tsv = String::from("id\tmean\tover_pct\tover_px\tcompared_px\tover24_sum\tbbox\tnotes\n");
            let mut ok = true;
            for v in &list {
                match compare_one(&o, &root, &mut s, v, &manifest, &out) {
                    Ok(row) => {
                        print_row(&row);
                        ok &= !over(&row.m);
                        let bbox = row.m.bbox.map_or("-".into(), |b| format!("{},{},{},{}", b.x0, b.y0, b.x1, b.y1));
                        tsv += &format!(
                            "{}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{}\t{}\n",
                            row.id,
                            row.m.mean,
                            row.m.over_pct,
                            row.m.over,
                            row.m.compared,
                            row.m.over24_sum,
                            bbox,
                            row.notes.join(" ")
                        );
                    }
                    Err(e) => {
                        ok = false;
                        println!("{:<26} ERROR {e}", v.id);
                        tsv += &format!("{}\t\t\t\t\t\t\tERROR {}\n", v.id, e.replace(['\n', '\t'], " "));
                    }
                }
            }
            std::fs::create_dir_all(&out).ok();
            std::fs::write(out.join("summary.tsv"), tsv).map_err(|e| e.to_string())?;
            println!("images and summary.tsv: {}", out.display());
            Ok(ok)
        }
        "reference" => cmd_reference(&o, &root).map(|_| true),
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) if e.is_empty() => {
            print!("{USAGE}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("compare: {e}");
            ExitCode::from(2)
        }
    }
}
