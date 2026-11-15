//! The SQLite index: one connection, owned by whoever indexes or queries (never shared across
//! threads). Schema in `schema.sql`; a version mismatch or an unreadable file rebuilds it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, TransactionBehavior, params};

use crate::model::{MessageRow, ModelUsage, Role, SessionSummary, Source, TextKind, Tokens, UsageRow};

/// Also bump this when a price in `pricing.rs` changes: stored costs are computed at index time.
pub const SCHEMA_VERSION: i32 = 5;
/// rusqlite's own default, kept for `Db::open`.
const DEFAULT_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const SCHEMA: &str = include_str!("schema.sql");

pub struct Db {
    conn: Connection,
    path: Option<PathBuf>,
}

/// True when `error` comes from SQLite finding the database locked by another connection
/// (`SQLITE_BUSY` / `SQLITE_LOCKED`), e.g. the CLI scanning while the app writes the index.
pub fn is_busy(error: &anyhow::Error) -> bool {
    has_sqlite_code(error, busy_code)
}

fn busy_code(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
}

/// The file is not an SQLite database, or a damaged one.
fn is_corrupt(error: &anyhow::Error) -> bool {
    has_sqlite_code(error, |code| matches!(code, ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt))
}

fn has_sqlite_code(error: &anyhow::Error, matches: impl Fn(ErrorCode) -> bool) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<rusqlite::Error>().and_then(rusqlite::Error::sqlite_error_code).is_some_and(&matches))
}

/// Per-file bookkeeping for incremental scans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexState {
    pub session_id: String,
    /// Empty for a session transcript, else the subagent file stem (`session_id` is its parent).
    pub agent: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub inode: Option<u64>,
    /// Bytes parsed so far (always at a line boundary).
    pub indexed_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectInfo {
    pub key: String,
    pub path: Option<String>,
    pub session_count: usize,
    pub last_updated: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub sessions: usize,
    pub history_only: usize,
    /// Text messages as counted before the index had kinds: no thinking rows, no rows of user
    /// records without text (images, documents). A user record with empty text is left out as
    /// well, which the old count included; real transcripts have none.
    pub messages: usize,
    pub projects: usize,
    pub files: usize,
    pub db_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// The reference script's list order: first prompt time, newest first, no-prompt last.
    #[default]
    StartedDesc,
    UpdatedDesc,
}

#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    pub favorites_only: bool,
    pub project_keys: Option<Vec<String>>,
    /// Keep sessions with `updated_at >= since_ms`.
    pub since_ms: Option<i64>,
    /// Keep sessions with `updated_at < until_ms`.
    pub until_ms: Option<i64>,
    pub order: Order,
}

/// What `stats` counts as messages: no thinking rows, no user rows without text (records with only
/// images), as before text kinds were indexed. The expression is the one the index
/// `messages_counted` (schema.sql) stores, written the same way, so counting searches that index
/// only: `stats` runs before the app reports a finished scan, and user rows can carry long text in
/// overflow pages. A partial index with this condition is not enough: SQLite would still read the
/// rows to test it again.
const COUNT_MESSAGES: &str =
    "SELECT count(*) FROM messages WHERE (kind <> 'thinking' AND NOT (role = 'user' AND text = '')) = 1";

const SESSION_COLUMNS: &str = "s.id, s.id_known, p.encoded_dir, p.path, s.file_path, s.source, s.transcript_missing, \
     s.title, s.title_rank, s.first_prompt, s.last_prompt, s.last_message, s.started_at, s.updated_at, \
     s.git_branch, s.cwd, s.cc_version, s.entrypoint, s.permission_mode, s.is_background, \
     s.user_msg_count, s.assistant_msg_count, s.tool_use_count, s.subagent_count, s.file_size, \
     s.total_cost_usd, s.duration_ms, s.lines_added, s.lines_removed, s.pr_url, s.models, \
     s.cost_usd, s.tokens_in, s.tokens_out, s.tokens_cache_read, s.tokens_cache_write, s.unpriced_models, \
     s.context_tokens, s.context_peak, s.context_model, s.favorite";

