# Agent Session Manager

macOS 桌面应用（Rust + GPUI / gpui-kit），以表格形式列出本机所有 Claude Code session，支持按项目 / 时间过滤、表头排序、按角色范围全文搜索、可切换的行展示方式与详情面板。核心诉求是界面响应与查询速度。

## 当前状态

2026-09-17：**M1 完成**（核心 crate，`kb/notes/2026-09-17-m1-core-crate.md`）。`crates/core` 现在是完整数据层：字节预过滤 + 借用式 serde 的 JSONL 解析器、增量 SQLite 索引（`index_state` 状态机、history.jsonl 补位、损坏 / 版本不符自动重建）、FTS5 trigram 搜索（<3 字 token 走 LIKE）、对齐 Python 脚本的正则扫描，以及 `asm` CLI。61 个 BDD 场景 + 真实数据上与 `claude_session_search.py` 21 组逐字比对全部通过；本机 1.1GB 全量索引 2.1–2.4s（其中 FTS 建索引 1.6s），增量扫描 0.14s，正则扫全量 0.4s，FTS 查询 3–11ms。

界面（2026-09-16 按 Claude Design 设计稿重构，`kb/notes/2026-09-16-design-refactor.md`）**仍跑在 `crates/app/src/fake.rs` 的 10k 假行上**，尚未接入 core。下一步是 PRD 的 M2：用 `Db::list_sessions` 填 `SessionStore`、流式索引进度、真实的过滤 / 排序。

## 开发命令

常用命令都在 `Makefile`，`make help` 列出；README 有说明。要点：

- `make dev`（别名 `make dv`）编译并启动 app，与其他 macOS 项目的 `dev: build run` 约定一致；`run` 会先 pkill 掉已有实例，所以看到的一定是刚编出来的版本
- `make screenshot` 唤醒屏幕、带 `ASM_SPIKE_AUTOPLAY=1` 启动并按窗口 ID 截三张图到 `tmp/<date>-screenshots/`（`SHOT_DIR=` 可改）；`make screenshot-clicks` 用 cliclick 真点击驱动下拉 / tab / 行展开，截 12 张（坐标写在 `scripts/screenshot-clicks.sh`，改布局后要校对）
- 构建产物不在 `./target`：全局 `~/.cargo/config.toml` 把 target 指到 `~/Library/Caches/cargo-target`，`make bin` 打印二进制路径。首次构建约 1.5 分钟，增量约 2 秒；首次 `cargo check` 单独要 40 秒
- Rust 由 mise 管理（`mise.toml` 锁 1.98.1，gpui-pre 需要 ≥ 1.98）；Metal Toolchain 必须装（`make setup`）
- 手动启动 app 做验证后必须 `kill` 掉进程；`pgrep -fl agent-session-manager` 检查（`make dev` 自带 pkill，手动起的不算）
- `asm` CLI（`cargo run -p agent-session-core --bin asm -- …` 或 `make asm ARGS="…"`）：`index [--rebuild]`、`list [PATH|SUBSTR] [-d DAYS] [-n N] [--full]`、`session ID`、`search QUERY [-d 7] [-p SUBSTR] [-r user|assistant|tool]… [-l LABEL]… [--case-sensitive] [-C 80] [-n 200] [--index]`、`stats`。环境变量 `ASM_CLAUDE_DIR`（默认 `$CLAUDE_CONFIG_DIR` 或 `~/.claude`）、`ASM_DB`（默认 `~/Library/Application Support/AgentSessionManager/index.sqlite`）、`NO_COLOR`。`list` / `session` / `search --index` 会先做一次增量扫描（`--no-scan` 跳过）
- `make parity` 在真实 `~/.claude` 上把 `asm` 与 Python 脚本逐字比对（`scripts/compare-with-python.sh`，索引写到 `tmp/<date>-python-parity/`）；改动解析器、清理逻辑或输出格式后必须跑
- core 的测试：`cargo test -p agent-session-core`（1 秒内）；fixture 改动要重跑 `uv run crates/core/tests/fixtures/gen_fixtures.py` 并提交生成结果

