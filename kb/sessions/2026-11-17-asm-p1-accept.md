---
created: 2026-11-17
tags:
  - cli
  - skill
  - asm
  - acceptance
  - agent-session-manager
---

# asm P1-7：真实数据验收与 skill 端到端

## 协调者摘要（asmcli-p1-accept）

被测：f857f19 导出到 `src/`，用自己的 `CARGO_TARGET_DIR` 构建；`~/.local/bin/asm` 与该构建逐字节相同（`cmp`）。`ASM_DB` 全程在自己子目录。证据：`tmp/2026-11-14-asm-skill-cli/asmcli-p1-accept/`（`ev/`、`parity1/`、`parity2/`、`ev/8-*`）。未改任何已跟踪文件，未提交。

1. parity **通过**：两次 18/21、17/21，失败组（list-code-30d、search-cjk/roles/case/limit）的 diff 全是比对期间正在写入的会话（updated 时间、「N sessions scanned」+1、同一时间戳的会话顺序对调；排序后内容一致）。
2. 搜索 **通过**：160 个会话，每个有标题 / 项目 / 时间 / 各类别命中数 / 带回合号的片段 / resume 命令；当前会话默认不在，`--include-self` 才出现（用 `asmcli-p1-accept` 验：0 → 出现；这条查询本身在我的会话里没有文字命中，所以 160 = 160）。抽查前 20 个里的 3 个：b123082a（VocalFlow 连不上节点，相关）、e15bd5d3（IP 检测测试误报「连不上」，部分相关）、1d5676f0（只是 `offline` HTML 模板，不相关）；前 20 个里约一半相关（见 F5）。
3. 耗时 **基本通过**：30 天、`--in tool`，机器负载 3.5–6.5（别的 agent 在构建）。热身一次后三次取中位数：`connection refused` 2.49 s（1.70–2.54）、`ECONNRESET` 1.92 s（1.59–3.08）、`offline` 1.75 s（1.35–2.15）。前两组一次完整执行才是热身，文件缓存已热；最慢的那组超 2 s，看波动与负载有关，没做空载复测（F4）。
4. 回合号 **通过**：5 个会话 `transcript --outline` 回合数与 `session-old --conversation` 的 `turns=` 全部相等（13a97fe7 计划交接 =1、875ea045 =1、759a2bcb 有 `/model` 本地命令 =3、5caa2356 含 /compact 与粘贴 =9、85e5a2f3 `<bash-input>` 开头 =2）；`asm search` 的 11 个片段（会话, 回合）用 `--turns N --grep` 全部读得到命中词。与 app 详情的 Turn 编号未比（要 GUI）。
5. 管道无 `\x1b` **通过**；`--json` 的 search / session / projects 都能被 jq 解析，字段与 `cli.md` 的 JSON 示例逐键比对一致（session.tools 是动态键）。`transcript` 按规范没有 `--json`（cli.md 写明），所以没有第四条。
6. 前缀 **通过**：不唯一时 stderr 列 20 个候选 + `… 180 more`、exit 1，`--json` 给 `error.code=ambiguous` + 全部候选、exit 1；无匹配 exit 1；`projects -d 30`：155 个项目（显示 50），按最近活动排序，合理。
7. 待核实项：**⏎**：原文确有字面 ⏎（过去 30 天 77 个记录文件含 U+23CE，例：vocalflow 的「历史浮窗 «⏎» 的保留模式」），所以片段里的 ⏎ 不一定是换行（F2）。**`--until`**：按 `cli.md` 是区间相交，所以起始早于该日、今天仍在写的会话在任何 `--until D`（D ≥ 起始日）里都会出现，选不掉；只能用 `--sort hits` 或 `-p` 避开（F3）。**未知记录类型**：`api-request-blob`（含整份 system-reminder，含 CLAUDE.md）、`api-request`、`api-request-shape`、`session-owner`、`tools-used`：在 147 / 123 / 118 个近 3 天的文件里；asm 的 search（全类别、regex）、transcript、session 都没读到其中的文字，也没有 warning，费用、回合数正常，没有造成问题；但这些记录会把闲置会话的 mtime 推新（F1）。**延迟写入**：不是分钟级。我写出独特标记后 ≤3 s 原始文件就有（tool_use 在调用开始 3 s 内落盘），`asm search --include-self` 下一次调用就搜得到；中间那次 20 s 后仍为 0 是因为当时消息尚未落盘（落盘在该消息结束时，≤20 s）。未复现「几分钟」。
8. skill 端到端 **通过**：被测会话（无模型参数、新索引路径，已确认进程环境有 `ASM_DB`）自己加载了 asm skill、读 finding-sessions.md、用 `asm projects --help` 做前提检查，约 3 分钟内跑了 10 条命令（search → outline → `--turns --grep` → `--tools`），冷建索引没有特殊处理也没出错（第一条 search 里完成）。最终给出 4 个会话，各有会话 ID / 项目 / 时间 / 回合位置 / 一句话结论 / resume 命令，明确说明哪个没找到处理记录、另 28 个只是顺带提到、榆林中控是设备不是 app；核实过（读了 `--turns` 的原文）。相关性合理（用户问题本身没指定 app）。辅助 tab 已关，无遗留进程。

