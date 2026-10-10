---
created: 2026-10-10
tags:
  - cli
  - transcript
  - xml
---

# Transcript Reader：XML 标签组织的 TXT

原 Markdown 导出的 `## User` / `## Assistant` 会与对话正文的 Markdown 标题混淆。现在统一输出 UTF-8 TXT，以 XML 结构区分角色和工具摘要；不提供旧 Markdown 模式。压缩规则不变。

## 格式契约

```xml
<transcript session_id="example">

<user><![CDATA[请修复这个问题]]></user>

<tool_calls>
  <tool name="Bash" count="1" />
  <tool name="Read" count="3" />
</tool_calls>
<assistant><![CDATA[## 修改结果

已完成。]]></assistant>

</transcript>
```

- 外层 `transcript` 带文件 stem 作为 `session_id`。
- `user` 与 `assistant` 按对话顺序排列。正文使用 CDATA，Markdown、HTML、XML 示例都不会变成外层结构。
- `tool_calls` 紧邻其所属的 `assistant` 之前，包含按名称排序的 `tool` 元素，属性为 `name` 与 `count`；没有调用时省略整个摘要。
- 未记录最终回复时，使用 `<assistant status="missing">` 和简短文字标记。
- 正文的 `]]>` 拆分为相邻 CDATA 段，XML 解码后恢复原文。属性中的特殊字符与换行进行实体编码。XML 1.0 不允许的控制字符用 U+FFFD 替换，保证结构可解析。
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

先更新格式行为测试，确认旧 API 不能满足测试，再实现；新增标题/XML/CDATA 冲突和非法控制字符用例。core 全部 79 个测试通过（导出相关 8 个）。实际导出验收由 Herdr 独立 Codex tab 执行，产物归档在 `tmp/2026-10-10-transcript-reader-xml/`。

独立验收通过：fixture、真实 transcript、特殊字符和非法控制字符样本均通过 Python 标准库 XML 解析；正文中的标题、结束标签、重复 CDATA 结束符解码后完整还原，属性值正确还原，stdout 与文件输出逐字节一致。真实导出包含 2 条用户输入、2 条回复、61 次工具调用，详见目录内 `report.json` 与 `real-session.txt`。未启动 GUI、浏览器或模拟器。

再次运行 `make parity` 为 20/21 通过，唯一差异仍是已登记的空 session 并列排序问题，与初版一致。

初版实现、压缩语义与历史验证见 [Transcript Reader 初版](2026-10-10-transcript-reader.md)。