impl Db {
    /// Open (or create) the index at `path`. A file that is not an SQLite database, or a damaged
    /// one, is deleted and recreated; an index of another schema version is emptied and rebuilt,
    /// so callers must rescan afterwards. Connections that open an index needing a build at the
    /// same time build it once: the first to take the write lock builds it, the others wait and
    /// find it built. One that waits longer than the busy timeout fails as busy and changes nothing.
    pub fn open(path: &Path) -> Result<Db> {
        Self::open_with(path, DEFAULT_BUSY_TIMEOUT)
    }

    /// `open`, waiting at most `busy_timeout` for another connection's lock, from the schema
    /// check on.
    pub fn open_with(path: &Path, busy_timeout: std::time::Duration) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        match Self::open_checked(path, busy_timeout) {
            // Nobody can use such a file. Every other failure (another process holding the lock,
            // a full disk, permissions) leaves the file alone: another process may be using it.
            Err(e) if is_corrupt(&e) => {
                remove_db_files(path);
                Self::open_checked(path, busy_timeout)
            }
            result => result,
        }
    }

    /// A second, query-only connection to an index another thread maintains (WAL lets it read
    /// while that thread writes). Never creates, repairs or migrates anything. The file itself is
    /// opened read-write: a `SQLITE_OPEN_READ_ONLY` connection cannot open a WAL database whose
    /// `-shm` file is gone (the last writer closed cleanly).
    pub fn open_read_only(path: &Path) -> Result<Db> {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;
             PRAGMA cache_size = -65536;",
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(Db { conn, path: Some(path.to_path_buf()) })
    }

    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        let mut db = Db { conn, path: None };
        db.configure()?;
        db.ensure_schema()?;
        Ok(db)
    }

    fn open_checked(path: &Path, busy_timeout: std::time::Duration) -> Result<Db> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.busy_timeout(busy_timeout)?;
        let mut db = Db { conn, path: Some(path.to_path_buf()) };
        db.use_wal(busy_timeout).with_context(|| format!("{}: another connection kept it locked", path.display()))?;
        db.configure()?;
        db.ensure_schema()?;
        Ok(db)
    }

    /// Switch the file to WAL (nothing to do once it is). Connections opening a new file at the
    /// same moment all try to; SQLite fails those that lose as busy at once instead of waiting
    /// (they would deadlock), so they wait here, as long as the busy timeout.
    fn use_wal(&self, busy_timeout: std::time::Duration) -> Result<()> {
        let started = std::time::Instant::now();
        loop {
            match self.conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
                Err(e) if e.sqlite_error_code().is_some_and(busy_code) && started.elapsed() < busy_timeout => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                result => return Ok(result.map(drop)?),
            }
        }
    }

    fn configure(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;
             PRAGMA cache_size = -65536;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(())
    }

    /// Keep the index only if both its version and its schema are this build's. The version covers
    /// what the schema can't show (prices, parsing); comparing the schema catches a development
    /// build that stamped the current version on an older one.
    ///
    /// Several connections may find the index needs building at once (two `asm` started together
    /// on a new index, or the app and `asm` after a schema change). The build happens in one
    /// write transaction that looks again once it holds the lock, so the first one builds and the
    /// others find its build instead of dropping it halfway.
    fn ensure_schema(&mut self) -> Result<()> {
        let Some(why) = schema_mismatch(&self.conn)? else { return Ok(()) };
        let path = self.path.as_deref().map_or_else(|| ":memory:".into(), |p| p.display().to_string());
        // dropping tables with foreign keys on deletes their rows one by one; the pragma is a
        // no-op inside a transaction
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .with_context(|| format!("{path} {why}, but another connection is writing it"))?;
        if schema_mismatch(&tx)?.is_some() {
            reset_schema(&tx)?;
        }
        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    /// Delete every row but keep the schema (rebuild).
    pub fn clear_all(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM messages; DELETE FROM session_files; DELETE FROM session_tool_counts;
             DELETE FROM sessions; DELETE FROM projects; DELETE FROM index_state; DELETE FROM api_usage; DELETE FROM meta;
             DELETE FROM session_model_usage;
             INSERT INTO messages_fts(messages_fts) VALUES ('rebuild');",
        )?;
        Ok(())
    }

    pub fn set_user_version_for_test(&self, version: i32) {
        self.conn.execute_batch(&format!("PRAGMA user_version = {version};")).unwrap();
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO meta VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// How long a statement waits for another connection's lock before failing as busy
    /// (rusqlite's default is 5 s).
    pub fn set_busy_timeout(&self, timeout: std::time::Duration) -> Result<()> {
        self.conn.busy_timeout(timeout)?;
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run `f` inside one transaction.
    pub fn transaction<T>(&self, f: impl FnOnce(&Db) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(self)?;
        tx.commit()?;
        Ok(out)
    }

    // ---- projects -------------------------------------------------------------------------

    /// Insert or update a project; a known real path is never overwritten by `None`.
    pub fn upsert_project(&self, key: &str, path: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO projects (encoded_dir, path) VALUES (?1, ?2)
             ON CONFLICT(encoded_dir) DO UPDATE SET path = COALESCE(excluded.path, projects.path)",
            params![key, path],
        )?;
        Ok(self.conn.query_row("SELECT id FROM projects WHERE encoded_dir = ?1", [key], |r| r.get(0))?)
    }

    pub fn project_keys(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT encoded_dir FROM projects ORDER BY encoded_dir")?;
        let keys = stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        Ok(keys)
    }

    /// Projects with their session count, most recently active first.
    pub fn projects(&self) -> Result<Vec<ProjectInfo>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT p.encoded_dir, p.path, count(s.id), coalesce(max(s.updated_at), 0)
             FROM projects p LEFT JOIN sessions s ON s.project_id = p.id
             GROUP BY p.id ORDER BY 4 DESC, 1",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ProjectInfo {
                    key: r.get(0)?,
                    path: r.get(1)?,
                    session_count: r.get::<_, i64>(2)? as usize,
                    last_updated: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    // ---- sessions -------------------------------------------------------------------------

    pub fn upsert_session(&self, s: &SessionSummary, project_id: i64) -> Result<()> {
        let models = serde_json::to_string(&s.models)?;
        self.conn.execute(
            "INSERT INTO sessions (id, id_known, project_id, file_path, source, transcript_missing, title, title_rank,
                first_prompt, last_prompt, last_message, started_at, updated_at, git_branch, cwd, cc_version,
                entrypoint, permission_mode, is_background, user_msg_count, assistant_msg_count, tool_use_count,
                subagent_count, file_size, total_cost_usd, duration_ms, lines_added, lines_removed, pr_url, models,
                context_tokens, context_peak, context_model, favorite)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34)
             ON CONFLICT(id) DO UPDATE SET
                id_known = excluded.id_known, project_id = excluded.project_id, file_path = excluded.file_path,
                source = excluded.source, transcript_missing = excluded.transcript_missing, title = excluded.title,
                title_rank = excluded.title_rank, first_prompt = excluded.first_prompt, last_prompt = excluded.last_prompt,
                last_message = excluded.last_message, started_at = excluded.started_at, updated_at = excluded.updated_at,
                git_branch = excluded.git_branch, cwd = excluded.cwd, cc_version = excluded.cc_version,
                entrypoint = excluded.entrypoint, permission_mode = excluded.permission_mode,
                is_background = excluded.is_background, user_msg_count = excluded.user_msg_count,
                assistant_msg_count = excluded.assistant_msg_count, tool_use_count = excluded.tool_use_count,
                subagent_count = excluded.subagent_count, file_size = excluded.file_size,
                total_cost_usd = excluded.total_cost_usd, duration_ms = excluded.duration_ms,
                lines_added = excluded.lines_added, lines_removed = excluded.lines_removed, pr_url = excluded.pr_url,
                models = excluded.models, context_tokens = excluded.context_tokens,
                context_peak = excluded.context_peak, context_model = excluded.context_model, favorite = excluded.favorite",
            params![
                s.id,
                s.id_known,
                project_id,
                s.file_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                s.source.as_str(),
                s.transcript_missing,
                s.title,
                s.title_rank,
                s.first_prompt,
                s.last_prompt,
                s.last_message,
                s.started_at,
                s.updated_at,
                s.git_branch,
                s.cwd,
                s.cc_version,
                s.entrypoint,
                s.permission_mode,
                s.is_background,
                s.user_msg_count,
                s.assistant_msg_count,
                s.tool_use_count,
                s.subagent_count,
                s.file_size.map(|n| n as i64),
                s.total_cost_usd,
                s.duration_ms,
                s.lines_added,
                s.lines_removed,
                s.pr_url,
                models,
                s.context_tokens.map(|n| n as i64),
                s.context_peak.map(|n| n as i64),
                s.context_model,
                s.favorite,
            ],
        )?;
        self.conn.execute("DELETE FROM session_tool_counts WHERE session_id = ?1", [&s.id])?;
        let mut ins = self.conn.prepare_cached("INSERT INTO session_tool_counts (session_id, tool_name, count) VALUES (?1, ?2, ?3)")?;
        for (name, n) in &s.tool_counts {
            ins.execute(params![s.id, name, n])?;
        }
        self.conn.execute("DELETE FROM session_files WHERE session_id = ?1", [&s.id])?;
        let mut ins = self.conn.prepare_cached("INSERT INTO session_files (session_id, path) VALUES (?1, ?2)")?;
        for path in &s.files_touched {
            ins.execute(params![s.id, path])?;
        }
        Ok(())
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionSummary>> {
        let sql = format!("SELECT {SESSION_COLUMNS} FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.id = ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let Some(mut s) = stmt.query_row([id], row_to_session).optional()? else { return Ok(None) };
        let mut tools = self.conn.prepare_cached("SELECT tool_name, count FROM session_tool_counts WHERE session_id = ?1")?;
        for row in tools.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))? {
            let (name, n) = row?;
            s.tool_counts.insert(name, n);
        }
        let mut files = self.conn.prepare_cached("SELECT path FROM session_files WHERE session_id = ?1")?;
        for row in files.query_map([id], |r| r.get::<_, String>(0))? {
            s.files_touched.insert(row?);
        }
        let mut usage = self.conn.prepare_cached(&model_usage_sql("WHERE session_id = ?1"))?;
        for row in usage.query_map([id], row_to_model_usage)? {
            s.model_usage.push(row?.1);
        }
        Ok(Some(s))
    }

    /// `id -> source` for every session (cheap; used to decide what history may overwrite).
    pub fn session_sources(&self) -> Result<HashMap<String, Source>> {
        let mut stmt = self.conn.prepare_cached("SELECT id, source FROM sessions")?;
        let mut out = HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, source) = row?;
            out.insert(id, Source::parse(&source));
        }
        Ok(out)
    }

    pub fn list_sessions(&self, filter: &ListFilter) -> Result<Vec<SessionSummary>> {
        let mut sql = format!("SELECT {SESSION_COLUMNS} FROM sessions s JOIN projects p ON p.id = s.project_id WHERE 1 = 1");
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if filter.favorites_only { sql.push_str(" AND s.favorite = 1"); }
        if let Some(keys) = &filter.project_keys {
            let marks: Vec<String> = keys.iter().enumerate().map(|(i, _)| format!("?{}", args.len() + i + 1)).collect();
            sql.push_str(&format!(" AND p.encoded_dir IN ({})", marks.join(",")));
            args.extend(keys.iter().map(|k| k.clone().into()));
        }
        if let Some(since) = filter.since_ms {
            args.push(since.into());
            sql.push_str(&format!(" AND s.updated_at >= ?{}", args.len()));
        }
        if let Some(until) = filter.until_ms {
            args.push(until.into());
            sql.push_str(&format!(" AND s.updated_at < ?{}", args.len()));
        }
        sql.push_str(match filter.order {
            Order::StartedDesc => " ORDER BY s.started_at IS NULL, s.started_at DESC, s.updated_at DESC, s.id",
            Order::UpdatedDesc => " ORDER BY s.updated_at DESC, s.id",
        });
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut sessions: Vec<SessionSummary> =
            stmt.query_map(rusqlite::params_from_iter(args), row_to_session)?.collect::<std::result::Result<_, _>>()?;
        self.attach_details(&mut sessions)?;
        Ok(sessions)
    }

    /// Sessions whose id starts with `prefix`, newest started first. Anonymous history sessions
    /// only match the empty prefix (they have no id), like the reference script.
    pub fn find_by_prefix(&self, prefix: &str) -> Result<Vec<SessionSummary>> {
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions s JOIN projects p ON p.id = s.project_id
             WHERE (s.id_known = 1 AND substr(s.id, 1, ?2) = ?1) OR (s.id_known = 0 AND ?1 = '')
             ORDER BY s.started_at IS NULL, s.started_at DESC, s.updated_at DESC, s.id"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut sessions: Vec<SessionSummary> = stmt
            .query_map(params![prefix, prefix.len() as i64], row_to_session)?
            .collect::<std::result::Result<_, _>>()?;
        self.attach_details(&mut sessions)?;
        Ok(sessions)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        self.delete_usage(id, None)?;
        self.conn.execute("DELETE FROM session_model_usage WHERE session_id = ?1", [id])?;
        Ok(())
    }

    pub fn mark_transcript_missing(&self, id: &str) -> Result<()> {
        self.conn.execute("UPDATE sessions SET favorite = 0, transcript_missing = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Fill `tool_counts` / `files_touched` for the given sessions with two bulk queries.
    fn attach_details(&self, sessions: &mut [SessionSummary]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let mut by_id: HashMap<&str, usize> = sessions.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        let mut tools: HashMap<usize, BTreeMap<String, u32>> = HashMap::new();
        let mut files: HashMap<usize, BTreeSet<String>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached("SELECT session_id, tool_name, count FROM session_tool_counts")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?)))? {
            let (sid, name, n) = row?;
            if let Some(&ix) = by_id.get(sid.as_str()) {
                tools.entry(ix).or_default().insert(name, n);
            }
        }
        let mut stmt = self.conn.prepare_cached("SELECT session_id, path FROM session_files")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (sid, path) = row?;
            if let Some(&ix) = by_id.get(sid.as_str()) {
                files.entry(ix).or_default().insert(path);
            }
        }
        let mut usage: HashMap<usize, Vec<ModelUsage>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(&model_usage_sql(""))?;
        for row in stmt.query_map([], row_to_model_usage)? {
            let (sid, group) = row?;
            if let Some(&ix) = by_id.get(sid.as_str()) {
                usage.entry(ix).or_default().push(group);
            }
        }
        by_id.clear();
        for (ix, s) in sessions.iter_mut().enumerate() {
            s.tool_counts = tools.remove(&ix).unwrap_or_default();
            s.files_touched = files.remove(&ix).unwrap_or_default();
            s.model_usage = usage.remove(&ix).unwrap_or_default();
        }
        Ok(())
    }

    // ---- messages -------------------------------------------------------------------------

    pub fn insert_messages(&self, session_id: &str, messages: &[MessageRow]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO messages (session_id, uuid, parent_uuid, role, kind, ts, text, is_meta, byte_offset, byte_len, model, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )?;
        for m in messages {
            stmt.execute(params![
                session_id,
                m.uuid,
                m.parent_uuid,
                m.role.as_str(),
                m.kind.as_str(),
                m.ts,
                m.text,
                m.is_meta,
                m.byte_offset as i64,
                m.byte_len as i64,
                m.model,
                m.input_tokens,
                m.output_tokens
            ])?;
        }
        Ok(())
    }

    /// uuids of a session's indexed user rows, for a parser that appends to its transcript.
    pub fn user_uuids(&self, session_id: &str) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT uuid FROM messages WHERE session_id = ?1 AND role = 'user' AND uuid <> ''")?;
        let uuids = stmt.query_map([session_id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        Ok(uuids)
    }

    pub fn delete_messages(&self, session_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
        Ok(())
    }

    /// Messages of one session in file order (the rows of one line in the order they were written).
    pub fn messages(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT uuid, parent_uuid, role, kind, ts, text, is_meta, byte_offset, byte_len, model, input_tokens, output_tokens
             FROM messages WHERE session_id = ?1 ORDER BY byte_offset, id",
        )?;
        let rows = stmt
            .query_map([session_id], |r| {
                Ok(MessageRow {
                    uuid: r.get(0)?,
                    parent_uuid: r.get(1)?,
                    role: Role::parse(&r.get::<_, String>(2)?).unwrap_or(Role::User),
                    kind: TextKind::parse(&r.get::<_, String>(3)?).unwrap_or(TextKind::Other),
                    ts: r.get(4)?,
                    text: r.get(5)?,
                    is_meta: r.get(6)?,
                    byte_offset: r.get::<_, i64>(7)? as u64,
                    byte_len: r.get::<_, i64>(8)? as u64,
                    model: r.get(9)?,
                    input_tokens: r.get(10)?,
                    output_tokens: r.get(11)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    // ---- api_usage ------------------------------------------------------------------------

    /// Add API responses of a session's transcript (`agent` = "") or of one of its subagents.
    /// A response already stored under the same id is replaced, keeping its earliest timestamp.
    pub fn insert_usage(&self, session_id: &str, agent: &str, rows: &[UsageRow]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO api_usage (session_id, agent, msg_id, model, ts, input, cache_write_5m, cache_write_1h,
                cache_read, output, web_search, fast, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(session_id, agent, msg_id) DO UPDATE SET
                model = excluded.model, ts = min(api_usage.ts, excluded.ts), input = excluded.input,
                cache_write_5m = excluded.cache_write_5m, cache_write_1h = excluded.cache_write_1h,
                cache_read = excluded.cache_read, output = excluded.output, web_search = excluded.web_search,
                fast = excluded.fast, cost_usd = excluded.cost_usd",
        )?;
        for u in rows {
            stmt.execute(params![
                session_id,
                agent,
                u.msg_id,
                u.model,
                u.ts,
                u.input as i64,
                u.cache_write_5m as i64,
                u.cache_write_1h as i64,
                u.cache_read as i64,
                u.output as i64,
                u.web_search as i64,
                u.fast,
                u.cost_usd,
            ])?;
        }
        Ok(())
    }

    /// Drop a session's usage: all of it, or only what one file (`agent`) contributed.
    pub fn delete_usage(&self, session_id: &str, agent: Option<&str>) -> Result<()> {
        match agent {
            Some(agent) => self.conn.execute("DELETE FROM api_usage WHERE session_id = ?1 AND agent = ?2", params![session_id, agent])?,
            None => self.conn.execute("DELETE FROM api_usage WHERE session_id = ?1", [session_id])?,
        };
        Ok(())
    }

    /// Recompute a session's cost and token totals, and its usage by model, from its stored API
    /// responses. The groups match `SessionSummary::set_usage`.
    pub fn refresh_session_usage(&self, session_id: &str) -> Result<()> {
        self.conn.prepare_cached("DELETE FROM session_model_usage WHERE session_id = ?1")?.execute([session_id])?;
        let groups = format!(
            "INSERT INTO session_model_usage
             SELECT session_id, model, fast, input + cache_write_5m + cache_write_1h + cache_read > {} AS long_prompt,
                count(*), sum(input), sum(cache_write_5m), sum(cache_write_1h), sum(cache_read), sum(output), sum(web_search), sum(cost_usd)
             FROM api_usage WHERE session_id = ?1 GROUP BY model, fast, long_prompt",
            crate::pricing::LONG_PROMPT
        );
        self.conn.prepare_cached(&groups)?.execute([session_id])?;
        let mut stmt = self.conn.prepare_cached(
            "UPDATE sessions SET (cost_usd, tokens_in, tokens_out, tokens_cache_read, tokens_cache_write) = (
                SELECT sum(cost_usd), coalesce(sum(input), 0), coalesce(sum(output), 0), coalesce(sum(cache_read), 0),
                    coalesce(sum(cache_write_5m + cache_write_1h), 0)
                FROM api_usage WHERE session_id = ?1),
             unpriced_models = (SELECT json_group_array(model) FROM
                (SELECT DISTINCT model FROM api_usage WHERE session_id = ?1 AND cost_usd IS NULL ORDER BY model))
             WHERE id = ?1",
        )?;
        stmt.execute([session_id])?;
        Ok(())
    }

    /// Every stored API response of a session, in time order.
    pub fn usage(&self, session_id: &str) -> Result<Vec<UsageRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT msg_id, model, ts, input, cache_write_5m, cache_write_1h, cache_read, output, web_search, fast, cost_usd
             FROM api_usage WHERE session_id = ?1 ORDER BY ts, agent, msg_id",
        )?;
        let rows = stmt
            .query_map([session_id], |r| {
                Ok(UsageRow {
                    msg_id: r.get(0)?,
                    model: r.get(1)?,
                    ts: r.get(2)?,
                    input: r.get::<_, i64>(3)? as u64,
                    cache_write_5m: r.get::<_, i64>(4)? as u64,
                    cache_write_1h: r.get::<_, i64>(5)? as u64,
                    cache_read: r.get::<_, i64>(6)? as u64,
                    output: r.get::<_, i64>(7)? as u64,
                    web_search: r.get::<_, i64>(8)? as u64,
                    fast: r.get(9)?,
                    cost_usd: r.get(10)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    // ---- index_state ----------------------------------------------------------------------

    pub fn index_state(&self, path: &Path) -> Result<Option<IndexState>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT session_id, agent, size, mtime_ns, inode, indexed_offset FROM index_state WHERE path = ?1",
        )?;
        Ok(stmt.query_row([path.to_string_lossy()], row_to_state).optional()?)
    }

    pub fn all_index_states(&self) -> Result<HashMap<PathBuf, IndexState>> {
        let mut stmt = self.conn.prepare_cached("SELECT path, session_id, agent, size, mtime_ns, inode, indexed_offset FROM index_state")?;
        let mut out = HashMap::new();
        for row in stmt.query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), row_to_state_at(r, 1)?)))? {
            let (path, state) = row?;
            out.insert(path, state);
        }
        Ok(out)
    }

    pub fn set_index_state(&self, path: &Path, state: &IndexState) -> Result<()> {
        self.conn.execute(
            "INSERT INTO index_state (path, session_id, agent, size, mtime_ns, inode, indexed_offset, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET session_id = excluded.session_id, agent = excluded.agent, size = excluded.size,
                mtime_ns = excluded.mtime_ns, inode = excluded.inode, indexed_offset = excluded.indexed_offset,
                indexed_at = excluded.indexed_at",
            params![
                path.to_string_lossy(),
                state.session_id,
                state.agent,
                state.size as i64,
                state.mtime_ns,
                state.inode.map(|n| n as i64),
                state.indexed_offset as i64,
                chrono::Utc::now().timestamp_millis()
            ],
        )?;
        Ok(())
    }

    pub fn delete_index_state(&self, path: &Path) -> Result<()> {
        self.conn.execute("DELETE FROM index_state WHERE path = ?1", [path.to_string_lossy()])?;
        Ok(())
    }

    pub fn stats(&self) -> Result<IndexStats> {
        let count = |sql: &str| -> Result<usize> { Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize) };
        let db_bytes = self.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0);
        Ok(IndexStats {
            sessions: count("SELECT count(*) FROM sessions")?,
            history_only: count("SELECT count(*) FROM sessions WHERE source = 'history' OR transcript_missing = 1")?,
            messages: count(COUNT_MESSAGES)?,
            projects: count("SELECT count(*) FROM projects")?,
            files: count("SELECT count(*) FROM index_state")?,
            db_bytes,
        })
    }
}

