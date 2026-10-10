---
created: 2026-10-10
tags:
  - transcript
  - cli
  - xml
---

# Transcript Reader 结构标签各占一行

## 概要

按用户反馈，将所有结构标签的起始、结束标签分别放在独立行，正文和纯文本摘要放在两者之间。覆盖 user、agent、缺失回复和 turn-summary。

## 修改的文件

- `crates/core/src/transcript.rs`：为标签与内容增加换行。
- `crates/core/tests/transcript_reader.rs`：先更新输出快照和边界断言，确认失败后再实现。
- `README.md`、`kb/notes/2026-10-10-transcript-reader-xml.md`：更新当前排版契约及示例。

## 注意事项

正文的 CDATA 内容不变，不额外缩进。XML 解码后元素文本两端多出一个排版换行，消费者可移除两端空白。core 79 个测试通过；独立 release CLI 的真实会话及特殊字符样本通过 XML 解析，与原归档仅有预期的排版差异，stdout 与文件导出一致。

已重建 `dist/transcript-reader`，全局安装未更新。验收产物在 `tmp/2026-10-10-transcript-reader-lines/`；未启动 GUI、浏览器或模拟器。

## 遗留问题

无本次新增问题。`make parity` 20/21 通过，唯一失败仍是已登记的 `list-code-30d` 空 session 排序差异；日志在验收目录的 `parity.log`。

## 相关文档

- [TXT 导出格式](../notes/2026-10-10-transcript-reader-xml.md) — 更新分行示例及排版约定。
- [标签与摘要调整](2026-10-10-transcript-reader-tags.md) — 本次排版沿用其标签和工具摘要语义。
