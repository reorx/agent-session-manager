//! The SQLite index: one connection, owned by whoever indexes or queries (never shared across
//! threads). Schema in `schema.sql`; a version mismatch or an unreadable file rebuilds it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::model::{MessageRow, Role, SessionSummary, Source};

pub const SCHEMA_VERSION: i32 = 1;
const SCHEMA: &str = include_str!("schema.sql");

pub struct Db {
    conn: Connection,
    path: Option<PathBuf>,
}

/// Per-file bookkeeping for incremental scans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexState {
    pub session_id: String,
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
    pub project_keys: Option<Vec<String>>,
    /// Keep sessions with `updated_at >= since_ms`.
    pub since_ms: Option<i64>,
    pub order: Order,
}

const SESSION_COLUMNS: &str = "s.id, s.id_known, p.encoded_dir, p.path, s.file_path, s.source, s.transcript_missing, \
     s.title, s.title_rank, s.first_prompt, s.last_prompt, s.last_message, s.started_at, s.updated_at, \
     s.git_branch, s.cwd, s.cc_version, s.entrypoint, s.permission_mode, s.is_background, \
     s.user_msg_count, s.assistant_msg_count, s.tool_use_count, s.subagent_count, s.file_size, \
     s.total_cost_usd, s.duration_ms, s.lines_added, s.lines_removed, s.pr_url, s.models";

impl Db {
    /// Open (or create) the index at `path`. Unreadable files are deleted and recreated; an old
    /// schema version is dropped, so callers must rescan afterwards.
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        match Self::open_checked(path) {
            Ok(db) => Ok(db),
            Err(_) => {
                remove_db_files(path);
                Self::open_checked(path)
            }
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

    fn open_checked(path: &Path) -> Result<Db> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut db = Db { conn, path: Some(path.to_path_buf()) };
        db.configure()?;
        db.ensure_schema()?;
        Ok(db)
    }

    fn configure(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;
             PRAGMA cache_size = -65536;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(())
    }

    fn ensure_schema(&mut self) -> Result<()> {
        let version: i32 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let has_tables: i64 = self
            .conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'sessions'", [], |r| r.get(0))?;
        if version == SCHEMA_VERSION && has_tables == 1 {
            return Ok(());
        }
        self.reset_schema()
    }

    /// Drop every table and recreate the schema (used for version bumps and `rebuild`).
    pub fn reset_schema(&mut self) -> Result<()> {
        let names: Vec<(String, String)> = self
            .conn
            .prepare("SELECT type, name FROM sqlite_master WHERE type IN ('table', 'view', 'trigger') AND name NOT LIKE 'sqlite_%'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut drop = String::from("PRAGMA foreign_keys = OFF;");
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
        drop.push_str("PRAGMA foreign_keys = ON;");
        self.conn.execute_batch(&drop)?;
        self.conn.execute_batch(SCHEMA)?;
        self.conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        Ok(())
    }

    /// Delete every row but keep the schema (rebuild).
    pub fn clear_all(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM messages; DELETE FROM session_files; DELETE FROM session_tool_counts;
             DELETE FROM sessions; DELETE FROM projects; DELETE FROM index_state;
             INSERT INTO messages_fts(messages_fts) VALUES ('rebuild');",
        )?;
        Ok(())
    }

