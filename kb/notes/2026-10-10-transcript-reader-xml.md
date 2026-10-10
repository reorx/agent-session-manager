---
created: 2026-10-10
tags:
  - cli
  - transcript
  - xml
---

# Transcript Reader：XML 标签组织的 TXT

原 Markdown 导出的 `## User` / `## Assistant` 会与对话正文的 Markdown 标题混淆。现在统一输出 UTF-8 TXT，用 XML 风格标签分隔角色和工具摘要，供 agent 阅读，不要求符合标准 XML。压缩规则不变。

## 格式契约

```text
<transcript session_id="example">

<user>
请修复这个问题
</user>

<turn-summary>
Called Bash 1 time, Read 3 times.
</turn-summary>
<agent>
## 修改结果

已完成。
</agent>

</transcript>
```

- 外层 `transcript` 带文件 stem 作为 `session_id`。
- 每个起始和结束标签独占一行，正文或摘要放在两者之间，不额外缩进。
- `user` 与 `agent` 按对话顺序排列。正文直接保留 Markdown、HTML、XML 示例，不加 CDATA，也不转义实体；正文里的同名标签也原样保留，由阅读的 agent 结合上下文理解。
- `turn-summary` 紧邻其所属的 `agent` 之前，用一句纯文本列出按名称排序的工具和调用次数，不再生成工具子元素；没有调用时省略整个摘要。单次为 `1 time`，多次为 `N times`。
- 未记录最终回复时，使用 `<agent status="missing">` 和简短文字标记。
- 正文和摘要中的 `&`、`<`、`>`、`]]>` 原样输出，不做 XML 转义或拆分。只对 session_id 属性中的特殊字符与换行进行实体编码，保持引号边界清晰；不可读控制字符与 U+FFFE/U+FFFF 仍用 U+FFFD 替换。
- CLI stdout 是此文本，`-o session.txt` 保存新文件；扩展名不强制限制，仍拒绝覆盖已有文件。
- core API 改为 `transcript::read_transcript` → `model::TranscriptExport { text, skipped_lines }`，GUI 未来复用此接口。

## 构建与使用

```sh
make build-transcript-reader
./dist/transcript-reader SESSION_ID -o session.txt
make asm ARGS="transcript SESSION_ID" > session.txt
```

二进制位于 `dist/transcript-reader`，不纳入 Git。现有全局安装需重新 `cargo install --path crates/core --bin transcript-reader` 才会更新。

## 验证

以下记录前几版的格式及验收过程；当前纯文本格式以顶部契约为准。

先更新格式行为测试，确认旧 API 不能满足测试，再实现；新增标题/XML/CDATA 冲突和非法控制字符用例。core 全部 79 个测试通过（导出相关 8 个）。实际导出验收由 Herdr 独立 Codex tab 执行，产物归档在 `tmp/2026-10-10-transcript-reader-xml/`。

独立验收通过：fixture、真实 transcript、特殊字符和非法控制字符样本均通过 Python 标准库 XML 解析；正文中的标题、结束标签、重复 CDATA 结束符解码后完整还原，属性值正确还原，stdout 与文件输出逐字节一致。真实导出包含 2 条用户输入、2 条回复、61 次工具调用，详见目录内 `report.json` 与 `real-session.txt`。未启动 GUI、浏览器或模拟器。

再次运行 `make parity` 为 20/21 通过，唯一差异仍是已登记的空 session 并列排序问题，与初版一致。

初版实现、压缩语义与历史验证见 [Transcript Reader 初版](2026-10-10-transcript-reader.md)。

## 标签与摘要调整

按用户要求，将导出角色标签改为 `agent`，工具摘要改为纯文本 `turn-summary`，不再嵌套工具元素；输入 JSONL 中的 `assistant` 记录类型不变。上述格式契约与 README 已同步更新。

先修改行为测试，确认旧实现有 4 项失败，再实现新格式。额外用失败测试覆盖摘要里的回车，实体编码防止 XML 解析器将其归一化为换行。core 全部 79 个测试通过。

独立 release CLI 验收使用标准库 XML 解析，逐项对照先前归档的 fixture、真实会话、特殊字符及控制字符导出：正文和缺失状态完全一致，工具名称和次数完整保留，摘要没有子元素，stdout 与 `-o` 输出逐字节一致。已阅读 fixture 和真实导出；验收脚本、报告及导出归档在 `tmp/2026-10-10-transcript-reader-tags/`。已重建 `dist/transcript-reader`；未启动 GUI、浏览器或模拟器。

本次 `make parity` 仍为 20/21 通过；核对 diff，唯一失败仍是两组空 session 的并列排序差异，已登记在 `kb/known-issues.md`。完整日志见上述验收目录的 `parity.log`。

## 标签分行排版

根据后续反馈，所有结构标签的起始和结束标签各占一行，用户正文、最终回复、缺失回复标记与纯文本工具摘要统一分行。CDATA 内容和原有 Markdown 不额外缩进；解码后的元素文本两端新增一个排版换行。

先更新行为测试，确认旧排版有 3 项失败，再修改输出。core 79 个测试通过。release CLI 的 fixture、真实会话、特殊字符及控制字符样本均可被标准库 XML 解析；与原归档比较，正文仅新增两端排版换行，摘要仍无子元素，stdout 和文件输出一致。已阅读 fixture 和真实导出。产物在 `tmp/2026-10-10-transcript-reader-lines/`，已重建 `dist/transcript-reader`。

本次 `make parity` 为 20/21 通过，唯一失败仍是已登记的空 session 排序差异；日志在上述目录的 `parity.log`。

## 移除 CDATA，标签只作分隔

用户明确导出供 agent 阅读，只需标签分隔，不要求标准 XML。去掉所有生成的 CDATA 包裹和结束符拆分，正文和工具摘要原样保留 `<`、`&`、`]]>`、标签示例及 Markdown；继续保持结构标签独占一行。session_id 属性仍转义，不可读控制字符仍替换 U+FFFD。

先更新行为测试，确认旧实现有 3 项失败，再简化导出实现。core 79 个测试通过，README、CLI 帮助、Makefile 与 AGENTS.md 已同步说明新格式；`make parity` 仍为 20/21 通过，唯一差异是已登记的空 session 排序，日志在 `tmp/2026-10-10-transcript-reader-plain/parity.log`。

验收前 context 为 32%，按项目规则使用 Herdr 新 Codex tab 做独立 CLI 验收。76 项检查、44 次命令调用通过：fixture 逐字节一致，真实会话的 2 条用户输入、2 条回复、61 次工具调用与旧导出的解码内容一致；特殊字符、字面 CDATA 示例、缺失回复、控制字符、两个 CLI 入口及 stdout/文件输出均符合契约，拒绝覆盖已有文件。已阅读 fixture、真实导出与特殊字符样本。脚本、导出、日志及 `report.json` 归档在上述目录，未启动 GUI、浏览器或模拟器。
