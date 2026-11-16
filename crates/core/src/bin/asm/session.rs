//! `session`: one session's card, for agents: what it was about, where it ran, how big it got,
//! and how to resume it.

use std::io::Write;
use std::time::Instant;

use agent_session_core::claude::clean::one_line;
use agent_session_core::claude::model_catalog::ModelCatalog;
use agent_session_core::cost::primary_model;
use agent_session_core::format::{fmt_datetime, fmt_tokens, fmt_usd};
use agent_session_core::model::SessionSummary;
use agent_session_core::pricing::context_window;
use agent_session_core::search::sessions::{last_prompt, turn_count};
use agent_session_core::session::{SessionKind, files_touched};
use anyhow::Result;
use clap::Args;
use serde_json::{Map, Value, json};

use crate::agent::{LookupError, favorite_copy_only, json_prompt, plural, project_name, resume, resume_text, rfc3339, title, usage};
use crate::App;

/// Characters of a prompt in text output.
const PROMPT_LEN: usize = 300;
const TOOLS_SHOWN: usize = 8;
const FILES_SHOWN: usize = 10;
const NONE: &str = "—";

#[derive(Args)]
pub struct SessionArgs {
    /// Full session ID or a unique prefix
    #[arg(value_name = "ID")]
    id: String,
    /// JSON output
    #[arg(long)]
    json: bool,
    /// Do not refresh the index first
    #[arg(long)]
    no_scan: bool,
}

/// What the card shows beyond the index row.
struct Card {
    turns: u32,
    /// The last prompt with text (see `last_prompt`); from `history.jsonl` for a history-only
    /// session, which has no prompt rows.
    last_prompt: Option<String>,
    /// Context window of the model of the last main-thread call.
    window: Option<u64>,
    favorite_copy: bool,
}

impl App {
    pub(crate) fn session_card(&mut self, args: SessionArgs) -> Result<()> {
        let json = args.json;
        let result = self.session_card_inner(&args);
        self.fail_as_json(json, result)
    }

    fn session_card_inner(&mut self, args: &SessionArgs) -> Result<()> {
        let started = Instant::now();
        // an empty prefix would match every session (an unset shell variable, say)
        if args.id.trim().is_empty() {
            return Err(usage("ID is empty"));
        }
        let (db, refresh) = self.open_db_refreshed(!args.no_scan)?;
        let mut found = db.find_by_prefix(&args.id)?;
        let s = match found.len() {
            0 => return Err(LookupError::NotFound(args.id.clone()).into()),
            1 => found.remove(0),
            _ => {
                found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
                return Err(LookupError::Ambiguous { prefix: args.id.clone(), candidates: found }.into());
            }
        };
        let catalog = ModelCatalog::load(&self.claude);
        let card = Card {
            turns: turn_count(&db, &s.id)?,
            last_prompt: if s.history_only() { s.last_prompt.clone() } else { last_prompt(&db, &s.id)? },
            window: s.context_model.as_deref().and_then(|m| context_window(&catalog, m)),
            favorite_copy: favorite_copy_only(&s, &self.claude),
        };
        match args.json {
            true => {
                let meta = json!({ "command": "session", "query": args.id, "index_refresh": refresh.as_str(), "elapsed_ms": started.elapsed().as_millis() });
                let session = self.card_json(&s, &card);
                self.write_json(&json!({ "meta": meta, "session": session }))
            }
            false => self.card_text(&s, &card),
        }
    }

