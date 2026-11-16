CREATE TABLE responses (
  session_id TEXT NOT NULL,
  agent TEXT NOT NULL DEFAULT '',       -- '' 是 session 自己的记录文件，否则是 subagent 文件名
  msg_id TEXT NOT NULL,
  model TEXT NOT NULL,                  -- 原样存，计价时再 normalize
  ts INTEGER NOT NULL,                  -- unix ms
  input INTEGER NOT NULL,
  cache_write_5m INTEGER NOT NULL,
  cache_write_1h INTEGER NOT NULL,
  cache_read INTEGER NOT NULL,
  output INTEGER NOT NULL,
  web_search INTEGER NOT NULL,
  fast INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (session_id, agent, msg_id)
) WITHOUT ROWID;

-- responses 按决定价格的键和小时分组，由 Ledger::refresh_hourly 按 session 重写。
CREATE TABLE usage_hourly (
  session_id TEXT NOT NULL,
  hour INTEGER NOT NULL,                -- ts / 3_600_000，UTC 整点
  model TEXT NOT NULL,
  fast INTEGER NOT NULL,
  long_prompt INTEGER NOT NULL,         -- 这次请求的输入超过 pricing::LONG_PROMPT
  sub INTEGER NOT NULL,                 -- 1 = 来自 subagent 文件
  responses INTEGER NOT NULL,
  input INTEGER NOT NULL,
  cache_write_5m INTEGER NOT NULL,
  cache_write_1h INTEGER NOT NULL,
  cache_read INTEGER NOT NULL,
  output INTEGER NOT NULL,
  web_search INTEGER NOT NULL,
  PRIMARY KEY (session_id, hour, model, fast, long_prompt, sub)
) WITHOUT ROWID;

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  project_key TEXT NOT NULL,            -- 索引里的 projects.encoded_dir
  title TEXT NOT NULL DEFAULT '',       -- display_title() 的前 200 个字符
  entrypoint TEXT,
  is_background INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER,
  updated_at INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;

CREATE TABLE projects (key TEXT PRIMARY KEY, path TEXT) WITHOUT ROWID;

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
