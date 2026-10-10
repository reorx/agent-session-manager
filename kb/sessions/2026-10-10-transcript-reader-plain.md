---
created: 2026-10-10
tags:
  - transcript
  - cli
  - text-export
---

# Transcript Reader 去掉 CDATA 包裹

## 概要

按用户要求，让 XML 风格标签仅作内容分隔。去掉导出生成的 CDATA 包裹与拆分逻辑，直接输出正文和工具摘要的原始文本，保留结构标签各占一行的排版。

## 修改的文件

- `crates/core/src/transcript.rs`：正文与摘要不再生成 CDATA 或 XML 实体；保留 session_id 属性转义及不可读控制字符替换。
- `crates/core/tests/transcript_reader.rs`：先更新行为测试，覆盖原始 Markdown、同名标签、实体文本、CDATA 结束符、特殊工具名和缺失回复。
- 两个 CLI 入口及共享参数模块、`Makefile`、`README.md`、`AGENTS.md`：将格式说明更新为 XML 风格标签分隔的纯文本。
- `kb/notes/2026-10-10-transcript-reader-xml.md`：更新格式契约和示例，并注明先前标准 XML 验证为历史记录。

## 注意事项

输出面向 agent 阅读，正文中的同名标签和字面 CDATA 示例也保留原样，不再要求标准 XML。core 79 个测试通过，已重建 `dist/transcript-reader`，全局安装未更新。

验收前 context 为 32%，按项目规则用 Herdr 新 Codex tab 独立验收。76 项检查及 44 次命令调用通过，fixture 逐字节一致，真实会话正文和 61 次工具调用与旧导出一致；特殊字符、缺失回复、控制字符、入口一致性、stdout/文件输出与拒绝覆盖均通过。产物在 `tmp/2026-10-10-transcript-reader-plain/`，包括 `verify_plain.py`、`report.json`、导出及日志。未启动 GUI、浏览器或模拟器。

## 遗留问题

无本次新增问题。`make parity` 20/21 通过，唯一失败仍是已登记的空 session 排序差异；日志在 `tmp/2026-10-10-transcript-reader-plain/parity.log`。

## 相关文档

- [TXT 导出格式](../notes/2026-10-10-transcript-reader-xml.md) — 更新当前纯文本契约、历史格式说明和验证结果。
- [标签分行](2026-10-10-transcript-reader-lines.md) — 延续结构标签独占一行的排版。
