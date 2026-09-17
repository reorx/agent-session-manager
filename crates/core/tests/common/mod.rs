//! Shared helpers for the behavior tests: a throw-away copy of the fixture `~/.claude` tree with
//! deterministic mtimes, so `updated` timestamps and sizes are stable.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use agent_session_core::claude::paths::ClaudeDir;
use tempfile::TempDir;

pub const S1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
pub const S2: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
pub const S3: &str = "33333333-aaaa-4bbb-8ccc-000000000003";
pub const S4: &str = "44444444-aaaa-4bbb-8ccc-000000000004";
pub const S5: &str = "55555555-aaaa-4bbb-8ccc-000000000005";
pub const S6: &str = "66666666-aaaa-4bbb-8ccc-000000000006";

pub const ALPHA: &str = "-Users-me-Code-alpha";
pub const BETA: &str = "-Users-me-Code-beta";
pub const GAMMA: &str = "-Users-me-Code-gamma";

/// Transcript mtimes (unix ms, UTC) applied to the sandbox copy.
pub const MTIME_S1: i64 = 1788_274_800_000; // 2026-09-01T15:00:00Z
pub const MTIME_S2: i64 = 1788_627_630_000; // 2026-09-05T17:00:30Z
pub const MTIME_S3: i64 = 1789_027_200_000; // 2026-09-10T08:00:00Z
pub const MTIME_S5: i64 = 1788_408_000_000; // 2026-09-03T04:00:00Z

pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude")
}

pub struct Sandbox {
    pub tmp: TempDir,
    pub claude: ClaudeDir,
}

impl Sandbox {
    pub fn root(&self) -> PathBuf {
        self.tmp.path().join("claude")
    }

    pub fn transcript(&self, project: &str, sid: &str) -> PathBuf {
        self.root().join("projects").join(project).join(format!("{sid}.jsonl"))
    }

    pub fn db_path(&self) -> PathBuf {
        self.tmp.path().join("index.sqlite")
    }
}

pub fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    copy_tree(&fixture_root(), &root);
    for (project, sid, mtime) in [
        (ALPHA, S1, MTIME_S1),
        (ALPHA, S2, MTIME_S2),
        (ALPHA, S5, MTIME_S5),
        (BETA, S3, MTIME_S3),
    ] {
        set_mtime(&root.join("projects").join(project).join(format!("{sid}.jsonl")), mtime);
    }
    let claude = ClaudeDir::new(root);
    Sandbox { tmp, claude }
}

pub fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

pub fn set_mtime(path: &Path, unix_ms: i64) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_millis(unix_ms as u64)).unwrap();
}

pub fn append(path: &Path, text: &str) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

pub fn strip_ansi(s: &str) -> String {
    let re = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
    re.replace_all(s, "").into_owned()
}