    fn card_text(&mut self, s: &SessionSummary, card: &Card) -> Result<()> {
        let prompt = |p: Option<&str>| p.map(str::trim).filter(|p| !p.is_empty()).map_or(NONE.to_string(), |p| one_line(p, PROMPT_LEN));
        let or_none = |v: Option<&str>| v.filter(|v| !v.is_empty()).unwrap_or(NONE).to_string();
        let mut rows: Vec<(&str, String)> = vec![
            ("title", title(s).unwrap_or_else(|| NONE.into())),
            ("first prompt", prompt(Some(&s.first_prompt))),
            ("last prompt", prompt(card.last_prompt.as_deref())),
            ("project", or_none(s.project_path.as_deref())),
            ("cwd", or_none(s.cwd.as_deref())),
            ("branch", or_none(s.git_branch.as_deref())),
            ("kind", SessionKind::of(s).as_str().into()),
            ("favorite", if s.favorite { "yes" } else { "no" }.into()),
            ("started", s.started_at.map_or(NONE.into(), |ms| fmt_datetime(Some(ms)))),
            ("updated", fmt_datetime(Some(s.updated_at))),
            ("turns", format!("{} ({})", card.turns, plural(s.user_msg_count as usize, "prompt", "prompts"))),
            ("tools", tools_text(s)),
            ("files", files_text(s)),
            ("cost", cost_text(s)),
            ("tokens", tokens_text(s)),
            ("context", context_text(s, card.window)),
            ("models", models_text(s)),
            ("subagents", s.subagent_count.to_string()),
        ];
        let transcript = match (&s.file_path, s.history_only()) {
            (Some(path), false) => {
                let size = s.file_size.map(|b| format!(" ({})", byte_size(b))).unwrap_or_default();
                let copy = if card.favorite_copy { " (favorite copy)" } else { "" };
                format!("{}{size}{copy}", path.display())
            }
            _ => "none (history only)".into(),
        };
        rows.push(("transcript", transcript));
        rows.push(("resume", resume_text(s, &self.claude)));
        writeln!(self.out, "{}{}{}", self.pal.bold, s.id, self.pal.reset)?;
        for (label, value) in rows {
            writeln!(self.out, "  {:<14}{value}", format!("{label}:"))?;
        }
        Ok(())
    }

    fn card_json(&self, s: &SessionSummary, card: &Card) -> Value {
        let mut tools = Map::new();
        for (name, n) in tools_by_use(s) {
            tools.insert(name.to_string(), n.into());
        }
        let history = s.history_only();
        json!({
            "session_id": s.id,
            "title": title(s),
            "first_prompt": json_prompt(Some(&s.first_prompt)),
            "last_prompt": json_prompt(card.last_prompt.as_deref()),
            "project": project_name(s),
            "project_path": s.project_path,
            "cwd": s.cwd,
            "branch": s.git_branch.as_deref().filter(|b| !b.is_empty()),
            "kind": SessionKind::of(s).as_str(),
            "favorite": s.favorite,
            "started_at": s.started_at.and_then(rfc3339),
            "updated_at": rfc3339(s.updated_at),
            "turns": card.turns,
            "prompts": s.user_msg_count,
            "tools": tools,
            "files": files_touched(s),
            "cost_usd": s.cost_usd,
            "recorded_cost_usd": s.total_cost_usd,
            "tokens": {
                "input": s.tokens_in,
                "output": s.tokens_out,
                "cache_read": s.tokens_cache_read,
                "cache_write": s.tokens_cache_write,
                "hit_rate": s.cache_hit_ratio(),
            },
            "context": { "last": s.context_tokens, "peak": s.context_peak, "window": card.window },
            "main_model": primary_model(&s.model_usage),
            "models": models(s),
            "subagents": s.subagent_count,
            "transcript": s.file_path.as_ref().filter(|_| !history).map(|p| p.display().to_string()),
            "transcript_bytes": s.file_size.filter(|_| !history),
            "favorite_copy": card.favorite_copy,
            "resume": resume(s, &self.claude).ok(),
        })
    }
}

/// Calls per tool, most used first, then by name.
fn tools_by_use(s: &SessionSummary) -> Vec<(&str, u32)> {
    let mut tools: Vec<(&str, u32)> = s.tool_counts.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    tools.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    tools
}

