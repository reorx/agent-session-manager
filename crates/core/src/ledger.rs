//! Durable API usage. Only derived hourly rows may be deleted; schema changes require migrations.
use std::{collections::{BTreeMap, BTreeSet, HashMap}, path::{Path, PathBuf}, time::Duration};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use crate::model::{HourlyUsage, LedgerSession, SessionUsage, Tokens, UsageRow};

pub const LEDGER_VERSION: i32 = 1;
const SCHEMA: &str = include_str!("ledger.sql");

#[derive(Debug)]
pub struct NewerVersion(pub i32);
impl std::fmt::Display for NewerVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "ledger version {} is newer than {LEDGER_VERSION}", self.0) }
}
impl std::error::Error for NewerVersion {}

/// Marks only errors from the ledger connection, never errors reading the source index.
#[derive(Debug)]
pub(crate) struct LedgerFailure;
impl std::fmt::Display for LedgerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "usage ledger") }
}
impl std::error::Error for LedgerFailure {}

#[derive(Default)]
pub struct LedgerBatch {
    pub responses: Vec<(String, String, UsageRow)>,
    pub projects: Vec<(String, Option<String>)>,
    pub sessions: BTreeSet<String>,
}
impl LedgerBatch {
    pub fn is_empty(&self) -> bool { self.responses.is_empty() && self.projects.is_empty() && self.sessions.is_empty() }
}

pub struct Ledger { conn: Connection }