type SchemaEntry = (String, String, Option<String>);

/// Why the index has to be built or rebuilt, or `None` when it is this build's.
fn schema_mismatch(conn: &Connection) -> Result<Option<String>> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let schema = schema_of(conn)?;
    Ok(if version == SCHEMA_VERSION && schema == expected_schema()? {
        None
    } else if version == 0 && schema.is_empty() {
        Some("is a new index and needs building".into())
    } else if version != SCHEMA_VERSION {
        Some(format!("was built with schema version {version} (this build uses {SCHEMA_VERSION}) and needs rebuilding"))
    } else {
        Some("has another schema than this build and needs rebuilding".into())
    })
}

/// Drop every table and create this build's schema, stamped with its version. Run inside a
/// write transaction with foreign keys off (see `ensure_schema`).
fn reset_schema(conn: &Connection) -> Result<()> {
    let names: Vec<(String, String)> = conn
        .prepare("SELECT type, name FROM sqlite_master WHERE type IN ('table', 'view', 'trigger') AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut drop = String::new();
    for (kind, name) in names.iter().filter(|(k, _)| k == "trigger") {
        drop.push_str(&format!("DROP {kind} IF EXISTS \"{name}\";"));
    }
    // the FTS virtual table must go before its content table
    for (kind, name) in names.iter().filter(|(k, n)| k != "trigger" && n == "messages_fts") {
        drop.push_str(&format!("DROP {kind} IF EXISTS \"{name}\";"));
    }
    for (kind, name) in names.iter().filter(|(k, n)| k != "trigger" && n != "messages_fts" && !n.starts_with("messages_fts_")) {
        drop.push_str(&format!("DROP {kind} IF EXISTS \"{name}\";"));
    }
    conn.execute_batch(&drop)?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    Ok(())
}

/// Every table, index and trigger with the statement SQLite kept for it.
fn schema_of(conn: &Connection) -> Result<Vec<SchemaEntry>> {
    Ok(conn
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

/// The schema this build creates, read back from a scratch database so it is normalized the same way.
fn expected_schema() -> Result<Vec<SchemaEntry>> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;
    schema_of(&conn)
}

fn row_to_state(r: &Row) -> rusqlite::Result<IndexState> {
    row_to_state_at(r, 0)
}

/// `session_id, agent, size, mtime_ns, inode, indexed_offset` starting at column `at`.
fn row_to_state_at(r: &Row, at: usize) -> rusqlite::Result<IndexState> {
    Ok(IndexState {
        session_id: r.get(at)?,
        agent: r.get(at + 1)?,
        size: r.get::<_, i64>(at + 2)? as u64,
        mtime_ns: r.get(at + 3)?,
        inode: r.get::<_, Option<i64>>(at + 4)?.map(|n| n as u64),
        indexed_offset: r.get::<_, i64>(at + 5)? as u64,
    })
}

fn row_to_session(r: &Row) -> rusqlite::Result<SessionSummary> {
    let models: Vec<String> = serde_json::from_str(&r.get::<_, String>(30)?).unwrap_or_default();
    let unpriced: Vec<String> = serde_json::from_str(&r.get::<_, String>(36)?).unwrap_or_default();
    let count = |ix: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(ix)? as u64) };
    let tokens = |ix: usize| -> rusqlite::Result<Option<u64>> { Ok(r.get::<_, Option<i64>>(ix)?.map(|n| n as u64)) };
    Ok(SessionSummary {
        id: r.get(0)?,
        id_known: r.get(1)?,
        project_key: r.get(2)?,
        project_path: r.get(3)?,
        file_path: r.get::<_, Option<String>>(4)?.map(PathBuf::from),
        source: Source::parse(&r.get::<_, String>(5)?),
        transcript_missing: r.get(6)?,
        title: r.get(7)?,
        title_rank: r.get(8)?,
        first_prompt: r.get(9)?,
        last_prompt: r.get(10)?,
        last_message: r.get(11)?,
        started_at: r.get(12)?,
        updated_at: r.get(13)?,
        git_branch: r.get(14)?,
        cwd: r.get(15)?,
        cc_version: r.get(16)?,
        entrypoint: r.get(17)?,
        permission_mode: r.get(18)?,
        is_background: r.get(19)?,
        user_msg_count: r.get(20)?,
        assistant_msg_count: r.get(21)?,
        tool_use_count: r.get(22)?,
        subagent_count: r.get(23)?,
        file_size: r.get::<_, Option<i64>>(24)?.map(|n| n as u64),
        total_cost_usd: r.get(25)?,
        duration_ms: r.get(26)?,
        lines_added: r.get(27)?,
        lines_removed: r.get(28)?,
        pr_url: r.get(29)?,
        tool_counts: BTreeMap::new(),
        files_touched: BTreeSet::new(),
        models: models.into_iter().collect(),
        cost_usd: r.get(31)?,
        tokens_in: count(32)?,
        tokens_out: count(33)?,
        tokens_cache_read: count(34)?,
        tokens_cache_write: count(35)?,
        unpriced_models: unpriced.into_iter().collect(),
        context_tokens: tokens(37)?,
        context_peak: tokens(38)?,
        context_model: r.get(39)?,
        favorite: r.get(40)?,
        model_usage: Vec::new(),
    })
}