发现：F1 medium 闲置会话 `updated` 被无时间戳的记录（permission-mode / session-owner / atis-latch）顶到 mtime：36050a78 的 last turn 是 11-15 11:52，`asm session` 的 updated 是 11-16 09:54（已知，status 待转交）。F2 low ⏎ 有歧义（skill 说「⏎ 是换行」）。F3 low `--until` 选不掉在写的长会话，skill 未提。F4 low 30 天工具搜索在负载下中位数 1.7–2.5 s，达标边缘。F5 low `连不上|…|offline` 的排前面的结果里 `offline` 带来噪声（HTML 模板、「离线」功能）。F6 low skill 的 `--no-scan` 规则没说 `transcript` 不支持，被测 agent 用错一次（exit 2，立刻改正）。

未验证：与 app 详情 Turn 编号一致（要 GUI）；空载下的耗时；`.claude/skills` 符号链接在别的机器上；agent 的结论正确性只从原文片段抽查，没全程复核。

交给协调者：F1 记 known-issues（已在待转交）；F2 / F3 / F6 可在 SKILL.md 各加一句；耗时记录写进 session-search.md 验收；scratch 里 `idx/`（~230 MB）、`e2e-idx/`、`parity*/index.sqlite`、`target/`（构建产物，大）、`src/` 可删。

## 证据索引（`tmp/2026-11-14-asm-skill-cli/asmcli-p1-accept/`）

- `parity1.log`、`parity2.log`、`parity{1,2}/*.diff`
- `ev/2-search.txt`、`ev/2-search-self.txt`、`ev/2-self-check.txt`
- `ev/3-timing.txt`
- `ev/4-outline-*.txt`、`ev/4-old-*.txt`、`ev/4-turn-check.txt`（11 个片段，0 BAD）
- `ev/5-*.json`、`ev/5-esc.txt`
- `ev/6-*`、`ev/7-until.txt`、`ev/7-unknown-types.txt`、`ev/7-latency*.txt`
- `ev/8-commands.txt`、`ev/8-answer.md`、`ev/8-e2e-transcript.jsonl`

## 做法与踩过的坑

- zsh 不拆分 `$r` 里的多个参数，循环里测 `--until` 第一次得到「unexpected argument」，改用 `eval`。
- `ls` 在本机是 eza 别名，`grep` 是 ugrep；对原始记录找字面 ⏎ 要用 `LC_ALL=C grep $'\xe2\x8f\x8e'`。
- parity 要在自己的会话不并行调用时跑；这里两次都有别的 agent 在写入，按 brief 视为通过。
