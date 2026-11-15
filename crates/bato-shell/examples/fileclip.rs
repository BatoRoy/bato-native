//! Drive the file clipboard from a shell (the nested-session proof, and a
//! quick check on a real desktop):
//!
//! ```sh
//! cargo run -p bato-shell --example fileclip -- copy [--cut] [--hold=SECS] PATH…
//! cargo run -p bato-shell --example fileclip -- paste
//! cargo run -p bato-shell --example fileclip -- types
//! ```
//!
//! `copy` keeps serving for `--hold` seconds (default 60): the in-process
//! owners live as long as the process. `BATO_FILE_CLIPBOARD` forces a
//! backend (`data-control`, `wl-clipboard`, `x11`).

use std::path::PathBuf;
use std::time::Duration;

use bato_shell::clipboard::{self, Op};

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    let code = match cmd.as_str() {
        "copy" => {
            let op = if rest.iter().any(|a| a == "--cut") {
                Op::Cut
            } else {
                Op::Copy
            };
            let hold = rest
                .iter()
                .find_map(|a| a.strip_prefix("--hold="))
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            let paths: Vec<PathBuf> = rest
                .iter()
                .filter(|a| !a.starts_with("--"))
                .map(PathBuf::from)
                .collect();
            match clipboard::copy_files(&paths, op) {
                Ok(b) => {
                    println!("copied {} item(s) as {op:?} via {b:?}", paths.len());
                    std::thread::sleep(Duration::from_secs(hold));
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "paste" => match clipboard::paste_files() {
            Ok(Some(f)) => {
                println!("op={:?}", f.op);
                for p in &f.paths {
                    println!("path={}", p.display());
                }
                for o in &f.other {
                    println!("other={o}");
                }
                0
            }
            Ok(None) => {
                println!("no files");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        "types" => match clipboard::types() {
            Ok(t) => {
                for t in t {
                    println!("{t}");
                }
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        _ => {
            eprintln!("usage: fileclip copy [--cut] [--hold=SECS] PATH… | paste | types");
            2
        }
    };
    std::process::exit(code);
}
