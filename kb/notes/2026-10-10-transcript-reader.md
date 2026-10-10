---
created: 2026-10-10
tags:
  - cli
  - transcript
  - markdown
---

# Transcript Reader

> 本文记录初版 Markdown 导出。当前已改为 XML 标签组织的 TXT，API 为 `read_transcript` / `TranscriptExport`，见 [格式更新](2026-10-10-transcript-reader-xml.md)。

为 Session 交接提供精简 Markdown。用户确认“Cloud Session ID”指本机 Claude Code Session ID。此次完成独立 `transcript-reader` CLI、`asm transcript` 子命令及 core 复用接口；GUI 导出按钮留待 M4 接入。

## 使用与接口

- `make transcript-reader ARGS="SESSION_ID -o session.md"`；也可 `make asm ARGS="transcript SESSION_ID" > session.md`。
- `cargo install --path crates/core --bin transcript-reader` 安装独立命令。
- 输入支持主 transcript 完整 ID、唯一前缀或文件路径。精确 ID 优先，存在多个候选时报错并列路径。subagent 用 `subagents/*.jsonl` 路径。
- 数据目录遵循 `--claude-dir` / `ASM_CLAUDE_DIR` / `CLAUDE_CONFIG_DIR` / `~/.claude`。直接读源文件，不创建/修改 SQLite 索引。
- stdout 只写 Markdown，诊断走 stderr。`-o` 使用 `create_new`，拒绝覆盖任何已有文件。
- `transcript::resolve_transcript` 负责定位；`transcript::read_markdown` 返回 `model::MarkdownExport { markdown, skipped_lines }`。GUI 已知路径时只调用后者，放后台线程执行。

## 压缩语义

复用 `parse_content` / `user_text` / `clean_prompt`，不改变现有索引解析器。

1. 保留清理后的真实用户输入；过滤 isMeta、compact summary、非 human origin、tool_result 及其伴随文本。图片和文档用占位符保留其存在。
2. 保留最终助手回复的 Markdown、代码块、表格，不截断正文。每轮工具按名称排序并计数，放在回复前一行。
3. 去掉 thinking、工具输入输出、中间进度文本。同一 message ID 的分块文本合并；工具调用清除之前的中间文本；重复 UUID 不重复计数。
4. 下一条真实用户输入、system/turn_duration 或 EOF 结束当前轮。显式 end_turn 的回复在下一条不同 API message 开始前单独保留，避免自动续跑覆盖最终回复。
5. 旧记录无 stop_reason 时，以轮次边界前最后一条不含工具的文本消息推断最终回复。只有工具或中断标志时输出 `[No final response recorded]`，不把已知中断文本当最终答案。
6. 按文件顺序读取，不做 parentUuid 分支选择；compact summary 不重复导出。完整但没有末尾换行的 JSON 可以读取；损坏/截断记录计入 skipped_lines，CLI 给警告。

这是确定性筛选，不调用 LLM，不保证所有会话固定达到 90% 对话占比；极短回答与长工具名称列表会影响比例。只有 history 的 session 无法恢复已删除的 transcript。没有 stop_reason 的正在进行的文本可能被当作最终文本。

## 验证

- BDD：先写测试并确认缺少模块导致失败，再实现。补充自动续跑、中断、subagent 路径的失败测试后修复。
- `mise exec -- cargo test -p agent-session-core`：77 个测试通过，其中新增 6 个，覆盖主要压缩规则、分块回复、ID 定位、歧义、文件输出保护、损坏尾行与两个 CLI 入口一致性。
- `mise exec -- cargo build --release -p agent-session-core --bins` 通过。
- 本项目最近 5 份真实 transcript：全部显式 end_turn 文本块逐一确认出现在导出结果中；输出 2.7–14.5 KB，原文件 0.30–11.03 MB；字节体积减少 99.12–99.93%，对话占导出字节约 96.24–97.68%。耗时含启动、ID 定位与解析 12.5–20.6ms（热缓存）。比例按字节计算，非模型 token 数。
- 人工读过三轮的导出 `sample.md`，用户输入、各轮最终回复、表格与代码块保持可读。
- `make parity`：20/21 通过。唯一失败 `list-code-30d` 是两组没有 started_at 的空 session 排序不同；涉及的索引/排序代码本次未改。新导出命令没有 Python 对照实现。
- 验收文件、JSON 指标、脚本、日志与真实导出均在 `tmp/2026-10-10-transcript-reader/`，parity 原始结果在 `tmp/2026-10-10-python-parity/`。未启动 GUI/浏览器/模拟器，无需截图或清理此类资源。

## 后续 GUI 接入

M4 的导出按钮可以传入选中 session 的 transcript 路径，后台运行 `read_markdown`，展示 skipped_lines 提示后保存 Markdown；不应从现有前 300 条索引消息生成导出。