fn tools_text(s: &SessionSummary) -> String {
    let tools = tools_by_use(s);
    if tools.is_empty() {
        return NONE.into();
    }
    let named: Vec<String> = tools.iter().take(TOOLS_SHOWN).map(|(name, n)| format!("{name} {n}")).collect();
    let mut text = named.join(" · ");
    if tools.len() > TOOLS_SHOWN {
        text.push_str(&format!(" (+{} more)", tools.len() - TOOLS_SHOWN));
    }
    text
}

fn files_text(s: &SessionSummary) -> String {
    let files = files_touched(s);
    if files.is_empty() {
        return NONE.into();
    }
    let mut text = format!("{} changed: {}", files.len(), files.iter().take(FILES_SHOWN).cloned().collect::<Vec<_>>().join(", "));
    if files.len() > FILES_SHOWN {
        text.push_str(&format!(" (+{} more)", files.len() - FILES_SHOWN));
    }
    text
}

fn cost_text(s: &SessionSummary) -> String {
    let mut text = s.cost_usd.map_or(NONE.to_string(), fmt_usd);
    if let Some(recorded) = s.total_cost_usd {
        text.push_str(&format!(" (Claude Code recorded {})", fmt_usd(recorded)));
    }
    text
}

/// Cache reads over all input, per mille rounded down, as the app's detail panel.
fn tokens_text(s: &SessionSummary) -> String {
    let total = s.input_tokens_total();
    if total == 0 && s.tokens_out == 0 {
        return NONE.into();
    }
    let t = fmt_tokens;
    let mut text = format!(
        "in {} · out {} · cache read {} · cache write {}",
        t(s.tokens_in),
        t(s.tokens_out),
        t(s.tokens_cache_read),
        t(s.tokens_cache_write)
    );
    if total > 0 {
        let pm = s.tokens_cache_read * 1000 / total;
        text.push_str(&format!(" · hit {}.{}%", pm / 10, pm % 10));
    }
    text
}

fn context_text(s: &SessionSummary, window: Option<u64>) -> String {
    let Some(last) = s.context_tokens else { return NONE.into() };
    let mut text = fmt_tokens(last);
    if let Some(window) = window.filter(|w| *w > 0) {
        let ratio = last as f64 / window as f64;
        let pct = if ratio < 0.01 { "<1%".to_string() } else { format!("{:.0}%", ratio * 100.0) };
        text.push_str(&format!(" / {} ({pct})", fmt_tokens(window)));
    }
    text.push_str(" at the end");
    if let Some(peak) = s.context_peak {
        text.push_str(&format!(" · peak {}", fmt_tokens(peak)));
    }
    text
}

/// Every model the session used, sorted, the main one included.
fn models(s: &SessionSummary) -> Vec<String> {
    let mut models: Vec<String> = s.models.iter().cloned().collect();
    models.extend(s.model_usage.iter().map(|u| u.model.clone()));
    models.sort();
    models.dedup();
    models
}

/// The main model first, marked, then the others.
fn models_text(s: &SessionSummary) -> String {
    let main = primary_model(&s.model_usage);
    let mut parts: Vec<String> = main.map(|m| format!("{m} (main)")).into_iter().collect();
    parts.extend(models(s).into_iter().filter(|m| Some(m.as_str()) != main));
    if parts.is_empty() { NONE.into() } else { parts.join(", ") }
}

/// `185 B`, `11.9 KB`, `2.1 MB`, `1.0 GB` (powers of 1024).
fn byte_size(bytes: u64) -> String {
    let mut n = bytes as f64;
    for unit in ["B", "KB", "MB"] {
        if n < 1024.0 {
            return if unit == "B" { format!("{bytes} B") } else { format!("{n:.1} {unit}") };
        }
        n /= 1024.0;
    }
    format!("{n:.1} GB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_sizes_use_powers_of_1024() {
        assert_eq!(byte_size(185), "185 B");
        assert_eq!(byte_size(12_186), "11.9 KB");
        assert_eq!(byte_size(2_202_009), "2.1 MB");
        assert_eq!(byte_size(1 << 30), "1.0 GB");
    }
}
