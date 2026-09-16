//! Fake data until `crates/core` reads real sessions (PRD M1). Deterministic so screenshots
//! are stable between runs.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Local, TimeZone};
use gpui_kit::*;

use crate::model::{Conversation, Kind, Message, MessageKind, SessionRow};

pub const ROW_COUNT: usize = 10_000;

const PROJECTS: [&str; 12] = [
    "agent-session-manager",
    "scripts",
    "agent-sessions",
    "ideachat",
    "dotfiles",
    "blog",
    "herdr",
    "zed",
    "gpui-kit",
    "notes",
    "infra",
    "playground",
];
const BRANCHES: [&str; 6] = ["main", "master", "feat/table", "fix/parser", "wip", "develop"];
const PROMPTS: [&str; 8] = [
    "开始进行 PRD 的 M0：工具链准备和 gpui-kit spike",
    "Fix the JSONL parser so tool_result blocks with nested arrays are handled",
    "Why does the FTS5 trigram tokenizer miss two-character CJK queries?",
    "Refactor SessionStore so filtering runs on the main thread in under a frame",
    "帮我看看 history.jsonl 里没有 sessionId 的旧记录应该怎么分组",
    "Add a Kind column with SDK / BG badges and make it sortable",
    "Write BDD tests for asm list against the Python script output",
    "Set up cargo-bundle and produce a .app with an icon",
];
const LAST_PROMPTS: [&str; 6] = [
    "Make History sort after everything else, and the badge should be uppercase mono",
    "Can you also batch the sort so it never blocks input?",
    "Add a regression test with the sample from session 3f2a",
    "先把 gpui-kit 的 table 跑起来，别管样式",
    "那就按 cwd 归到 project，没有 cwd 的放 Unknown",
    "Run the whole suite once more and summarize failures",
];
const REPLIES: [&str; 6] = [
    "Here is the plan: first the delegate, then the store, then the toolbar.",
    "Filtering now runs in ~7ms for 10k rows; sorting is stable on Updated.",
    "The parser handles nested arrays; 14 tests pass including the new fixture.",
    "表格已能渲染 10k 行，滚动流畅。下一步接入真实数据。",
    "按 cwd 分组后剩 12 条无法归属，已标记为 Unknown。",
    "All 23 scenarios pass. Two were flaky on timing and now use a fixed clock.",
];

/// Tiny deterministic PRNG so the fake data is stable between runs.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

fn format_ts(ms: i64, full: bool) -> SharedString {
    let dt = Local.timestamp_millis_opt(ms).single().unwrap_or_else(Local::now);
    let fmt = if full { "%Y-%m-%d %H:%M" } else { "%m-%d %H:%M" };
    dt.format(fmt).to_string().into()
}

fn format_size(bytes: u64) -> SharedString {
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        format!("{kb:.0} KB").into()
    } else {
        format!("{:.1} MB", kb / 1024.0).into()
    }
}

pub fn generate_rows(n: usize) -> Vec<Arc<SessionRow>> {
    let mut rng = Lcg(0x5eed);
    let now = Local::now().timestamp_millis();
    let ninety_days: u64 = 90 * 24 * 3600 * 1000;
    (0..n)
        .map(|i| {
            let updated_at = now - rng.below(ninety_days) as i64;
            let started_at = updated_at - rng.below(6 * 3600 * 1000) as i64;
            let prompts = rng.below(60) as u32 + 1;
            let size = rng.below(30 * 1024 * 1024) + 2048;
            let kind = match rng.below(20) {
                0..=13 => Kind::Cli,
                14..=16 => Kind::Sdk,
                17..=18 => Kind::Background,
                _ => Kind::History,
            };
            let (tools, cost) = if kind == Kind::History {
                (0, 0.0)
            } else {
                (rng.below(400) as u32, rng.below(4000) as f64 / 100.0)
            };
            let prompt = rng.pick(&PROMPTS);
            let last_prompt = rng.pick(&LAST_PROMPTS);
            let reply = rng.pick(&REPLIES);
            let uuid = format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                rng.next() as u32,
                rng.below(0xffff),
                rng.below(0xfff),
                rng.below(0xffff) | 0x8000,
                rng.next() & 0xffff_ffff_ffff
            );
            Arc::new(SessionRow {
                num: i,
                num_fmt: format!("#{i}").into(),
                short_id: uuid[..8].to_string().into(),
                uuid: uuid.into(),
                project: rng.pick(&PROJECTS).into(),
                title: prompt.into(),
                first_prompt: prompt.into(),
                last_prompt: last_prompt.into(),
                last_message: reply.into(),
                started_at,
                updated_at,
                started_fmt: format_ts(started_at, false),
                updated_fmt: format_ts(updated_at, false),
                started_full: format_ts(started_at, true),
                updated_full: format_ts(updated_at, true),
                prompts,
                prompts_fmt: prompts.to_string().into(),
                size,
                size_fmt: format_size(size),
                cost,
                cost_fmt: if cost > 0.0 { format!("${cost:.2}").into() } else { "—".into() },
                branch: rng.pick(&BRANCHES).into(),
                tools,
                tools_fmt: tools.to_string().into(),
                kind,
            })
        })
        .collect()
}

