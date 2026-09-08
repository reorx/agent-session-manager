# Agent Session Manager

macOS 桌面应用（Rust + GPUI / gpui-kit），以表格形式列出本机所有 Claude Code session，支持按项目 / 时间过滤、表头排序、按角色范围全文搜索、可切换的行展示方式与详情面板。核心诉求是界面响应与查询速度。

## 当前状态

2026-09-08：完成前期调研与 PRD，**尚未编写任何代码**。下一步是 PRD 里的 M0（工具链准备 + gpui-kit spike）。

## 关键决策

- UI 依赖 `gpui-kit = "=0.6.0"`（精确锁版，提交 Cargo.lock），不直接依赖 `gpui`；Zed 的 `ui` / `theme` crate 是 GPL 且未发布，只读源码不依赖
- 数据层：SQLite（rusqlite bundled，WAL，单写线程）+ FTS5 trigram 只索引 user / assistant 文本（约 20MB）；正则模式用 grep-searcher 扫原始文件
- 核心逻辑放独立 crate `crates/core`（无 gpui 依赖，带 `asm` CLI），UI 层 `crates/app` 保持薄
- 开工前必须：rustup 最新 stable（Homebrew 1.91 编不过 gpui-pre）、`xcodebuild -downloadComponent MetalToolchain`（或 `runtime_shaders` feature）

## 参考

- 现有 Python 实现：`~/Code/scripts/claude_session_search.py`（搜索 + `--list`，本项目的行为基准）
- 旧项目 `~/Code/agent-sessions`：transcript → 静态站点 SSG，含 Codex parser，二期多 agent 支持的参考

## Documentation

- `kb/docs/claude-session-data-format.md` — 修改解析器、派生字段、history.jsonl 回退逻辑或遇到未知记录类型时先读；含 `~/.claude` 全部数据源与 JSONL 记录类型实测
- `kb/docs/gpui-framework-research.md` — 写任何 GPUI / gpui-kit 代码前读；含 API 签名、DataTable delegate、异步加载模式、坑清单、依赖锁版策略
- `kb/docs/data-layer-design.md` — 改动 SQLite schema、索引器、增量更新、搜索（FTS5 / LIKE / 正则）时读；含 DDL 与 serde 类型草案
- `kb/plans/2026-09-08-agent-session-manager-prd.md` — 需求 R1–R13、架构、里程碑 M0–M5、风险与开放问题；开发任务的总入口
