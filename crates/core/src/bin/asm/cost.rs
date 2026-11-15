//! `cost`: what sessions cost and how full their context got; `--as`, `--compare` and
//! `--windows` look at the same sessions from other angles.

use std::io::Write;

use agent_session_core::claude::model_catalog::ModelCatalog;
use agent_session_core::cost::{Comparison, compare, primary_model};
use agent_session_core::format::{fmt_change, fmt_datetime, fmt_tokens, fmt_usd};
use agent_session_core::index::ListFilter;
use agent_session_core::model::{SessionSummary, Tokens};
use agent_session_core::pricing::{CostLines, display_name, normalize, resolve};
use anyhow::{Result, anyhow};
use clap::{Args, ValueEnum};

use crate::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum CostSort {
    Cost,
    Context,
}

#[derive(Args)]
pub struct CostArgs {
    /// Project directory, or a substring of project directory names (default: all projects)
    #[arg(short = 'p', long)]
    project: Option<String>,
    /// Only sessions updated on or after this local date (YYYY-MM-DD)
    #[arg(long)]
    since: Option<chrono::NaiveDate>,
    /// Only sessions updated on or before this local date (YYYY-MM-DD)
    #[arg(long)]
    until: Option<chrono::NaiveDate>,
    #[arg(long, value_enum, default_value_t = CostSort::Cost)]
    sort: CostSort,
    #[arg(short = 'n', long, default_value_t = 30)]
    limit: usize,
    /// Compare with the cost Claude Code recorded itself (`cost-state`) instead of listing
    #[arg(long)]
    compare: bool,
    /// Compare catalog and built-in context windows, showing sessions whose CTX changes
    #[arg(long, conflicts_with = "compare")]
    windows: bool,
    /// Only sessions whose main model (the one that cost the most) is this, e.g. `opus-5-5`
    #[arg(long)]
    model: Option<String>,
    /// Also price each session with this as its main model, e.g. `fable-5-2`; the other models
    /// keep their price and the token counts stay the same
    #[arg(long = "as", value_name = "MODEL", conflicts_with_all = ["compare", "windows"])]
    as_model: Option<String>,
    #[arg(long)]
    no_scan: bool,
}

const COST_TITLE_LEN: usize = 60;

/// `opus-5-5` → `claude-opus-5-5`, without dropping a date or `[1m]` suffix.
fn claude_id(name: &str) -> String {
    if name.starts_with("claude-") { name.to_string() } else { format!("claude-{name}") }
}

/// Local midnight of `date`, unix ms.
fn local_midnight(date: chrono::NaiveDate) -> Option<i64> {
    use chrono::TimeZone;
    chrono::Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest().map(|dt| dt.timestamp_millis())
}

fn project_name(s: &SessionSummary) -> &str {
    let path = s.project_path.as_deref().or(s.cwd.as_deref()).unwrap_or(&s.project_key);
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// `Iterator::sum` of no floats is `-0.0`, which prints as `$-0.00`.
fn sum(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0, |a, b| a + b)
}

fn context_cell(s: &SessionSummary, catalog: &ModelCatalog) -> String {
    match s.context_ratio(catalog) {
        Some(r) if r < 0.01 => "<1%".into(),
        Some(r) => format!("{:.0}%", r * 100.0),
        None => "—".into(),
    }
}

