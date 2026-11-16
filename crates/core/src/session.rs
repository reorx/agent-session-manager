//! Facts about a session that the app and the `asm` CLI both show: how to resume it, what kind
//! of session it is, which files it wrote. One copy, so the two never disagree.

use crate::model::SessionSummary;

/// `cd '<project>' && claude --resume <id>`, or without the `cd` once the folder is gone (Claude
/// Code looks the id up in every project). `None` for an id that is not a plain session id.
pub fn resume_command(id: &str, project_path: &str, dir_exists: bool) -> Option<String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(match dir_exists {
        true => format!("cd {} && claude --resume {id}", shell_quote(project_path)),
        false => format!("claude --resume {id}"),
    })
}

/// Single quotes keep spaces, `$`, backticks and the like literal; a quote inside closes the
/// quoting, adds an escaped quote and opens it again.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The app's Kind column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Cli,
    Sdk,
    Background,
    /// Claude Code kept only the prompts in `history.jsonl`.
    History,
}

impl SessionKind {
    pub fn of(s: &SessionSummary) -> SessionKind {
        if s.history_only() {
            SessionKind::History
        } else {
            Self::from_metadata(s.is_background, s.entrypoint.as_deref())
        }
    }

    pub fn from_metadata(is_background: bool, entrypoint: Option<&str>) -> Self {
        if is_background {
            SessionKind::Background
        } else if entrypoint == Some("sdk-cli") {
            SessionKind::Sdk
        } else {
            SessionKind::Cli
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Cli => "cli",
            SessionKind::Sdk => "sdk",
            SessionKind::Background => "background",
            SessionKind::History => "history",
        }
    }
}

/// `files_touched` with the project's files relative to it, listed first. Claude Code records a
/// file under the session's working directory relative to it already, and others absolute.
pub fn files_touched(s: &SessionSummary) -> Vec<String> {
    let project = s.project_path.as_deref().or(s.cwd.as_deref()).map(|p| p.trim_end_matches('/')).filter(|p| !p.is_empty());
    let (mut inside, mut outside): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for path in &s.files_touched {
        match project.and_then(|p| path.strip_prefix(p)?.strip_prefix('/')) {
            Some(rel) => inside.push(rel),
            None if !path.starts_with('/') => inside.push(path),
            None => outside.push(path),
        }
    }
    // the others come sorted from the BTreeSet
    inside.sort_unstable();
    inside.dedup();
    inside.into_iter().chain(outside).map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "11111111-aaaa-4bbb-8ccc-000000000001";

    #[test]
    fn the_resume_command_changes_into_the_project_folder_quoted() {
        assert_eq!(resume_command(ID, "/Users/me/Code/alpha", true).as_deref(), Some(format!("cd '/Users/me/Code/alpha' && claude --resume {ID}").as_str()));
        assert_eq!(
            resume_command(ID, "/Users/me/O'Brien $HOME `x`", true).as_deref(),
            Some(format!(r"cd '/Users/me/O'\''Brien $HOME `x`' && claude --resume {ID}").as_str())
        );
        assert_eq!(resume_command(ID, "/missing", false), Some(format!("claude --resume {ID}")));
        assert_eq!(resume_command("1111; rm -rf x", "/p", true), None);
    }

    #[test]
    fn the_kind_follows_history_background_and_entrypoint_in_that_order() {
        let mut s = SessionSummary::transcript(ID, "-p", std::path::Path::new("/p/x.jsonl"));
        assert_eq!(SessionKind::of(&s), SessionKind::Cli);
        s.entrypoint = Some("sdk-cli".into());
        assert_eq!(SessionKind::of(&s).as_str(), "sdk");
        s.is_background = true;
        assert_eq!(SessionKind::of(&s).as_str(), "background");
        s.transcript_missing = true;
        assert_eq!(SessionKind::of(&s).as_str(), "history");
    }

    #[test]
    fn files_inside_the_project_come_first_and_relative() {
        let mut s = SessionSummary::transcript(ID, "-p", std::path::Path::new("/p/x.jsonl"));
        s.project_path = Some("/Users/me/Code/alpha/".into());
        s.files_touched = ["/Users/me/Code/alpha/src/b.rs", "src/a.rs", "/tmp/notes.md", "/Users/me/Code/alphabet/x.rs"].map(String::from).into();
        assert_eq!(files_touched(&s), ["src/a.rs", "src/b.rs", "/Users/me/Code/alphabet/x.rs", "/tmp/notes.md"]);
    }
}