    pub fn set_user_version_for_test(&self, version: i32) {
        self.conn.execute_batch(&format!("PRAGMA user_version = {version};")).unwrap();
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
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
                subagent_count, file_size, total_cost_usd, duration_ms, lines_added, lines_removed, pr_url, models)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)
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
                models = excluded.models",
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
        if let Some(keys) = &filter.project_keys {
            let marks: Vec<String> = keys.iter().enumerate().map(|(i, _)| format!("?{}", args.len() + i + 1)).collect();
            sql.push_str(&format!(" AND p.encoded_dir IN ({})", marks.join(",")));
            args.extend(keys.iter().map(|k| k.clone().into()));
        }
        if let Some(since) = filter.since_ms {
            args.push(since.into());
            sql.push_str(&format!(" AND s.updated_at >= ?{}", args.len()));
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
        Ok(())
    }

    pub fn mark_transcript_missing(&self, id: &str) -> Result<()> {
        self.conn.execute("UPDATE sessions SET transcript_missing = 1 WHERE id = ?1", [id])?;
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
        by_id.clear();
        for (ix, s) in sessions.iter_mut().enumerate() {
            s.tool_counts = tools.remove(&ix).unwrap_or_default();
            s.files_touched = files.remove(&ix).unwrap_or_default();
        }
        Ok(())
    }

    // ---- messages -------------------------------------------------------------------------

    pub fn insert_messages(&self, session_id: &str, messages: &[MessageRow]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO messages (session_id, uuid, parent_uuid, role, ts, text, is_meta, byte_offset, byte_len, model, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for m in messages {
            stmt.execute(params![
                session_id,
                m.uuid,
                m.parent_uuid,
                m.role.as_str(),
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

    pub fn delete_messages(&self, session_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
        Ok(())
    }

    /// Messages of one session in file order.
    pub fn messages(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT uuid, parent_uuid, role, ts, text, is_meta, byte_offset, byte_len, model, input_tokens, output_tokens
             FROM messages WHERE session_id = ?1 ORDER BY byte_offset",
        )?;
        let rows = stmt
            .query_map([session_id], |r| {
                Ok(MessageRow {
                    uuid: r.get(0)?,
                    parent_uuid: r.get(1)?,
                    role: Role::parse(&r.get::<_, String>(2)?).unwrap_or(Role::User),
                    ts: r.get(3)?,
                    text: r.get(4)?,
                    is_meta: r.get(5)?,
                    byte_offset: r.get::<_, i64>(6)? as u64,
                    byte_len: r.get::<_, i64>(7)? as u64,
                    model: r.get(8)?,
                    input_tokens: r.get(9)?,
                    output_tokens: r.get(10)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    // ---- index_state ----------------------------------------------------------------------

    pub fn index_state(&self, path: &Path) -> Result<Option<IndexState>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT session_id, size, mtime_ns, inode, indexed_offset FROM index_state WHERE path = ?1",
        )?;
        Ok(stmt.query_row([path.to_string_lossy()], row_to_state).optional()?)
    }

    pub fn all_index_states(&self) -> Result<HashMap<PathBuf, IndexState>> {
        let mut stmt = self.conn.prepare_cached("SELECT path, session_id, size, mtime_ns, inode, indexed_offset FROM index_state")?;
        let mut out = HashMap::new();
        for row in stmt.query_map([], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                IndexState {
                    session_id: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    mtime_ns: r.get(3)?,
                    inode: r.get::<_, Option<i64>>(4)?.map(|n| n as u64),
                    indexed_offset: r.get::<_, i64>(5)? as u64,
                },
            ))
        })? {
            let (path, state) = row?;
            out.insert(path, state);
        }
        Ok(out)
    }

    pub fn set_index_state(&self, path: &Path, state: &IndexState) -> Result<()> {
        self.conn.execute(
            "INSERT INTO index_state (path, session_id, size, mtime_ns, inode, indexed_offset, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET session_id = excluded.session_id, size = excluded.size,
                mtime_ns = excluded.mtime_ns, inode = excluded.inode, indexed_offset = excluded.indexed_offset,
                indexed_at = excluded.indexed_at",
            params![
                path.to_string_lossy(),
                state.session_id,
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
            messages: count("SELECT count(*) FROM messages")?,
            projects: count("SELECT count(*) FROM projects")?,
            files: count("SELECT count(*) FROM index_state")?,
            db_bytes,
        })
    }
}

fn row_to_state(r: &Row) -> rusqlite::Result<IndexState> {
    Ok(IndexState {
        session_id: r.get(0)?,
        size: r.get::<_, i64>(1)? as u64,
        mtime_ns: r.get(2)?,
        inode: r.get::<_, Option<i64>>(3)?.map(|n| n as u64),
        indexed_offset: r.get::<_, i64>(4)? as u64,
    })
}

fn row_to_session(r: &Row) -> rusqlite::Result<SessionSummary> {
    let models: Vec<String> = serde_json::from_str(&r.get::<_, String>(30)?).unwrap_or_default();
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
    })
}

fn remove_db_files(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
}
