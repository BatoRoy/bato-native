//! Test helpers shared by the modules' tests.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write an executable script that a test then runs.
///
/// Written by a child `sh`, never through a file descriptor of this process:
/// tests run in parallel threads, and a thread that forks (to spawn any
/// child) while another holds the script open for writing hands that
/// descriptor to its child until the child's `exec`; running the script in
/// that window fails with ETXTBSY ("Text file busy"). Seen once in 40 runs
/// of `cargo test -p bato-shell` beside a CPU hog.
pub(crate) fn write_script(path: &Path, body: &str) {
    let mut sh = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("sh");
    sh.stdin
        .take()
        .expect("stdin")
        .write_all(body.as_bytes())
        .expect("write script");
    assert!(
        sh.wait().expect("sh").success(),
        "writing {}",
        path.display()
    );
}
