# Agent Session Manager

macOS 桌面应用（Rust + GPUI / gpui-kit），以表格列出本机所有 Claude Code session：项目 / 时间 / Kind 过滤、表头排序、全文搜索、详情面板。核心诉求是界面响应与查询速度。

## 当前状态

M0–M3 完成（2026-09-21），下一步 **M4：详情面板**，方案在 `kb/plans/2026-09-28-m4-detail-panel.md`（第 1 节的待确认项 D1–D6 还没有答复）。功能状态表在 `kb/docs/product-prd.md`，里程碑定义在 `kb/plans/2026-09-08-agent-session-manager-prd.md` §3。

- `crates/core`（`agent-session-core`，无 gpui 依赖）：JSONL 解析、增量 SQLite 索引、FTS5 / 正则搜索、`asm` CLI
- `crates/app`（`agent-session-manager`）：gpui-kit 界面，保持薄。`store.rs` 后台线程独占 `Db`，启动先读索引出表，再流式增量扫描；`search.rs` 是另一条线程，自带只读连接，文本模式走 FTS、正则模式流式扫描 transcript
- Conversation tab 是过渡实现（索引里的 user / assistant 文本，前 300 条），M4 改为直接读 transcript
- 未解决的问题记在 `kb/known-issues.md`

## 开发命令

`make help` 列出全部命令，README 有说明。要点：

- `make dev`：编译并启动 app（`run` 会先 pkill 旧实例）。手动起的 app 验证后必须 kill，用 `pgrep -fl agent-session-manager` 检查
- 测试：`cargo test -p agent-session-core`（1 秒内）、`cargo test -p agent-session-manager`（纯逻辑，不起窗口）、`make test` 全跑
- `make asm ARGS="…"`：跑 `asm` CLI（`index` / `list` / `session` / `search [--index]` / `stats`，参数看 `--help`）
- `make parity`：在真实 `~/.claude` 上把 `asm` 与 Python 脚本逐字比对；改解析器、清理逻辑、`regex.rs` 或输出格式后必须跑
- `make screenshot`（autoplay，3 张）、`make screenshot-clicks`（cliclick 真点击，12 张；坐标在 `scripts/screenshot-clicks.sh`，改布局后要校对）、`make screenshot-search`（`ASM_SEARCH_AUTOPLAY=1`，app 自己走完搜索流程，脚本等日志标记后截 7 张）。输出到 `tmp/<date>-screenshots/`（`SHOT_DIR=` 可改），跑在真实索引上，但用一次性的 `state.json` 保证从默认视图开始
- 构建产物在 `~/Library/Caches/cargo-target`（全局 cargo 配置），不在 `./target`；`make bin` 打印二进制路径。首次构建约 1.5 分钟，增量约 2 秒
- 环境变量：`ASM_CLAUDE_DIR`（默认 `$CLAUDE_CONFIG_DIR` 或 `~/.claude`）、`ASM_DB`（默认 `~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite`）、`ASM_STATE`（仅 app，默认为索引同目录的 `state.json`）。验收时把 `ASM_DB` / `ASM_STATE` 指到 `tmp/`，可反复测冷启动而不动真实索引
- Rust 由 mise 管理（版本锁在 `mise.toml`）；新机器先 `make setup`（含 Metal Toolchain）

## 验收流程

1. 截图前先 `wake-screen`（`make screenshot` 已包含），熄屏时截图是黑的
2. 截图必须用 `Read` 打开看过再下结论，不要只看文件大小
3. 通过后把截图归档到 `tmp/<date>-<task>/`（已 gitignore）并在报告里给出路径
4. 里程碑完成后更新本文件的"当前状态"和 `kb/docs/product-prd.md` 的状态表，然后 commit

## core crate 编码规则

细节与原因见 `kb/notes/2026-09-17-m1-core-crate.md`。

- 对外类型在 `model.rs`（`SessionSummary` = `sessions` 表一行，`MessageRow` = `messages` 表一行）；UI 层只消费这些类型，不自己解析 JSONL
- 解析器先写行为测试：fixture 在 `crates/core/tests/fixtures/claude/`，由同目录 `gen_fixtures.py` 生成（`uv run` 重跑并提交结果）；新记录类型先加进生成脚本再改 `parse.rs`
- 改 `parse.rs::classify`（字节预过滤）前，按 M1 笔记的方法在真实数据上全量验证
- `&RawValue` 不能放在 `#[serde(tag)]` / `untagged` / `flatten` 之下；记录和 block 都用"大结构体 + Option 字段 + match type"
- 改 `schema.sql` 必须同步 `db.rs` 的 `SCHEMA_VERSION`、`SESSION_COLUMNS` 和 `row_to_session` 的列序；版本不符会整库重建，无迁移
- 一个 `Db` 只在一个线程用；rayon 只用于解析，写库在调用线程的单个事务里
- `SessionDelta::apply` 必须保持"全量解析 == 多次增量解析"（有测试断言）；新字段要归类为 first（只设一次）、last（覆盖）或 count（累加）

## GPUI / gpui-kit 编码规则