/// Distinct projects with their session counts, most sessions first.
pub fn project_counts(rows: &[Arc<SessionRow>]) -> Vec<(SharedString, usize)> {
    let mut counts: HashMap<SharedString, usize> = HashMap::new();
    for row in rows {
        *counts.entry(row.project.clone()).or_default() += 1;
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn msg(kind: MessageKind, time: &str, text: &str) -> Message {
    Message {
        kind,
        time: time.into(),
        text: text.into(),
        name: "".into(),
        arg: "".into(),
        result: "".into(),
        input: "".into(),
        output: "".into(),
        more: "".into(),
        meta: "".into(),
        preview: "".into(),
    }
}

fn thinking(time: &str, chars: u32, text: &str) -> Message {
    let meta = if chars >= 1000 { format!("{:.1}k chars", chars as f64 / 1000.0) } else { format!("{chars} chars") };
    let preview: String = text.chars().take(60).collect();
    let mut m = msg(MessageKind::Thinking, time, text);
    m.meta = meta.into();
    m.preview = format!("{preview}…").into();
    m
}

fn tool(time: &str, name: &str, arg: &str, result: &str, input: &str, output: &str, more: &str) -> Message {
    let mut m = msg(MessageKind::Tool, time, "");
    m.name = name.into();
    m.arg = arg.into();
    m.result = result.into();
    m.input = input.into();
    m.output = output.into();
    m.more = more.into();
    m
}

/// A fixed transcript shaped like a real Claude Code session, with the session's own
/// first prompt, last prompt and last reply spliced in.
pub fn conversation_for(row: &SessionRow) -> Conversation {
    use MessageKind::*;
    let mut list = vec![
        msg(User, "15:05", &row.first_prompt),
        thinking("15:05", 1180, "The column set lives in an enum in table.rs; sorting is driven by a SortKey per column. Kind needs a total order, so History should probably sort last since those rows have no live record. Badges render inline in the cell — I should check how the Prompts cell renders numbers so the badge aligns with the same baseline."),
        msg(Assistant, "15:05", "I'll look at how columns and sort keys are defined before touching anything."),
        tool("15:05", "Read", "src/ui/table.rs", "212 lines", "file_path: src/ui/table.rs", "use gpui::*;\nuse crate::model::{Session, SortDir};\n\npub enum SessionColumn {", "… 208 more lines"),
        tool("15:06", "Grep", "SessionColumn:: -r src/", "9 matches · 4 files", "pattern: SessionColumn::\npath: src/", "src/ui/table.rs:41\nsrc/ui/table.rs:88\nsrc/ui/header.rs:17", "… 6 more matches"),
        tool("15:06", "Read", "src/model/session.rs", "140 lines", "file_path: src/model/session.rs", "#[derive(Clone, Debug)]\npub struct Session {\n    pub id: Uuid,\n    pub project: Arc<str>,", "… 136 more lines"),
        msg(Assistant, "15:07", "Plan:\n1. Add `SessionKind { Cli, Sdk, Bg, History }` to the model with an explicit `Ord` so sort is stable.\n2. Add `SessionColumn::Kind` and a `kind_badge()` renderer — SDK filled blue, BG filled gray, History outlined.\n3. Wire the header sort key and add a fixture for each kind."),
        tool("15:08", "Edit", "src/model/session.rs", "+18 −2", "old: pub struct Session {\nnew: #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]\npub enum SessionKind { Cli, Sdk, Bg, History }\n\npub struct Session {", "The file src/model/session.rs has been updated.", ""),
        tool("15:09", "Edit", "src/ui/table.rs", "+64 −5", "old: SessionColumn::Size => …\nnew: SessionColumn::Size => …\nSessionColumn::Kind => kind_badge(session.kind),", "The file src/ui/table.rs has been updated.", ""),
        tool("15:10", "Bash", "cargo check -p asm-ui", "exit 0 · 2.3s", "cargo check -p asm-ui", "    Checking asm-ui v0.1.0\n    Finished dev [unoptimized] target(s) in 2.31s", ""),
        msg(Assistant, "15:10", "Kind column added after Size and sortable via the header. SDK renders as a filled blue badge, BG as gray, History as an outlined badge; CLI rows show nothing so the column stays quiet."),
        msg(User, "15:41", "Make History sort after everything else, and the badge should be uppercase mono"),
        thinking("15:41", 420, "History is already last in the enum order, but the default direction for a fresh click is ascending, so it appears first when the user sorts descending. Flip the default direction for this column. The badge font is inherited — switch to the mono face used by the ID column."),
        tool("15:42", "Edit", "src/ui/table.rs", "+7 −3", "old: SessionColumn::Kind => SortDir::Desc\nnew: SessionColumn::Kind => SortDir::Asc", "The file src/ui/table.rs has been updated.", ""),
        tool("15:43", "Bash", "cargo test -p asm-ui -- kind", "14 passed · 4.1s", "cargo test -p asm-ui -- kind", "running 14 tests\ntest kind::sort_history_last ... ok\ntest kind::badge_uppercase ... ok", "… 12 more lines"),
        msg(Assistant, "15:44", "Done. Order is now CLI → SDK → BG → History on first click; the badge uses the mono face at 9px uppercase with 0.3px tracking to match the ID column."),
        msg(User, "17:50", &row.last_prompt),
        thinking("17:50", 960, "Three places touch Kind now: the delegate that renders the cell, the store that filters, and the toolbar control. Sequence matters — the store filter is a no-op until the delegate exposes the enum, and the toolbar only needs the store API."),
        msg(Assistant, "17:53", &row.last_message),
    ];
    if row.kind == Kind::History {
        list.retain(|m| m.kind == User);
    }
    list.into_iter().map(Arc::new).collect()
}