/// Rows of `session_model_usage`, in key order; `filter` is a WHERE clause or empty.
fn model_usage_sql(filter: &str) -> String {
    format!(
        "SELECT session_id, model, fast, long_prompt, responses, input, cache_write_5m, cache_write_1h, cache_read, output,
            web_search, cost_usd
         FROM session_model_usage {filter} ORDER BY session_id, model, fast, long_prompt"
    )
}

fn row_to_model_usage(r: &Row) -> rusqlite::Result<(String, ModelUsage)> {
    let n = |ix: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(ix)? as u64) };
    let tokens = Tokens { input: n(5)?, cache_write_5m: n(6)?, cache_write_1h: n(7)?, cache_read: n(8)?, output: n(9)?, web_search: n(10)? };
    let group = ModelUsage { model: r.get(1)?, fast: r.get(2)?, long_prompt: r.get(3)?, responses: n(4)?, tokens, cost_usd: r.get(11)? };
    Ok((r.get(0)?, group))
}

fn remove_db_files(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_count_reads_its_own_index_and_never_the_rows() {
        let db = Db::open_in_memory().unwrap();
        let plan: Vec<String> = db
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {COUNT_MESSAGES}"))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(plan, ["SEARCH messages USING COVERING INDEX messages_counted (<expr>=?)"]);
    }
}
