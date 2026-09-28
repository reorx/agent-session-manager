# Known Issues

> 当前 app 已知、待后续处理的问题：缺陷、简化实现、未验收项、待建流程。只反映最新状态——解决了就删掉条目，经过记到 `kb/sessions/`。

## CLI：`asm --db` 帮助文本里的默认索引路径与实际不符

- 记录：2026-09-28（精简 AGENTS.md 时核对代码发现）
- `crates/core/src/bin/asm.rs` 的 `--db` 说明写的是 `~/Library/Application Support/AgentSessionManager/index.sqlite`，实际默认值来自 `claude/paths.rs` 的 `ProjectDirs::from("dev", "reorx", "AgentSessionManager")`，即 `~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite`。只影响 `--help` 的显示，行为正确。
- 处理方向：改帮助文本；CLI 测试若断言了 help 输出要一并更新。

## 打包：已定的 bundle id 与数据目录的限定符不一致

- 记录：2026-09-28（精简 AGENTS.md 时核对代码发现）
- PRD 已定 bundle id 为 `com.reorx.agent-session-manager`，而索引与 `state.json` 所在目录是 `dev.reorx.AgentSessionManager`。目前还没有 `.app` bundle，所以没有实际影响；M5 用 cargo-bundle 打包后，app 的标识与它的数据目录会对不上。
- 处理方向：M5 打包前决定是否统一。改数据目录名会让现有索引和视图状态落到新路径，索引可重建，`state.json` 需要迁移或接受重置。