impl App {
    pub(crate) fn cost(&mut self, args: CostArgs) -> Result<()> {
        let catalog = ModelCatalog::load(&self.claude);
        let db = self.open_db(!args.no_scan)?;
        let project_keys = match &args.project {
            Some(arg) => Some(self.resolve_project_keys(arg, &db)?),
            None => None,
        };
        let filter = ListFilter {
            project_keys,
            since_ms: args.since.and_then(local_midnight),
            until_ms: args.until.and_then(|d| d.succ_opt()).and_then(local_midnight),
            ..Default::default()
        };
        let mut sessions = db.list_sessions(&filter)?;
        if let Some(model) = &args.model {
            let want = claude_id(model);
            let want = normalize(&want);
            sessions.retain(|s| primary_model(&s.model_usage).is_some_and(|m| normalize(m) == want));
        }
        if args.compare {
            return self.compare_costs(&sessions, args.limit);
        }
        if args.windows {
            return self.compare_windows(&sessions, &catalog, args.limit);
        }
        let target = match &args.as_model {
            Some(name) => Some(resolve(name).ok_or_else(|| anyhow!("no price for model `{name}`"))?),
            None => None,
        };
        let key = |s: &SessionSummary| match args.sort {
            CostSort::Cost => s.cost_usd,
            CostSort::Context => s.context_ratio(&catalog),
        };
        sessions.sort_by(|a, b| {
            key(b).unwrap_or(f64::NEG_INFINITY).total_cmp(&key(a).unwrap_or(f64::NEG_INFINITY)).then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        if let Some(target) = target {
            return self.cost_as(&sessions, &target, &catalog, args.limit);
        }
        let (bold, dim, reset) = (self.pal.bold, self.pal.dim, self.pal.reset);
        writeln!(self.out, "{bold}{:>9} {:>9} {:>5}  {:<16}  {:<8}  {:<16}  TITLE{reset}", "COST", "CC", "CTX", "UPDATED", "SESSION", "PROJECT")?;
        let usd = |c: Option<f64>| c.map_or_else(|| "—".to_string(), fmt_usd);
        for s in sessions.iter().take(args.limit) {
            let id: String = if s.id_known { s.id.chars().take(8).collect() } else { "(none)".into() };
            let title = agent_session_core::claude::clean::one_line(s.display_title(), COST_TITLE_LEN);
            writeln!(
                self.out,
                "{:>9} {dim}{:>9}{reset} {:>5}  {}  {id:<8}  {:<16}  {title}",
                usd(s.cost_usd),
                usd(s.total_cost_usd),
                context_cell(s, &catalog),
                fmt_datetime(Some(s.updated_at)),
                project_name(s),
            )?;
        }
        let total = sum(sessions.iter().filter_map(|s| s.cost_usd));
        let cc = sum(sessions.iter().filter_map(|s| s.total_cost_usd));
        let shown = if sessions.len() > args.limit { format!(" (top {} shown)", args.limit) } else { String::new() };
        writeln!(self.out, "\n{bold}{} sessions · {}{reset} {dim}(CC {}){reset}{shown}", sessions.len(), fmt_usd(total), fmt_usd(cc))?;
        let tokens = |f: fn(&SessionSummary) -> u64| fmt_tokens(sessions.iter().map(f).sum());
        writeln!(
            self.out,
            "{dim}tokens: in {} · out {} · cache read {} · cache write {}{reset}",
            tokens(|s| s.tokens_in),
            tokens(|s| s.tokens_out),
            tokens(|s| s.tokens_cache_read),
            tokens(|s| s.tokens_cache_write)
        )?;
        let unpriced: std::collections::BTreeSet<&str> = sessions.iter().flat_map(|s| s.unpriced_models.iter().map(String::as_str)).collect();
        if !unpriced.is_empty() {
            writeln!(self.out, "{dim}not priced (unknown models): {}{reset}", unpriced.into_iter().collect::<Vec<_>>().join(", "))?;
        }
        Ok(())
    }

    /// `cost` with every session's main model priced as `target` as well: per session, then the
    /// totals by kind of token.
    fn cost_as(&mut self, sessions: &[SessionSummary], target: &str, catalog: &ModelCatalog, limit: usize) -> Result<()> {
        let (bold, dim, reset) = (self.pal.bold, self.pal.dim, self.pal.reset);
        let name = display_name(target);
        writeln!(
            self.out,
            "{bold}{:>9} {:>10} {:>7} {:>5}  {:<10}  {:<16}  {:<8}  {:<16}  TITLE{reset}",
            "COST", name.to_uppercase(), "CHANGE", "CTX", "MODEL", "UPDATED", "SESSION", "PROJECT"
        )?;
        let usd = |c: Option<f64>| c.map_or_else(|| "—".to_string(), fmt_usd);
        let comparisons: Vec<Option<Comparison>> = sessions.iter().map(|s| compare(&s.model_usage, target)).collect();
        for (s, c) in sessions.iter().zip(&comparisons).take(limit) {
            let id: String = if s.id_known { s.id.chars().take(8).collect() } else { "(none)".into() };
            let title = agent_session_core::claude::clean::one_line(s.display_title(), COST_TITLE_LEN);
            let (predicted, change, model) = match c {
                Some(c) => (Some(c.predicted_total()), c.actual_total().map_or_else(|| "—".into(), |a| fmt_change(a, c.predicted_total())), display_name(&c.source)),
                None => (None, "—".into(), "—".into()),
            };
            writeln!(
                self.out,
                "{:>9} {:>10} {:>7} {:>5}  {model:<10}  {}  {id:<8}  {:<16}  {title}",
                usd(s.cost_usd),
                usd(predicted),
                change,
                context_cell(s, catalog),
                fmt_datetime(Some(s.updated_at)),
                project_name(s),
            )?;
        }
        // totals over the sessions that can be compared: a main model with a price
        let compared: Vec<&Comparison> = comparisons.iter().flatten().filter(|c| c.actual.is_some()).collect();
        let (mut tokens, mut actual, mut predicted, mut others) = (Tokens::default(), CostLines::default(), CostLines::default(), 0.0);
        for c in &compared {
            tokens.add(&c.tokens);
            actual.add(&c.actual.unwrap_or_default());
            predicted.add(&c.predicted);
            others += c.others_usd;
        }
        let (total, total_as) = (actual.total() + others, predicted.total() + others);
        let shown = if sessions.len() > limit { format!(" (top {limit} shown)") } else { String::new() };
        writeln!(self.out, "\n{bold}{} sessions · {} → {name} {} ({}){reset}{shown}", compared.len(), fmt_usd(total), fmt_usd(total_as), fmt_change(total, total_as))?;
        writeln!(self.out, "{bold}{:<16} {:>8} {:>10} {:>10} {:>7}{reset}", "", "TOKENS", "ACTUAL", name.to_uppercase(), "CHANGE")?;
        let rows = [
            ("input", tokens.input, actual.input, predicted.input),
            ("cache write 5m", tokens.cache_write_5m, actual.cache_write_5m, predicted.cache_write_5m),
            ("cache write 1h", tokens.cache_write_1h, actual.cache_write_1h, predicted.cache_write_1h),
            ("cache read", tokens.cache_read, actual.cache_read, predicted.cache_read),
            ("input total", tokens.input_total(), actual.input_total(), predicted.input_total()),
            ("output", tokens.output, actual.output, predicted.output),
        ];
        for (label, n, a, p) in rows {
            writeln!(self.out, "{label:<16} {:>8} {:>10} {:>10} {:>7}", fmt_tokens(n), fmt_usd(a), fmt_usd(p), fmt_change(a, p))?;
        }
        if tokens.web_search > 0 {
            writeln!(self.out, "{:<16} {:>8} {:>10} {:>10}", "web search", tokens.web_search, fmt_usd(actual.web_search), fmt_usd(predicted.web_search))?;
        }
        writeln!(self.out, "{:<16} {:>8} {:>10} {:>10}", "other models", "", fmt_usd(others), fmt_usd(others))?;
        writeln!(self.out, "{bold}{:<16} {:>8} {:>10} {:>10} {:>7}{reset}", "total", "", fmt_usd(total), fmt_usd(total_as), fmt_change(total, total_as))?;
        writeln!(self.out, "{dim}main model of each session as {name}; other models keep their price; same token counts{reset}")?;
        let left_out = sessions.len() - compared.len();
        if left_out > 0 {
            writeln!(self.out, "{dim}{left_out} sessions left out: no API usage, or a main model without a price{reset}")?;
        }
        Ok(())
    }

    fn compare_windows(&mut self, sessions: &[SessionSummary], catalog: &ModelCatalog, limit: usize) -> Result<()> {
        use agent_session_core::claude::model_catalog::CatalogKind;
        use agent_session_core::pricing::builtin_window;
        use std::collections::BTreeMap;

        writeln!(self.out, "model catalog: {}", self.claude.model_catalog_dir().display())?;
        for file in catalog.files() {
            let name = file.path.file_name().unwrap_or_default().to_string_lossy();
            if let Some(error) = &file.error {
                writeln!(self.out, "  skipped    {name}  {error}")?;
            } else {
                let kind = match file.kind { CatalogKind::Served => "served", CatalogKind::Published => "published" };
                writeln!(self.out, "  {kind:<10} {name}  fetched {}  {} models", fmt_datetime(file.fetched_at), file.models)?;
            }
        }
        let builtin = ModelCatalog::default();
        let mut counts: BTreeMap<&str, (usize, usize)> = catalog.windows().keys().map(|id| (id.as_str(), (0, 0))).collect();
        let mut changed = Vec::new();
        let mut with_context = 0;
        for s in sessions {
            if let Some(model) = s.context_model.as_deref() {
                let count = counts.entry(normalize(model)).or_default();
                count.0 += 1;
                if context_cell(s, &builtin) != context_cell(s, catalog) {
                    count.1 += 1;
                    changed.push(s);
                }
            }
            if s.context_tokens.is_some() { with_context += 1; }
        }
        let mut models: Vec<_> = counts.into_iter().collect();
        models.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(b.0)));
        writeln!(self.out, "\n{:<32} {:>9} {:>9} {:>9} {:>9}", "MODEL", "CATALOG", "BUILT-IN", "SESSIONS", "CHANGED")?;
        let window = |w: Option<u64>| w.map_or_else(|| "—".into(), fmt_tokens);
        for (model, (count, differences)) in models {
            writeln!(self.out, "{model:<32} {:>9} {:>9} {count:>9} {differences:>9}", window(catalog.window(model)), window(builtin_window(model)))?;
        }
        writeln!(self.out, "\nsessions with context data: {with_context} · Ctx changed: {}", changed.len())?;
        changed.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
        for s in changed.into_iter().take(limit) {
            writeln!(self.out, "  {}  {}  {}  {} → {}", fmt_datetime(Some(s.updated_at)), s.id.chars().take(8).collect::<String>(),
                s.context_model.as_deref().unwrap_or("—"), context_cell(s, &builtin), context_cell(s, catalog))?;
        }
        Ok(())
    }