- 只依赖 `gpui-kit = "=0.6.1"`，**不要**再声明 `gpui` 或 `gpui-pre`，否则两份 GPUI 类型不匹配
- **API 以本地源码为准**，在线文档和 docs.rs 都有偏差。查签名直接 grep `~/.cargo/registry/src/index.crates.io-*/{gpui-component-0.6.1,gpui-base-0.6.1,gpui-pre-0.3.5}/src`
- 数据只经 `store.rs` 进出（UI 发 `StoreRequest`，后台线程回 `StoreEvent`）；`SessionRow::from_summary` 是 core 类型到界面行的唯一转换点
- 搜索只经 `search.rs` 进出：每次搜索取代上一次，`on_search_events` 丢弃旧 `seq` 的事件；搜索对象是通过当前过滤条件的 session，所以过滤变化或整表 reload 后要重搜
- 只有 SQLite 查询和文件扫描走后台，过滤 / 排序在主线程做；`render_td` 只取预格式化的字符串，不做计算
- 色值集中在 `style.rs`，不要在别处写 hex
- `Task` drop 即取消：`cx.spawn` 的结果要 `.detach()` 或存进字段；`this.update(cx, ..)` 返回 Result，用 `.ok()` 不要 unwrap
- 测试模块里要写 `use core::prelude::v1::test;`，否则 `use gpui_kit::*` 带进的 `test` 宏会让 `#[test]` 展开递归超限
- 程序化 `InputState::set_value` 不发 `InputEvent::Change`，之后要自己触发后续逻辑（如 `AppRoot::set_query` 直接调 `run_search`）
- **验收时不要用 System Events / osascript / cliclick 发按键，也不要动剪贴板**：全局按键只进最前台的 app，会打进用户正在用的窗口，GPUI 也收不到 cliclick 的按键。交互用环境变量驱动的 autoplay（app 每步稳定后打日志标记，脚本等到标记再按窗口 ID 截图）；cliclick 鼠标点击只用于无法程序化打开的弹出层

其余做法在 `kb/docs/gpui-kit-guide.md`：表格的行高 / 列宽 / 排序状态 / 选中保持、主题三步、下拉菜单、返回类型的借用冲突、截图脚本。改 `table.rs`、`style.rs`、`toolbar.rs` 或 `main.rs` 的渲染与选中逻辑前先读。

## 关键决策

- UI 走 gpui-kit 精确锁版（提交 Cargo.lock）：官方 `gpui` 在 crates.io 停更，Zed 的 `ui` / `theme` crate 是 GPL 且未发布，只读源码不依赖。理由见 `kb/docs/gpui-framework-research.md` §1
- 数据层：SQLite（rusqlite bundled，WAL，单写线程）+ FTS5 trigram，只索引 user / assistant 文本；正则模式用 memmap2 + memchr + `regex` 扫原始文件，在 JSON 解码后的文本上匹配
- SDK 与后台 session 不隐藏，用 Kind 列徽章区分
- bundle id 定为 `com.reorx.agent-session-manager`（M5 打包时用）；数据目录名是 `dev.reorx.AgentSessionManager`，与之不同

## 参考

- `~/Code/scripts/claude_session_search.py`：现有 Python 实现，本项目的行为基准
- `~/Code/agent-sessions`：旧项目（transcript → 静态站点），含 Codex parser，二期多 agent 支持的参考

## Documentation

- `kb/plans/2026-09-08-agent-session-manager-prd.md` — 需求 R1–R13、架构、里程碑 M0–M5、风险；开发任务的总入口
- `kb/docs/product-prd.md` — 产品视角的功能描述与实现状态表；核对功能状态或向他人介绍应用时读
- `kb/docs/claude-session-data-format.md` — `~/.claude` 数据源与 JSONL 记录类型实测；改解析器、派生字段、history 回退逻辑前读
- `kb/docs/data-layer-design.md` — DDL 与 serde 类型草案；改 schema、索引器、增量更新、搜索前读
- `kb/docs/gpui-kit-guide.md` — 本项目实际采用的 gpui-kit 做法、已核实的 API、绕不开的限制；改界面代码前读
- `kb/docs/gpui-framework-research.md` — 开工前的调研：GPUI 心智模型、组件目录、选型理由；用到还没用过的组件或重新评估依赖时读
- `kb/notes/2026-09-15-m0-spike.md` — M0：环境准备与构建耗时
- `kb/notes/2026-09-16-design-refactor.md` — 界面重构记录：设计稿链接、与设计稿的功能差异
- `kb/notes/2026-09-17-m1-core-crate.md` — M1：core 模块划分、关键实现细节、与设计文档的差异、实测数字、Python 比对方法；改 core 前读
- `kb/notes/2026-09-21-m2-real-data.md` — M2：store 线程与事件流、流式扫描时序、视图状态持久化、相对 PRD 未做的条目；改 `store.rs` / `main.rs` 数据流前读
- `kb/notes/2026-09-21-m3-search.md` — M3：搜索线程与取代机制、两段式 FTS 的原因与数字、`Matcher`、Matches 列 / 片段 / 详情跳转、实测；改 `search.rs`、`fts.rs`、`regex.rs::scan_sessions` 或搜索界面前读
- `kb/plans/2026-09-28-m4-detail-panel.md` — M4（详情面板，R6）实现方案：现状、实测、数据通路与类型设计、11 个步骤及各自的行为测试、验收规则；开工前先看第 1 节的待确认项 D1–D6 是否已有答复
