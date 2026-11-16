# Next up

> 接下来要做的事，按时间排。做完一项就删掉，结果记到条目指定的 kb 文档，这里不留记录。

## 下一次发版时：验证每日自动检查的真实升级

- 来源：2026-10-28 发布 1.0.0（`kb/docs/release.md`「发布后验收」）
- 在一台装着 1.0.0 的机器上（通过 brew 或 dmg 安装，不是测试副本）照常使用，不手动「Check for Updates…」。发出下一版后，等 Sparkle 的每日检查自然触发：切回 app 时出现更新提示（app 在后台时 Sparkle 会等你切回来才弹），确认安装后重启，About 显示新版本。通过后删掉本条，结果记进 `kb/docs/release.md`

## 下次改界面布局时：校对 `screenshot-clicks.sh` 的坐标

- 来源：2026-11-17 usage-p4-drill 发现（`kb/sessions/2026-11-16-usage-p4-drill.md`「交给协调者（r1）」）
- 第 3 期在工具栏加了 `Sessions | Usage` 切换后，脚本里写死的 x 坐标就偏了：现在 Project 在 164–270、Updated 在 381–523、Kind 在 529–614，脚本点的却是 54 / 274 / 383。要真点击才能校对，照 AGENTS.md「验收流程」的点击规则做。校对完删掉本条
