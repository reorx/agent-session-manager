//! What can be done with a session: copy its resume command or ID, reveal its transcript, open
//! its project folder, export it as TXT, favorite it. The detail header, the table's context menu
//! and the shortcuts dispatch the same actions; `available` decides from a few facts about a row
//! which of them can run, and says why the others cannot.
//!
//! Those facts need the file system (is the project folder there, is a favorite's original), so
//! `check` runs only when they matter: when a row is selected (for the header), when the context
//! menu opens, and when an action runs. Never per row of the table.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_session_core::claude::paths::{ClaudeDir, home_dir};
use agent_session_core::favorites;
use gpui_kit::component::WindowExt;
use gpui_kit::*;

use crate::AppRoot;
use crate::model::SessionRow;

pub const TRANSCRIPT_GONE: &str = "The transcript is gone";
pub const NO_ID: &str = "This session has no ID";
/// Claude Code could load the copy by its path, but would then write into the favorites folder
/// (kb/sessions/2026-10-27-asm-m4b-resume.md §4), so it is not offered yet.
pub const FAVORITE_ONLY: &str = "Only the favorite copy is left; restoring from a favorite is not supported here yet";
pub const NO_FOLDER: &str = "The project folder no longer exists";
const NOTHING_SELECTED: &str = "No session is selected";

/// `Err` carries the reason shown in a tooltip or a notification.
pub type Availability = Result<(), &'static str>;

/// Which actions a session allows.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionActions {
    pub favorite: Availability,
    pub copy_resume: Availability,
    pub copy_id: Availability,
    pub reveal: Availability,
    pub open_folder: Availability,
    pub export: Availability,
    /// What Copy resume copies; `None` while it cannot.
    pub resume: Option<String>,
}

impl Default for SessionActions {
    fn default() -> Self {
        let no = Err(NOTHING_SELECTED);
        Self { favorite: no, copy_resume: no, copy_id: no, reveal: no, open_folder: no, export: no, resume: None }
    }
}

/// `original_exists`: the transcript Claude Code keeps is still there (a favorite may be all
/// that is left of it); `dir_exists`: the project folder is.
pub fn available(row: &SessionRow, original_exists: bool, dir_exists: bool) -> SessionActions {
    let transcript = if row.transcript_path.is_some() { Ok(()) } else { Err(TRANSCRIPT_GONE) };
    let copy_id = if row.id_known { Ok(()) } else { Err(NO_ID) };
    let original = if original_exists { Ok(()) } else { Err(FAVORITE_ONLY) };
    let resumable = transcript.and(copy_id).and(original);
    let resume = resumable.ok().and_then(|_| resume_command(&row.id, &row.project_path, dir_exists));
    SessionActions {
        favorite: transcript,
        // an id unfit for a command line has no command either
        copy_resume: if resume.is_some() { Ok(()) } else { resumable.and(Err(NO_ID)) },
        copy_id,
        reveal: transcript,
        open_folder: if dir_exists { Ok(()) } else { Err(NO_FOLDER) },
        export: transcript,
        resume,
    }
}

/// `available` for a row as things are on disk now.
pub fn check(row: &SessionRow, claude: &ClaudeDir) -> SessionActions {
    let dir = Path::new(row.project_path.as_ref());
    let dir_exists = dir.is_absolute() && dir.is_dir();
    let original_exists = !row.favorite || favorites::original_exists(claude, &row.project_key, &row.id);
    available(row, original_exists, dir_exists)
}

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
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// `<project>-<first 8 of the id>.txt`.
pub fn export_name(row: &SessionRow) -> String {
    let project: String = row.project.chars().map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '-' } else { c }).collect();
    let short: String = row.id.chars().take(8).collect();
    format!("{project}-{short}.txt")
}