pub(crate) fn corrupt(e: &anyhow::Error) -> bool {
    e.chain().any(|e| e.downcast_ref::<rusqlite::Error>().and_then(|e|e.sqlite_error_code())
        .is_some_and(|c| matches!(c, rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)))
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Self> {
        match Self::open_checked(path) {
            Err(e) if corrupt(&e) => {
                Self::quarantine(path)?;
                Self::open_checked(path)
            }
            result => result,
        }
    }

    fn open_checked(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r|r.get(0))?;
        // A downgrade must not change journal mode, schema or version.
        if version > LEDGER_VERSION { return Err(NewerVersion(version).into()); }
        use_wal(&conn)?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        let ledger = Self { conn };
        if version < LEDGER_VERSION { ledger.migrate()?; }
        Ok(ledger)
    }

    pub fn open_in_memory() -> Result<Self> {
        let ledger = Self { conn: Connection::open_in_memory()? };
        ledger.migrate()?;
        Ok(ledger)
    }

    fn migrate(&self) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        // Recheck under the write lock: two indexes can create the shared ledger together.
        let version: i32 = tx.query_row("PRAGMA user_version", [], |r|r.get(0))?;
        if version > LEDGER_VERSION { return Err(NewerVersion(version).into()); }
        if version == 0 {
            tx.execute_batch(SCHEMA)?;
            tx.pragma_update(None, "user_version", 1)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Called only after closing the damaged connection. Keep every file for manual recovery.
    pub(crate) fn quarantine(path: &Path) -> Result<()> {
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%f");
        let target = path.with_file_name(format!("usage-ledger.corrupt-{stamp}.sqlite"));
        for suffix in ["", "-wal", "-shm"] {
            let source = suffixed(path, suffix);
            if source.exists() { std::fs::rename(&source, suffixed(&target, suffix))?; }
        }
        Ok(())
    }

    pub fn write(&self, batch: &LedgerBatch, index: &Connection) -> Result<()> {
        // Read the index before starting a ledger transaction, preserving source error identity.
        let sessions = read_sessions(index, &batch.sessions)?;
        self.write_rows(batch, &sessions).context(LedgerFailure)
    }

    fn write_rows(&self, batch: &LedgerBatch, sessions: &[LedgerSession]) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut stmt = tx.prepare_cached(
            "INSERT INTO responses VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(session_id,agent,msg_id) DO UPDATE SET model=excluded.model,
             ts=min(responses.ts,excluded.ts),input=excluded.input,cache_write_5m=excluded.cache_write_5m,
             cache_write_1h=excluded.cache_write_1h,cache_read=excluded.cache_read,output=excluded.output,
             web_search=excluded.web_search,fast=excluded.fast")?;
        let mut changed = batch.sessions.clone();
        for (sid, agent, u) in &batch.responses {
            stmt.execute(params![sid,agent,u.msg_id,u.model,u.ts,u.input as i64,u.cache_write_5m as i64,u.cache_write_1h as i64,u.cache_read as i64,u.output as i64,u.web_search as i64,u.fast])?;
            changed.insert(sid.clone());
        }
        drop(stmt);
        for (key,path) in &batch.projects { write_project(&tx, key, path.as_deref())?; }
        for s in sessions {
            write_project(&tx, &s.project_key, s.project_path.as_deref())?;
            tx.execute(
                "INSERT INTO sessions VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(id) DO UPDATE SET project_key=excluded.project_key,title=excluded.title,
                 entrypoint=excluded.entrypoint,is_background=excluded.is_background,
                 started_at=excluded.started_at,updated_at=excluded.updated_at",
                params![s.id,s.project_key,s.title,s.entrypoint,s.is_background,s.started_at,s.updated_at])?;
        }
        for sid in changed { self.refresh_hourly(&sid)?; }
        tx.commit()?;
        Ok(())
    }

    pub fn refresh_hourly(&self, session_id: &str) -> Result<()> {
        let tx = if self.conn.is_autocommit() {
            Some(rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?)
        } else {
            None
        };
        self.conn.execute("DELETE FROM usage_hourly WHERE session_id=?1", [session_id])?;
        self.conn.execute(&format!(
            "INSERT INTO usage_hourly SELECT session_id,ts/3600000 AS hour,model,fast,
             input+cache_write_5m+cache_write_1h+cache_read > {} AS long_prompt,agent<>'' AS sub,
             count(*),sum(input),sum(cache_write_5m),sum(cache_write_1h),sum(cache_read),sum(output),sum(web_search)
             FROM responses WHERE session_id=?1 GROUP BY session_id,hour,model,fast,long_prompt,sub",
            crate::pricing::LONG_PROMPT), [session_id])?;
        if let Some(tx) = tx { tx.commit()?; }
        Ok(())
    }

    pub fn response_counts(&self) -> Result<HashMap<String,u64>> {
        Ok(self.conn.prepare("SELECT session_id,sum(responses) FROM usage_hourly GROUP BY session_id")?
            .query_map([], |r|Ok((r.get(0)?,r.get::<_,i64>(1)? as u64)))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn usage(&self, ids: Option<&[String]>) -> Result<Vec<SessionUsage>> {
        let wanted = ids.map(|ids|ids.iter().map(String::as_str).collect::<BTreeSet<_>>());
        let mut out: BTreeMap<String,SessionUsage> = BTreeMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT ids.id,s.project_key,p.path,s.title,s.entrypoint,s.is_background,s.started_at,s.updated_at,
             h.hour,h.model,h.fast,h.long_prompt,h.sub,h.responses,h.input,h.cache_write_5m,h.cache_write_1h,h.cache_read,h.output,h.web_search
             FROM (SELECT id FROM sessions UNION SELECT session_id FROM usage_hourly) ids
             LEFT JOIN sessions s ON s.id=ids.id
             LEFT JOIN usage_hourly h ON h.session_id=ids.id
             LEFT JOIN projects p ON p.key=s.project_key ORDER BY ids.id,h.hour,h.model,h.fast,h.long_prompt,h.sub")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            if wanted.as_ref().is_some_and(|ids|!ids.contains(id.as_str())) { continue; }
            let session = LedgerSession { id:id.clone(), project_key:r.get::<_,Option<String>>(1)?.unwrap_or_default(),
                project_path:r.get(2)?, title:r.get::<_,Option<String>>(3)?.unwrap_or_default(), entrypoint:r.get(4)?,
                is_background:r.get::<_,Option<bool>>(5)?.unwrap_or_default(), started_at:r.get(6)?,
                updated_at:r.get::<_,Option<i64>>(7)?.unwrap_or_default() };
            let entry = out.entry(id).or_insert_with(||SessionUsage {session,hours:Vec::new()});
            let Some(hour) = r.get::<_, Option<i64>>(8)? else { continue; };
            let hour = HourlyUsage {hour,model:r.get(9)?,fast:r.get(10)?,long_prompt:r.get(11)?,sub:r.get(12)?,
                responses:r.get::<_,i64>(13)? as u64,tokens:Tokens {input:r.get::<_,i64>(14)? as u64,cache_write_5m:r.get::<_,i64>(15)? as u64,cache_write_1h:r.get::<_,i64>(16)? as u64,
                cache_read:r.get::<_,i64>(17)? as u64,output:r.get::<_,i64>(18)? as u64,web_search:r.get::<_,i64>(19)? as u64}};
            entry.hours.push(hour);
        }
        Ok(out.into_values().collect())
    }
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s=path.as_os_str().to_owned(); s.push(suffix); s.into()
}
fn write_project(conn: &Connection, key: &str, path: Option<&str>) -> Result<()> {
    conn.execute("INSERT INTO projects VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET path=coalesce(excluded.path,projects.path)",params![key,path])?;
    Ok(())
}
pub(crate) fn missing_schema(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.starts_with("no such table:") || msg.starts_with("no such column:"))
}
fn read_sessions(index: &Connection, ids: &BTreeSet<String>) -> Result<Vec<LedgerSession>> {
    let mut stmt = match index.prepare(
        "SELECT s.id,p.encoded_dir,p.path,s.title,s.first_prompt,s.entrypoint,s.is_background,s.started_at,s.updated_at
         FROM sessions s JOIN projects p ON p.id=s.project_id WHERE s.id=?1 AND s.source='transcript'") {
        Err(e) if missing_schema(&e) => return Ok(Vec::new()),
        result => result?,
    };
    let mut out=Vec::new();
    for id in ids {
        let s=stmt.query_row([id], |r| {
            let title:Option<String>=r.get(3)?;
            let prompt:String=r.get(4)?;
            let title=title.filter(|s|!s.is_empty()).unwrap_or_else(||prompt.lines().next().unwrap_or("").to_owned());
            Ok(LedgerSession {id:r.get(0)?,project_key:r.get(1)?,project_path:r.get(2)?,title:title.chars().take(200).collect(),
                entrypoint:r.get(5)?,is_background:r.get(6)?,started_at:r.get(7)?,updated_at:r.get(8)?})
        }).optional()?;
        if let Some(s)=s {out.push(s);}
    }
    Ok(out)
}

