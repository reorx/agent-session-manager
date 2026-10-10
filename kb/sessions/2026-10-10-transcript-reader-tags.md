---
created: 2026-10-10
tags:
  - transcript
  - cli
  - xml
---

# Transcript Reader 标签和纯文本工具摘要

## 概要

按用户要求将最终回复标签从 `assistant` 改为 `agent`，工具调用摘要从嵌套的 `tool_calls/tool` 改为 `turn-summary` 中的一句纯文本，例如 `Called Bash 2 times, Read 1 time.`。工具按名称排序，没有调用时省略摘要；缺失回复继续使用 `status="missing"`。

## 修改的文件

- `crates/core/src/transcript.rs`：修改导出标签与摘要，复用 XML 文本转义保护工具名称。
- `crates/core/tests/transcript_reader.rs`：先更新行为测试，覆盖新格式、缺失回复、自动续跑、subagent、特殊字符和回车。
- `README.md`、`kb/notes/2026-10-10-transcript-reader-xml.md`：同步当前导出格式。

## 注意事项

- 只改导出标签，源 JSONL 的 `assistant` 类型保持原有语义。
- 摘要使用 XML 实体编码，不含子元素；正文继续使用 CDATA，保留 Markdown 和标签示例。
- XML 解析器会归一化文本里的裸回车，摘要用 `&#13;` 保留它；非法控制字符仍替换为 U+FFFD。
- core 79 个测试全部通过。release CLI 的 fixture、真实会话及特殊字符输出通过标准库 XML 解析，与旧导出解码后的正文、工具名称和次数一致；stdout 与文件输出一致。
- 已重建 `dist/transcript-reader`，未更新全局安装的命令。验收产物在 `tmp/2026-10-10-transcript-reader-tags/`；本次无需 GUI 截图或测试资源清理。

## 遗留问题

无本次新增问题。`make parity` 20/21 通过，唯一失败仍是已登记的 `list-code-30d` 两组空 session 并列排序差异；核对 diff 与之前记录一致，未改排序逻辑。完整日志在验收目录的 `parity.log`。

## 相关文档

- [当前 TXT 导出格式](../notes/2026-10-10-transcript-reader-xml.md) — 更新标签、摘要与验证结果。
- [Transcript Reader 初版](../notes/2026-10-10-transcript-reader.md) — 参考压缩语义及既有 parity 差异。
- [M1 core crate](../notes/2026-09-17-m1-core-crate.md) — 参考测试与对照验证规则。