// Each acts on the session with `id`, or on the selected one when it is `None`: the context
// menu names the row it was opened on, the shortcuts and the header buttons mean the selection.
// A menu looks its shortcut up by action equality, so `Some(..)` items draw their hint themselves.

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct CopySessionId {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct CopyResume {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct RevealTranscript {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct OpenProjectFolder {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct ExportTranscript {
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct HideProject {
    pub id: Option<String>,
}

impl AppRoot {
    /// The row an action is for, with what it allows as things are on disk now.
    fn action_target(&self, id: Option<&str>, cx: &App) -> Option<(Arc<SessionRow>, SessionActions)> {
        let row = match id {
            Some(id) => self.table.read(cx).delegate().rows.iter().find(|r| r.id == id).cloned(),
            None => self.selected.clone(),
        }?;
        let actions = check(&row, &self.claude);
        Some((row, actions))
    }

    /// Run `act` if `allowed` says the session allows it; otherwise say why it does not and do nothing.
    fn run_action(
        &mut self,
        id: Option<&str>,
        allowed: fn(&SessionActions) -> Availability,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Self, &SessionRow, &SessionActions, &mut Window, &mut Context<Self>),
    ) {
        let Some((row, actions)) = self.action_target(id, cx) else { return };
        match allowed(&actions) {
            Ok(()) => act(self, &row, &actions, window, cx),
            Err(reason) => window.push_notification(reason, cx),
        }
    }

    pub(crate) fn copy_session_id(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, |a| a.copy_id, window, cx, |_, row, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(row.id.to_string()));
            window.push_notification("已复制 session ID", cx);
        });
    }

    pub(crate) fn copy_resume(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, |a| a.copy_resume, window, cx, |_, _, actions, window, cx| {
            if let Some(command) = &actions.resume {
                cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                window.push_notification("已复制 resume 命令", cx);
            }
        });
    }

    /// Select the transcript in Finder: the favorite copy for a favorite.
    pub(crate) fn reveal_transcript(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, |a| a.reveal, window, cx, |_, row, _, _, cx| {
            if let Some(path) = &row.transcript_path {
                cx.reveal_path(path);
            }
        });
    }

    pub(crate) fn open_project_folder(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, |a| a.open_folder, window, cx, |_, row, _, _, cx| cx.open_with_system(Path::new(row.project_path.as_ref())));
    }

    /// Ask where to save (in the folder of the last export of this run, at first Downloads), then
    /// have the reader thread write it; `exported` reports back.
    pub(crate) fn export_transcript(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, |a| a.export, window, cx, |this, row, _, window, cx| {
            let Some(src) = row.transcript_path.clone() else { return };
            let dir = this.last_export_dir.clone().unwrap_or_else(default_export_dir);
            let answer = cx.prompt_for_new_path(&dir, Some(&export_name(row)));
            cx.spawn_in(window, async move |this, cx| {
                let picked = answer.await;
                this.update_in(cx, |this, window, cx| {
                    match picked {
                        Ok(Ok(Some(dest))) => {
                            this.last_export_dir = dest.parent().map(Path::to_path_buf);
                            this.store.export(src, dest);
                        }
                        Ok(Err(error)) => window.push_notification(format!("{error}"), cx),
                        _ => {}
                    }
                    this.focus_table(window, cx);
                })
                .ok();
            })
            .detach();
        });
    }

    pub(crate) fn exported(&mut self, dest: &Path, result: Result<usize, String>, window: &mut Window, cx: &mut Context<Self>) {
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let message = match result {
            Ok(0) => format!("已导出 {name}"),
            Ok(skipped) => format!("已导出 {name}，有 {skipped} 行无法解析，已跳过"),
            Err(error) => format!("导出失败：{error}"),
        };
        window.push_notification(message, cx);
    }

    /// What View › Hide Project of Selected Session does, for any row.
    pub(crate) fn hide_project_of(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        if let Some((row, _)) = self.action_target(id, cx) {
            self.hide_project(row.project_key.clone(), cx);
        }
    }
}

/// Where a saved export goes when nothing was exported yet in this run.
pub fn default_export_dir() -> PathBuf {
    home_dir().join("Downloads")
}

#[cfg(test)]
mod tests {
    // `gpui_kit::*` exports gpui's own `test` attribute; name the standard one explicitly.
    use core::prelude::v1::test;

    use agent_session_core::model::SessionSummary;

    use super::*;
    use crate::model::tests::summary;

    const ID: &str = "11111111-aaaa-4bbb-8ccc-000000000001";

    fn transcript_row() -> SessionRow {
        SessionRow::from_summary(&summary(ID, "/Users/me/Code/alpha", 5000))
    }

    fn favorite_row() -> SessionRow {
        let mut s = summary(ID, "/Users/me/Code/alpha", 5000);
        s.favorite = true;
        SessionRow::from_summary(&s)
    }

    fn history_row(id: &str, id_known: bool) -> SessionRow {
        SessionRow::from_summary(&SessionSummary::history(id, id_known, "-Users-me-Code-alpha", "/Users/me/Code/alpha"))
    }

    /// (favorite, copy_resume, copy_id, reveal, open_folder, export) that can run.
    fn enabled(a: &SessionActions) -> [bool; 6] {
        [a.favorite, a.copy_resume, a.copy_id, a.reveal, a.open_folder, a.export].map(|x| x.is_ok())
    }

    #[test]
    fn the_resume_command_changes_into_the_project_folder() {
        assert_eq!(resume_command(ID, "/Users/me/Code/alpha", true).as_deref(), Some("cd '/Users/me/Code/alpha' && claude --resume 11111111-aaaa-4bbb-8ccc-000000000001"));
        let a = available(&transcript_row(), true, true);
        assert_eq!(enabled(&a), [true; 6]);
        assert_eq!(a.resume, resume_command(ID, "/Users/me/Code/alpha", true));
    }

