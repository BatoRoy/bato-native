//! "Restart to update" end to end, without the registry: a fake install dir
//! and a fake `bato` on `PATH` that "installs" a new binary the way the real
//! CLI does (unlink, write a new file), then `relaunch` starts it.
//!
//! Its own test binary (one test), because it changes `PATH` for the process.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bato_shell::AppIdentity;
use bato_shell::updater::{self, UpdatePhase, Updater};

const APP: AppIdentity = AppIdentity::new("Fixture", "batoshellrelaunch", "1.2.0");

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn wait_for(path: &Path) -> String {
    for _ in 0..100 {
        if let Ok(s) = std::fs::read_to_string(path)
            && s.ends_with('\n')
        {
            return s;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{} never appeared", path.display());
}

#[test]
fn install_with_a_fake_cli_then_relaunch_the_new_binary() {
    let root: PathBuf =
        std::env::temp_dir().join(format!("bato-shell-relaunch-{}", std::process::id()));
    let bin_dir = root.join("bin");
    let install = root.join("apps").join(APP.slug);
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&install).unwrap();
    let exe = install.join(APP.slug);
    let marker = root.join("relaunched");

    // The installed 1.2.0: must never run.
    script(&exe, "echo old >&2; exit 9\n");

    // `bato -V` answers; `bato install <slug> --json --progress` swaps the
    // binary for 1.3.0 and streams the CLI's NDJSON (bato 1.4.9 contract).
    script(
        &bin_dir.join("bato"),
        &format!(
            r#"case "$1" in
  -V) echo 1.4.9 ;;
  install)
    [ "$2" = "{slug}" ] && [ "$3" = "--json" ] && [ "$4" = "--progress" ] || exit 2
    echo '{{"event":"plan","name":"{slug}","version":"1.3.0","type":"desktop"}}'
    echo '{{"event":"progress","received":512,"total":1024}}'
    rm -f '{exe}'
    printf '#!/bin/sh\necho "1.3.0 $*" > "{m}.tmp"; mv "{m}.tmp" "{m}"\n' > '{exe}'
    chmod 755 '{exe}'
    echo '{{"event":"progress","received":1024,"total":1024}}'
    echo '{{"event":"done","ok":true,"name":"{slug}","version":"1.3.0","type":"desktop"}}'
    echo 'Installed {slug} 1.3.0' >&2 ;;
  *) exit 2 ;;
esac
"#,
            slug = APP.slug,
            exe = exe.display(),
            m = marker.display(),
        ),
    );

    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin_dir.clone()];
    dirs.extend(std::env::split_paths(&path));
    // SAFETY: the only test in this binary, before any thread reads the env.
    unsafe {
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::remove_var(updater::ENV_BATO_BIN);
    }
    assert_eq!(updater::find_bato(), Some(PathBuf::from("bato")));

    let up = Updater::new(&APP, updater::DEFAULT_REGISTRY)
        .with_install_dir(&install)
        .with_packaged(true);
    let mut phases = Vec::new();
    assert_eq!(up.apply(|p| phases.push(p)).as_deref(), Ok("1.3.0"));
    assert_eq!(
        phases,
        vec![
            UpdatePhase::Downloading { percent: 0 },
            UpdatePhase::Downloading { percent: 50 },
            UpdatePhase::Downloading { percent: 100 },
            UpdatePhase::Downloaded {
                version: "1.3.0".into()
            },
        ]
    );

    // What the About button / sidebar "Restart" does, minus the exit.
    up.relaunch().unwrap();
    let line = wait_for(&marker);
    assert!(line.starts_with("1.3.0"), "the new binary ran: {line:?}");
    std::fs::remove_dir_all(root).unwrap();
}