/// Read old indexes without depending on their current schema version or stored costs.
pub(crate) fn read_responses(source: &Connection, ids: Option<&BTreeSet<String>>) -> Result<LedgerBatch> {
    let mut batch=LedgerBatch::default();
    let mut sql = "SELECT session_id,agent,msg_id,model,ts,input,cache_write_5m,cache_write_1h,cache_read,output,web_search,fast FROM api_usage".to_owned();
    if ids.is_some() {
        sql.push_str(" WHERE session_id IN (SELECT value FROM json_each(?1))");
    }
    let mut stmt=match source.prepare(&sql) {
        Err(e) if missing_schema(&e) => return Ok(batch),
        result => result?,
    };
    let mut rows = match ids {
        Some(ids) => stmt.query([serde_json::to_string(ids)?])?,
        None => stmt.query([])?,
    };
    while let Some(r)=rows.next()? {
        let sid:String=r.get(0)?;
        batch.sessions.insert(sid.clone());
        batch.responses.push((sid,r.get(1)?,UsageRow {msg_id:r.get(2)?,model:r.get(3)?,ts:r.get(4)?,input:r.get::<_,i64>(5)? as u64,
            cache_write_5m:r.get::<_,i64>(6)? as u64,cache_write_1h:r.get::<_,i64>(7)? as u64,cache_read:r.get::<_,i64>(8)? as u64,output:r.get::<_,i64>(9)? as u64,web_search:r.get::<_,i64>(10)? as u64,fast:r.get(11)?,cost_usd:None}));
    }
    if ids.is_none() {
        match source.prepare("SELECT id FROM sessions WHERE source='transcript'") {
            Ok(mut stmt) => {
                batch.sessions.extend(stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
            }
            Err(e) if missing_schema(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(batch)
}

// As for the index, simultaneous WAL switches can fail immediately instead of honoring
// busy_timeout. Retry only this initialization operation, within the same five-second budget.
fn use_wal(conn: &Connection) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        match conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0)) {
            Err(e) if matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
                && start.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
            result => return Ok(result.map(drop)?),
        }
    }
}