## 验收流程

1. 截图前先 `wake-screen`（`make screenshot` 已包含），熄屏时截图是黑的
2. 截图必须用 `Read` 打开看过再下结论，不要只看文件大小
3. 通过后把截图归档到 `tmp/<date>-<task>/` 并在报告里给出路径；`tmp/` 在 gitignore 里，不会入库
4. 每个里程碑完成后更新本文件的"当前状态"、PRD 里程碑表，然后 commit

## core crate 编码规则

- `crates/core` 不依赖 gpui；对外类型在 `model.rs`（`SessionSummary` 即 `sessions` 表一行，`MessageRow` 即 `messages` 表一行）。UI 层只消费这些类型，不自己解析 JSONL
- 解析器先写行为测试：fixture 在 `crates/core/tests/fixtures/claude/`，由同目录 `gen_fixtures.py` 生成；新遇到的记录类型先加进生成脚本再改 `parse.rs`
- `parse.rs::classify` 的字节预过滤靠"前 512 字节里第一个 `"type":`；值是 `message` 就找 `"role":`；含 `"attachment":` 就跳过"。改它之前用 `kb/notes/2026-09-17-m1-core-crate.md` 里的方法在真实数据全量验证
- serde_json 的 `&RawValue` 不能放在 `#[serde(tag)]` / `untagged` / `flatten` 之下；记录和 block 都用"大结构体 + Option 字段 + match type"的写法
- 改 `schema.sql` 必须同步 `db.rs` 的 `SCHEMA_VERSION`、`SESSION_COLUMNS` 与 `row_to_session` 的列序；索引版本不符会整库重建，无迁移
- 一个 `Db` 只在一个线程用；rayon 只用于解析，写库在调用线程的单个事务里
- `SessionDelta::apply` 必须保持"全量解析 == 多次增量解析"，有测试断言相等；新字段要归类为 first（只设一次）、last（覆盖）或 count（累加）

## GPUI / gpui-kit 编码规则

- 只依赖 `gpui-kit = "=0.6.1"`，**不要**再声明 `gpui` 或 `gpui-pre`，否则两份 GPUI 类型不匹配。`use gpui_kit::*` 拿到 gpui；组件在 `gpui_kit::component::{table, tab, date_picker, resizable, status_bar, ...}`；Action 用 `gpui_kit::actions!` 宏
- **API 以本地源码为准**，调研文档和 docs.rs 都有偏差。查签名直接 grep registry 源码：
  `~/.cargo/registry/src/index.crates.io-*/{gpui-component-0.6.1,gpui-base-0.6.1,gpui-pre-0.3.5}/src`，例如 `grep -n "pub fn " .../gpui-component-0.6.1/src/table/state.rs`。已核实过的差异记在 `kb/notes/2026-09-15-m0-spike.md`
- 表格模式：delegate 持 `rows: Vec<Arc<Row>>` + `ix: Vec<usize>`（过滤 + 排序后的索引），`render_td` 只取预格式化好的 `SharedString`，不做任何计算；`perform_sort` 原地排 `ix`，同值按 updated 降序保证稳定
- 几千行的过滤 / 排序直接在主线程做（10k 行排序 7ms），只有 SQLite 查询和文件扫描才走后台
- `Task` drop 即取消：`cx.spawn` 的结果要 `.detach()` 或存进 `_task` 字段；`this.update(cx, ..)` 返回 Result，用 `.ok()` 不要 unwrap
- 需要脚本化交互（截图、验收）时优先加环境变量驱动的 autoplay 逻辑；弹出层（下拉菜单、popover）无法程序化打开，才用 `scripts/screenshot-clicks.sh` 的 cliclick 点击
- 主题与密度：`style::install_theme` 把设计稿色值写进 gpui-kit `Theme`（rem = 12px，组件内部尺寸随之缩小），改 `colors` 后必须重建 `tokens` 并 `Theme::sync_base`；DataTable 选中行是盖在单元格上面的覆盖层，`table_active` 必须半透明
- 设计稿色值集中在 `style.rs`，不要在别处写 hex；`impl IntoElement` 返回值带 `cx` 参数时用 `-> Div` / `AnyElement` / `+ use<>` 避免借用冲突

