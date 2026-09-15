# Agent Session Manager

macOS 桌面应用（Rust + GPUI / gpui-kit），以表格形式列出本机所有 Claude Code session，支持按项目 / 时间过滤、表头排序、按角色范围全文搜索、可切换的行展示方式与详情面板。核心诉求是界面响应与查询速度。

## 当前状态

2026-09-15：**M0 完成**（工具链 + gpui-kit spike，记录见 `kb/notes/2026-09-15-m0-spike.md`）。`crates/core` 是空壳，`crates/app` 是 10k 假行的 DataTable spike。下一步是 PRD 的 M1（核心 crate：解析器、SQLite 索引、搜索、`asm` CLI）。

## 开发命令

常用命令都在 `Makefile`，`make help` 列出；README 有说明。要点：

- `make dev`（别名 `make dv`）编译并启动 app，与其他 macOS 项目的 `dev: build run` 约定一致；`run` 会先 pkill 掉已有实例，所以看到的一定是刚编出来的版本
- `make screenshot` 唤醒屏幕、带 `ASM_SPIKE_AUTOPLAY=1` 启动并按窗口 ID 截三张图到 `tmp/<date>-screenshots/`（`SHOT_DIR=` 可改）
- 构建产物不在 `./target`：全局 `~/.cargo/config.toml` 把 target 指到 `~/Library/Caches/cargo-target`，`make bin` 打印二进制路径。首次构建约 1.5 分钟，增量约 2 秒；首次 `cargo check` 单独要 40 秒
- Rust 由 mise 管理（`mise.toml` 锁 1.98.1，gpui-pre 需要 ≥ 1.98）；Metal Toolchain 必须装（`make setup`）
- 手动启动 app 做验证后必须 `kill` 掉进程；`pgrep -fl agent-session-manager` 检查（`make dev` 自带 pkill，手动起的不算）

## 验收流程

1. 截图前先 `wake-screen`（`make screenshot` 已包含），熄屏时截图是黑的
2. 截图必须用 `Read` 打开看过再下结论，不要只看文件大小
3. 通过后把截图归档到 `tmp/<date>-<task>/` 并在报告里给出路径；`tmp/` 在 gitignore 里，不会入库
4. 每个里程碑完成后更新本文件的"当前状态"、PRD 里程碑表，然后 commit

## GPUI / gpui-kit 编码规则

- 只依赖 `gpui-kit = "=0.6.1"`，**不要**再声明 `gpui` 或 `gpui-pre`，否则两份 GPUI 类型不匹配。`use gpui_kit::*` 拿到 gpui；组件在 `gpui_kit::component::{table, tab, date_picker, resizable, status_bar, ...}`；Action 用 `gpui_kit::actions!` 宏
- **API 以本地源码为准**，调研文档和 docs.rs 都有偏差。查签名直接 grep registry 源码：
  `~/.cargo/registry/src/index.crates.io-*/{gpui-component-0.6.1,gpui-base-0.6.1,gpui-pre-0.3.5}/src`，例如 `grep -n "pub fn " .../gpui-component-0.6.1/src/table/state.rs`。已核实过的差异记在 `kb/notes/2026-09-15-m0-spike.md`
- 表格模式：delegate 持 `rows: Vec<Arc<Row>>` + `ix: Vec<usize>`（过滤 + 排序后的索引），`render_td` 只取预格式化好的 `SharedString`，不做任何计算；`perform_sort` 原地排 `ix`，同值按 updated 降序保证稳定
- 几千行的过滤 / 排序直接在主线程做（10k 行排序 7ms），只有 SQLite 查询和文件扫描才走后台
- `Task` drop 即取消：`cx.spawn` 的结果要 `.detach()` 或存进 `_task` 字段；`this.update(cx, ..)` 返回 Result，用 `.ok()` 不要 unwrap
- 需要脚本化交互（截图、验收）时优先加环境变量驱动的 autoplay 逻辑，不要依赖 GUI 自动化

## 关键决策

- UI 依赖 `gpui-kit = "=0.6.1"`（精确锁版，提交 Cargo.lock），不直接依赖 `gpui`；Zed 的 `ui` / `theme` crate 是 GPL 且未发布，只读源码不依赖
  - GPUI 本身是 Zed Industries 的（crates.io owner 为 zed-industries 团队），但官方 crates.io 版本停在 2025-10-22 的 0.2.2（平台层拆分之前），main 新增的 `gpui_platform` 等 crate 标记 `publish = false` 未发布。gpui-kit 走的是第三方 republish 的 `gpui-pre` 快照。三条路的对比与选择理由见 `kb/docs/gpui-framework-research.md` §1.1 / §1.3
- bundle id `com.reorx.agent-session-manager`；SDK 与后台 session 不隐藏，用 Kind 列徽章区分
- 数据层：SQLite（rusqlite bundled，WAL，单写线程）+ FTS5 trigram 只索引 user / assistant 文本（约 20MB）；正则模式用 grep-searcher 扫原始文件
- 核心逻辑放独立 crate `crates/core`（无 gpui 依赖，带 `asm` CLI），UI 层 `crates/app` 保持薄

## 参考

- 现有 Python 实现：`~/Code/scripts/claude_session_search.py`（搜索 + `--list`，本项目的行为基准）
- 旧项目 `~/Code/agent-sessions`：transcript → 静态站点 SSG，含 Codex parser，二期多 agent 支持的参考

## Documentation

- `kb/docs/claude-session-data-format.md` — 修改解析器、派生字段、history.jsonl 回退逻辑或遇到未知记录类型时先读；含 `~/.claude` 全部数据源与 JSONL 记录类型实测
- `kb/docs/gpui-framework-research.md` — 写任何 GPUI / gpui-kit 代码前读；含 API 签名、DataTable delegate、异步加载模式、坑清单、依赖锁版策略
- `kb/docs/data-layer-design.md` — 改动 SQLite schema、索引器、增量更新、搜索（FTS5 / LIKE / 正则）时读；含 DDL 与 serde 类型草案
- `kb/notes/2026-09-15-m0-spike.md` — M0 实测：构建耗时、与调研文档的 API 差异、对 M1/M2 的影响（表格横向虚拟化裁列、程序化排序不更新表头）
- `kb/plans/2026-09-08-agent-session-manager-prd.md` — 需求 R1–R13、架构、里程碑 M0–M5、风险与开放问题；开发任务的总入口
