# Next up

> 接下来要做的事，按时间排。做完一项就删掉，结果记到条目指定的 kb 文档，这里不留记录。

## 用户有空时（M4b 验收里只能人工做的三项）

步骤、要选的 session 和预期结果都在 `kb/sessions/2026-10-27-asm-m4b-accept.md`「请用户验证」B、C、D。

- **resume 命令端到端**（验收标准 2）：从右键菜单或 `⇧⌘C` 复制一个已结束 session 的 resume 命令，粘到终端执行，应进入这个对话；看完 `/exit`。
  → 结果记到 `kb/sessions/2026-10-27-asm-m4b-accept.md` 的验收表第 2 条
- **界面导出与 CLI 一致**（验收标准 7）：界面导出 `067ab941`，和 `tmp/2026-10-27-m4b-actions/accept/export/reader-067ab941.txt` 做 `diff`，应无输出。保存对话框可能弹 iCloud Drive 的授权，见同一节 A。
  → 同上，验收表第 7 条
- **标题行按钮的悬停提示**（验收标准 1）：6 个按钮的提示文字，置灰时提示 "The transcript is gone"。
  → 同上，验收表第 1 条
