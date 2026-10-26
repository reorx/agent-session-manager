-- Agent Session Manager index. Bump SCHEMA_VERSION in db.rs whenever this file changes:
-- the whole index is rebuilt from disk, nothing here is the source of truth.

CREATE TABLE projects (
  id INTEGER PRIMARY KEY,
  encoded_dir TEXT NOT NULL UNIQUE,     -- '-Users-reorx-Code-scripts'
  path TEXT,                            -- '/Users/reorx/Code/scripts' from cwd / history.project
  agent TEXT NOT NULL DEFAULT 'claude-code'
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,                  -- session uuid, or 'history:<project>:<ts>' when id_known = 0
  id_known INTEGER NOT NULL DEFAULT 1,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  file_path TEXT,
  source TEXT NOT NULL DEFAULT 'transcript',   -- 'transcript' | 'history'
  transcript_missing INTEGER NOT NULL DEFAULT 0,
  title TEXT,
  title_rank INTEGER NOT NULL DEFAULT 0,
  first_prompt TEXT NOT NULL DEFAULT '',
  last_prompt TEXT,
  last_message TEXT,
  started_at INTEGER,                   -- unix ms
  updated_at INTEGER NOT NULL DEFAULT 0,
  git_branch TEXT,
  cwd TEXT,
  cc_version TEXT,
  entrypoint TEXT,
  permission_mode TEXT,
  is_background INTEGER NOT NULL DEFAULT 0,
  user_msg_count INTEGER NOT NULL DEFAULT 0,
  assistant_msg_count INTEGER NOT NULL DEFAULT 0,
  tool_use_count INTEGER NOT NULL DEFAULT 0,
  subagent_count INTEGER NOT NULL DEFAULT 0,
  file_size INTEGER,
  total_cost_usd REAL,
  duration_ms INTEGER,
  lines_added INTEGER,
  lines_removed INTEGER,
  pr_url TEXT,
  models TEXT NOT NULL DEFAULT '[]',    -- JSON array of model ids
  -- usage totals over api_usage (subagents included), written only by refresh_session_usage
  cost_usd REAL,                        -- NULL when no call could be priced
  tokens_in INTEGER NOT NULL DEFAULT 0,
  tokens_out INTEGER NOT NULL DEFAULT 0,
  tokens_cache_read INTEGER NOT NULL DEFAULT 0,
  tokens_cache_write INTEGER NOT NULL DEFAULT 0,
  unpriced_models TEXT NOT NULL DEFAULT '[]',  -- JSON array of models without a price
  -- last / largest prompt of a main-thread call (from the parser, like the fields above)
  context_tokens INTEGER,
  context_peak INTEGER,
  context_model TEXT,
  favorite INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX sessions_updated ON sessions(updated_at DESC);
CREATE INDEX sessions_project ON sessions(project_id, updated_at DESC);

CREATE TABLE session_tool_counts (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  tool_name TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (session_id, tool_name)
) WITHOUT ROWID;

CREATE TABLE session_files (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  PRIMARY KEY (session_id, path)
) WITHOUT ROWID;

CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  uuid TEXT NOT NULL,
  parent_uuid TEXT,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  ts INTEGER NOT NULL,
  text TEXT NOT NULL,
  is_meta INTEGER NOT NULL DEFAULT 0,
  byte_offset INTEGER NOT NULL,
  byte_len INTEGER NOT NULL,
  model TEXT,
  input_tokens INTEGER,
  output_tokens INTEGER
);
CREATE INDEX messages_session_ts ON messages(session_id, byte_offset);

CREATE VIRTUAL TABLE messages_fts USING fts5(
  text,
  role UNINDEXED,
  session_id UNINDEXED,
  content='messages',
  content_rowid='id',
  tokenize="trigram case_sensitive 0"
);
CREATE TRIGGER messages_ai AFTER INSERT ON messages WHEN new.is_meta = 0 BEGIN
  INSERT INTO messages_fts(rowid, text, role, session_id) VALUES (new.id, new.text, new.role, new.session_id);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages WHEN old.is_meta = 0 BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text, role, session_id) VALUES ('delete', old.id, old.text, old.role, old.session_id);
END;

-- One row per API response. agent = '' for the session's own transcript, else the subagent file
-- stem. No foreign key: a subagent file may be indexed before its session row exists. Rows of
-- one response cut between two scans land on the same key, so they count once.
CREATE TABLE api_usage (
  session_id TEXT NOT NULL,
  agent TEXT NOT NULL DEFAULT '',
  msg_id TEXT NOT NULL,
  model TEXT NOT NULL,
  ts INTEGER NOT NULL,
  input INTEGER NOT NULL,
  cache_write_5m INTEGER NOT NULL,
  cache_write_1h INTEGER NOT NULL,
  cache_read INTEGER NOT NULL,
  output INTEGER NOT NULL,
  web_search INTEGER NOT NULL,
  fast INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL,                        -- NULL when the model has no known price
  PRIMARY KEY (session_id, agent, msg_id)
) WITHOUT ROWID;

CREATE TABLE index_state (
  path TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,             -- for a subagent file: the session it belongs to
  agent TEXT NOT NULL DEFAULT '',       -- '' for a session transcript, else the subagent file stem
  size INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  inode INTEGER,
  indexed_offset INTEGER NOT NULL DEFAULT 0,
  indexed_at INTEGER
);

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