    #[test]
    fn the_resume_command_quotes_a_path_with_spaces() {
        assert_eq!(resume_command(ID, "/Users/me/My Code/中文 项目", true).as_deref(), Some(format!("cd '/Users/me/My Code/中文 项目' && claude --resume {ID}").as_str()));
        assert_eq!(
            resume_command(ID, "/Users/me/O'Brien $HOME `x`", true).as_deref(),
            Some(format!(r"cd '/Users/me/O'\''Brien $HOME `x`' && claude --resume {ID}").as_str()),
            "a quote inside the path closes, escapes and reopens; $ and backticks stay literal"
        );
        assert_eq!(resume_command("1111; rm -rf x", "/p", true), None, "an id that is not a plain session id is never put on a command line");
    }

    #[test]
    fn the_resume_command_without_a_project_folder_follows_step_0() {
        // Claude Code (2.1.223+) finds a session id in any project, so it resumes from wherever
        // the terminal is (kb/sessions/2026-10-27-asm-m4b-resume.md)
        assert_eq!(resume_command(ID, "/missing/project", false), Some(format!("claude --resume {ID}")));
    }

    #[test]
    fn a_history_session_can_copy_its_id_and_open_its_folder_only() {
        let a = available(&history_row("44444444-aaaa-4bbb-8ccc-000000000004", true), true, true);
        assert_eq!(enabled(&a), [false, false, true, false, true, false]);
        for reason in [a.favorite, a.copy_resume, a.reveal, a.export] {
            assert_eq!(reason, Err(TRANSCRIPT_GONE));
        }
        assert_eq!(a.resume, None);
    }

    #[test]
    fn a_session_without_an_id_can_only_open_its_folder() {
        let a = available(&history_row("history:-Users-me-Code-alpha:1748736000000", false), true, true);
        assert_eq!(enabled(&a), [false, false, false, false, true, false]);
        assert_eq!(a.copy_id, Err(NO_ID));
        assert_eq!(a.copy_resume, Err(TRANSCRIPT_GONE));
        assert_eq!(a.resume, None);
    }

    #[test]
    fn a_favorite_whose_original_is_gone_cannot_be_resumed_but_can_be_exported() {
        let a = available(&favorite_row(), false, true);
        assert_eq!(enabled(&a), [true, false, true, true, true, true]);
        assert_eq!(a.copy_resume, Err(FAVORITE_ONLY));
        assert_eq!(a.resume, None);
        let a = available(&favorite_row(), false, false);
        assert_eq!(a.copy_resume, Err(FAVORITE_ONLY), "a missing folder does not make the copy resumable");
        assert_eq!(enabled(&available(&favorite_row(), true, true)), [true; 6], "with the original still there it is a session like any other");
    }

    #[test]
    fn a_missing_project_folder_disables_only_open_folder() {
        let a = available(&transcript_row(), true, false);
        assert_eq!(enabled(&a), [true, true, true, true, false, true]);
        assert_eq!(a.open_folder, Err(NO_FOLDER));
        assert_eq!(a.resume, Some(format!("claude --resume {ID}")));
    }

    #[test]
    fn the_export_name_is_project_and_short_id() {
        assert_eq!(export_name(&transcript_row()), "alpha-11111111.txt");
        let odd = SessionRow::from_summary(&summary(ID, "/Users/me/a:b\\c", 5000));
        assert_eq!(export_name(&odd), "a-b-c-11111111.txt", "no path separators in a file name");
    }

    #[test]
    fn check_looks_at_the_project_folder_and_at_the_original_of_a_favorite() {
        let dir = tempfile::tempdir().unwrap();
        let claude = ClaudeDir::new(dir.path().join("claude"));
        let project = dir.path().join("alpha");
        std::fs::create_dir_all(&project).unwrap();
        let mut s = summary(ID, project.to_str().unwrap(), 5000);
        let original = claude.projects_dir().join(&s.project_key).join(format!("{ID}.jsonl"));
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, "{}\n").unwrap();

        let a = check(&SessionRow::from_summary(&s), &claude);
        assert_eq!(enabled(&a), [true; 6]);
        assert_eq!(a.resume, resume_command(ID, project.to_str().unwrap(), true));

        s.favorite = true;
        std::fs::remove_file(&original).unwrap();
        assert_eq!(check(&SessionRow::from_summary(&s), &claude).copy_resume, Err(FAVORITE_ONLY));
        std::fs::remove_dir(&project).unwrap();
        assert_eq!(check(&SessionRow::from_summary(&s), &claude).open_folder, Err(NO_FOLDER));
        s.project_path = Some("alpha".into());
        assert_eq!(check(&SessionRow::from_summary(&s), &claude).open_folder, Err(NO_FOLDER), "a relative path is no folder");
    }
}
