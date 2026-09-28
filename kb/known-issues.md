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

## 详情：索引扫描进行中选中一行，对话要等扫描结束才出现

- 记录：2026-09-28（M4 方案调研时读代码发现；`kb/plans/2026-09-28-m4-detail-panel.md` §2）
- store 线程是单循环，`StoreRequest::Messages` 排在扫描后面。冷启动或 Rebuild 的 3.6–4s 内选中行，Conversation 一直是空的。
- 处理方向：M4 步骤 2 给 transcript 读取单开一条线程。

## 详情：Conversation 只显示索引里的前 300 条 user / assistant 文本，每条截到 4000 字

- 记录：2026-09-28（M2 的过渡实现，有意为之；`kb/notes/2026-09-21-m2-real-data.md`）
- 该视图没有虚拟化，全量渲染上千条会卡，所以设了上限。工具调用、thinking 不显示，没有 Markdown 渲染。搜索命中落在上限之外时靠两个补丁移动窗口和截断位置。
- 处理方向：M4 步骤 1–3 直接读 transcript 并虚拟化，届时删掉上限和两个补丁。

## 搜索：工具调用与 thinking 里的命中在详情面板里看不到

- 记录：2026-09-28（M3 遗留；`kb/notes/2026-09-21-m3-search.md`"已知问题与未做"）
- 正则 + Tools 范围或选了 Labels 时，命中只在表格的片段里可见，详情面板底部有一句说明。
- 处理方向：M4 步骤 4。

## 文档：数据格式调研里的本机规模数字已过时

- 记录：2026-09-28（M4 方案调研时实测）
- `kb/docs/claude-session-data-format.md` 写的是 654 份主 transcript、最大单文件 30MB；现在是 1159 份、最大 70MB，subagent transcript 368 份。不影响功能，但按这些数字估算性能会偏乐观。
- 处理方向：下次改动该文档时重新统计。