    /// How far the priced cost is from what Claude Code recorded, over the sessions where it
    /// recorded something.
    fn compare_costs(&mut self, sessions: &[SessionSummary], limit: usize) -> Result<()> {
        let pairs: Vec<(&SessionSummary, f64, f64)> =
            sessions.iter().filter_map(|s| Some((s, s.cost_usd.unwrap_or(0.0), s.total_cost_usd.filter(|c| *c > 0.0)?))).collect();
        let n = pairs.len();
        let off = |ours: f64, cc: f64| (ours - cc) / cc;
        let bucket = |lo: f64, hi: f64| pairs.iter().filter(|(_, o, c)| (lo..hi).contains(&off(*o, *c).abs())).count();
        let lower = pairs.iter().filter(|(_, o, c)| off(*o, *c) <= -0.10).count();
        let higher = pairs.iter().filter(|(_, o, c)| off(*o, *c) >= 0.10).count();
        let pct = |k: usize| if n == 0 { 0.0 } else { k as f64 * 100.0 / n as f64 };
        writeln!(self.out, "sessions with a cost recorded by Claude Code: {n}")?;
        for (label, k) in [("within 1%", bucket(0.0, 0.01)), ("1–10% off", bucket(0.01, 0.10)), (">10% lower", lower), (">10% higher", higher)] {
            writeln!(self.out, "  {label:<12} {k:>6}  ({:.1}%)", pct(k))?;
        }
        let ours = sum(pairs.iter().map(|(_, o, _)| *o));
        let cc = sum(pairs.iter().map(|(_, _, c)| *c));
        let diff = if cc > 0.0 { (ours - cc) / cc * 100.0 } else { 0.0 };
        writeln!(self.out, "total: {} priced here vs {} recorded ({diff:+.2}%)", fmt_usd(ours), fmt_usd(cc))?;
        let all = sum(sessions.iter().filter_map(|s| s.cost_usd));
        writeln!(self.out, "all {} sessions in range: {} priced here", sessions.len(), fmt_usd(all))?;
        let mut worst = pairs.clone();
        worst.sort_by(|a, b| (b.1 - b.2).abs().total_cmp(&(a.1 - a.2).abs()));
        if !worst.is_empty() && limit > 0 {
            writeln!(self.out, "\nlargest differences:")?;
        }
        for (s, o, c) in worst.iter().take(limit) {
            writeln!(self.out, "  {:>9} vs {:>9}  {:+6.1}%  {}  {}", fmt_usd(*o), fmt_usd(*c), off(*o, *c) * 100.0, &s.id[..s.id.len().min(8)], project_name(s))?;
        }
        Ok(())
    }
}