## 关键决策

- UI 依赖 `gpui-kit = "=0.6.1"`（精确锁版，提交 Cargo.lock），不直接依赖 `gpui`；Zed 的 `ui` / `theme` crate 是 GPL 且未发布，只读源码不依赖
  - GPUI 本身是 Zed Industries 的（crates.io owner 为 zed-industries 团队），但官方 crates.io 版本停在 2025-10-22 的 0.2.2（平台层拆分之前），main 新增的 `gpui_platform` 等 crate 标记 `publish = false` 未发布。gpui-kit 走的是第三方 republish 的 `gpui-pre` 快照。三条路的对比与选择理由见 `kb/docs/gpui-framework-research.md` §1.1 / §1.3
- bundle id `com.reorx.agent-session-manager`；SDK 与后台 session 不隐藏，用 Kind 列徽章区分
- 数据层：SQLite（rusqlite bundled，WAL，单写线程）+ FTS5 trigram 只索引 user / assistant 文本（约 20MB）；正则模式用 memmap2 + memchr + `regex` 扫原始文件（在 JSON 解码后的文本上匹配，见 M1 笔记）
- 核心逻辑放独立 crate `crates/core`（无 gpui 依赖，带 `asm` CLI），UI 层 `crates/app` 保持薄

## 参考

- 现有 Python 实现：`~/Code/scripts/claude_session_search.py`（搜索 + `--list` + `-s`，本项目的行为基准；`make parity` 逐字比对）
- 旧项目 `~/Code/agent-sessions`：transcript → 静态站点 SSG，含 Codex parser，二期多 agent 支持的参考

## Documentation

- `kb/docs/product-prd.md` — 面向产品视角的 PRD：产品定位、三段式界面架构、各功能的交互描述与当前实现状态（✅ / 🟡 / ⬜）；向他人介绍应用或核对功能状态时先读，每个里程碑完成后同步更新状态表
- `kb/docs/claude-session-data-format.md` — 修改解析器、派生字段、history.jsonl 回退逻辑或遇到未知记录类型时先读；含 `~/.claude` 全部数据源与 JSONL 记录类型实测
- `kb/docs/gpui-framework-research.md` — 写任何 GPUI / gpui-kit 代码前读；含 API 签名、DataTable delegate、异步加载模式、坑清单、依赖锁版策略
- `kb/docs/data-layer-design.md` — 改动 SQLite schema、索引器、增量更新、搜索（FTS5 / LIKE / 正则）时读；含 DDL 与 serde 类型草案
- `kb/notes/2026-09-15-m0-spike.md` — M0 实测：构建耗时、与调研文档的 API 差异、对 M1/M2 的影响（表格横向虚拟化裁列、程序化排序不更新表头）
- `kb/notes/2026-09-17-m1-core-crate.md` — M1 记录：core 模块划分、字节预过滤规则、增量状态机、与设计文档的差异（无 grep-searcher、chrono 代替 jiff、subagent 只计数）、真实数据实测、Python 比对方法与已知行为边界；改 core 前读
- `kb/notes/2026-09-16-design-refactor.md` — 界面重构：设计稿链接、`crates/app/src` 模块划分、gpui-kit 主题 / 下拉 / 选中覆盖层 / Preview 自适应列宽的做法、与设计稿的已知差异
- `kb/plans/2026-09-08-agent-session-manager-prd.md` — 需求 R1–R13、架构、里程碑 M0–M5、风险与开放问题；开发任务的总入口
