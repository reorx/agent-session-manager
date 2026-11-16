---
created: 2026-11-16
tags:
  - cli
  - skill
  - asm
  - transcript
  - agent-session-manager
---

# asm skill P1-5：transcript 按范围读

## 协调者摘要（asmcli-p1-transcript）

- 结果：`asm transcript` / `transcript-reader` 加上 `--outline`、`--turns`、`--grep`（`--regex`、`--context`）、`--thinking`、`--tools`。core 新增 `TranscriptOptions`、`TurnRange`、`read_transcript_with`（`transcript.rs`）和实现模块 `transcript/reading.rs`；不带选项时走原 `read_transcript`，输出逐字不变。回合号只用 `RecordFilter` + `opens_turn`；`--grep` 用 `Query`，tool-input 用 `tool_input_text`，tool-output 用 `tool_result_text`（`regex.rs` 抽出 `tool_input_map` 两处共用）。R1-3（开窗后还藏着命中）和 P1-4 #4（`--grep=-rf foo`）都写进了 `cli.md` 并有测试。
- 提交：见本提交。
- 证据：`cargo test -p agent-session-core` 312 过 0 败 4 忽略（P1-4 时 300；`tests/transcript_reader.rs` 新增 12 个，原有 9 个未改且通过）；`cargo test -p agent-session-manager` 151 过 0 败 2 忽略（同改动前）。新测试先于实现、编译失败（`tests-before-impl.txt`）；6 个变异（显示时按 RecordFilter 丢记录、`<user>` 用清理后的文本、去掉上下文回补、窗口改 200、去掉藏起命中的计数、thinking 总参与判定）都被抓到（`mutations.txt`）。
- 真实数据（新索引冷建 21.7 s，`real/`）：4 个会话（两个含中断标记 + compact 摘要，两个含本地命令）`--outline` 行数 = `session-old --conversation` 的 `turns=` = 索引 prompt 行数（17、10、38、30，`turns-compare.txt`）。9 组 `asm search`（index 与 scan、各类别、`--regex`）的 54 个带回合号片段，`--turns N --grep` 和只用 `--grep` 都读得到片段原文，其中 9 个在长工具输出 2,000 字符之后（如 `dd007b94` 第 1 回合，20,036 字符里第 11,709 处，`verify-hits.txt`、`deep-dd007b94-turn1*.txt`）。16 MB 会话各模式约 0.2 s（`timing-006aca10.txt`）。
- parity（决定 10）：两次 14/21、17/21，差异全在比对期间写入的会话（下文「parity」）；只有 `list-code-30d` 两次都失败，同 P1-4 #6。
- 自己的决定：按范围读时每个回合显示 `asm search` 搜的全部 item、用它搜的文本（不按 RecordFilter 丢记录，`<user>` 是原文带包装标签；`--outline` 用清理后的首行）；藏起的命中写在最后一个空隙行 `…[cut N chars, M more hits]`；放不下的窗口整个不开、之后的命中都算藏起；`--grep=-rf foo` 报错（exit 2），其余 TERM 另写一个 `--grep`；`--regex` / `--context` 没有 `--grep` 时 exit 2；`--context` 可以从范围外的命中回合伸进范围；每条 assistant 记录一个 `<agent>`；理由见「决定」。
- 未验证：GUI 详情面板本身（只比了它的读取函数 `read_conversation`，经 `session-old --conversation`）；单个回合工具输出极大时的内存（回合结束前全文留在内存）。
- 交给协调者：5 条（文末）。scratch：`tmp/2026-11-14-asm-skill-cli/asmcli-p1-transcript/`。

## 实现

core：

- `transcript.rs`：`TranscriptOptions { outline, turns, grep: Option<Query>, context, thinking, tools }`、`is_full()`；`TurnRange { first, last: Option<u32> }`（`N`、`N-M`、`N-`，`parse` 给 clap 当 value parser，错误即 exit 2）；`read_transcript_with` 在没有阅读选项时直接调 `read_transcript`。计划里写的 `turns: Option<(u32, u32)>` 表达不了 `N-`，改成了 `TurnRange`。
- `transcript/reading.rs`（`transcript.rs` 的子模块，可直接用父模块私有的 `Record`、`RecordFilter`、`sanitize_text`、`xml_attribute`）：逐行读（和完整导出同一个循环，未写完的末行照样读，坏行计数），每条记录先 `filter.keep`（拿走 uuid，回合号与索引一致），user 记录有自己的输入时 `kept && opens_turn` 开新回合，否则是 `<other>`；tool_result 按 `tool_use_id` 填进本回合对应调用，找不到就单独成一个 `<tool>`（名字从全文件的调用表找，找不到为 `?`）。assistant 记录依次出 thinking（非空块用 `\n` 拼）、reply（text 块用 `\n` 拼）、每个 tool_use；`final_reply` 按完整导出的规则（有工具调用或 aborted 清掉，纯文字记录置上），回合结束时 agent 说过话但没有最终回复就补 `<agent status="missing">`。
- 回合结束时决定去留：有命中（或没有 `--grep`）就先把待定的前 N 个回合输出，再输出本回合，并记下后面还要带 N 个；没有命中时，若还在「后 N 个」里就输出，否则渲染后放进长度为 N 的待定队列（范围外的放 `None` 占位）。这样不用把整份会话留在内存里。`matched` 是最后输出的回合数。
- 截断 `cut`：按字符计；没有命中取前 2,000；有命中先取前 500，再按命中顺序加 `[h-300, h+700)`，与上一个窗口相接就合并，累计超过 4,000 时这个命中和之后的都不开窗、计入最后一个空隙行。字节与字符偏移各用一次线性扫描互换。

CLI：`support/transcript.rs::ReaderArgs` 加 7 个选项（`--grep` 是 `num_args = 1..` 的 `Vec`，`--regex` / `--context` `requires = "grep"`，`--context` 用 `Option<u32>` 避免默认值触发 `requires`）；TERM 错误返回新的 `UsageError`，`asm` 和 `transcript-reader` 的 `main` 都映射为 exit 2；`--regex` 编不过的 TERM 和 `search` 一样在 stderr 提示。

## 决定

1. **按范围读 = `asm search` 的 item**：任务要求「`asm search` 报出的命中，`transcript --grep` 一定能读到」。索引和扫描都不按 RecordFilter 丢记录（同 uuid 的后一条 user 记录、主记录里的 sidechain 记录是 `other` 行；assistant 记录全部入库），所以这里也全显示。代价：这类记录出现时，回合的工具次数会比完整导出多。在 3,340 份真实记录里统计，有文字的此类记录为 0（`dup_census.py`）；fixture 里有（`turn_rules.jsonl`、`conversation.jsonl`、`reader.jsonl`），`every_hit_the_search_reports_is_shown_in_its_turn` 覆盖。`cli.md` 写明了。
2. **`<user>` 用原文**：索引存的是 prompt 原文，`clean_prompt` 会去掉 `<command-message>`、`<system-reminder>` 等包装。真实数据 8,106 条 user 文字记录里 671 条带包装、去掉的部分中位数 54 字符、最长 295（`clean_census.py`），噪声小；而用清理后的文本，命中落在包装里时 `--grep` 会保留一个看不到命中的回合。大纲仍用清理后的首行（`/herdr go` 比标签可读）。
3. **藏起的命中**（R1-3）：写在最后一个空隙行 `…[cut 13288 chars, 3 more hits]`。放不下的窗口不截短，因为之后的命中一定都在它后面，统一计数更好理解；规范让 agent 换更窄的 TERM 去读。
4. **`--grep=-rf foo`**：clap 把 `=` 后的值当作这一次的全部值，`foo` 成为多余的位置参数，报 `unexpected argument 'foo'`（exit 2）。规范改为「附着的 TERM 独占一个 `--grep`，其余另写 `--grep foo --grep=-rf`」，两种顺序都有测试。
5. **范围与上下文**：「`--turns` 和 `--grep` 同时满足」理解为：在范围内，且自己有命中或在某个命中回合的 `--context` 之内（那个命中回合可以在范围外）。`matched` 按规范是保留下来的回合数（含上下文回合）。
6. 其他：每条 assistant 记录一个 `<agent>`（与 item 粒度一致）；agent 从没说话的回合（本地命令）`</user>` 后什么都没有；`ts` 未知时省略属性、大纲写 `?`；大纲编号固定右对齐 4 列（总回合数要读完才知道）。

## 规范改动（`skills/asm/references/cli.md`）

- TERMs：`--grep=-rf` 独占一个 `--grep`，`--grep=-rf foo` 是错误。
- `asm transcript` 表：`--grep` 可重复、`--regex` / `--context` 需要 `--grep`（否则 exit 2）。
- 范围与上下文的组合、`--turns` 越界的报错原文、非法范围 exit 2、正则回退提示。
- 阅读选项一节：显示的是 `asm search` 的 item；`<user>` 是原文、图片占位；`<agent>` 每条记录一个、`missing` 何时出现；结果不在同一回合时单独成 `<tool>`；同 uuid / sidechain 记录照样显示；截断规则补上与前 500 合并、藏起命中的计数、按字符计、`<user>` / `<agent>` / `<thinking>` 不截。
- 大纲：首行格式（`, 2 matched`）、时间来源、编号 4 列、首行取清理后的第一个非空行。

## 真实数据

- 索引：`real/index.sqlite`（新路径冷建，3,350 份文件 21.7 s）。
- 回合数：`turns-compare.txt`，另存了各会话的大纲 `outline-*.txt` 和 `9550114a` 全部回合的阅读模式输出（19 个 `<other>`：15 个任务通知、3 个 compact 摘要等）。
- 命中可读：`verify_hits.py`（跳过 15 分钟内有写入的会话，免得和正在写的文件比；这次跳过 44 个，多 agent 同时在讨论这些词）。9 组查询：默认类别的中文多说法（index）、`--in all` 的 `database is locked`、`--in tool-output` 的 `healthz` / `Traceback` / `panicked at`、`--in thinking`、`--in other`、`--in tool-input --tool Bash`、`--regex --in all 'error\[E[0-9]+\]'`。54 个片段全部 ok。
- 深处命中样例：`asm transcript dd007b94 --turns 1 --tools --grep "database is locked"` 的那段输出是 `…[cut 11179 chars]` + 窗口 + `…[cut 7906 chars]`；不带 `--grep` 时这段输出里没有这句。
- 耗时（单次、含进程启动，16 MB 的 `006aca10`）：完整导出 135 ms、`--outline` 249、`--tools --thinking` 221（输出 576 KB）、`--tools --grep error` 219（600 KB）、`--outline --tools --grep error` 204。

## parity

`cargo build --release … && scripts/compare-with-python.sh <asm> tmp/2026-11-14-asm-skill-cli/asmcli-p1-transcript/parity{,2}`，两次之间没有并行别的调用；索引副本已删，diff 留着。

- 第一次 14/21：`list-code-30d`（4 个会话 09:33–09:35 的大小 / 更新时间）、`search-grafana`（两边 1,633 行内容相同，`6d6f3f3f` 在比对中 09:21 → 09:35 被排到最前）、`search-cjk`（09:37 新写入的一条命中、多 1 个会话）、`search-traceback`（09:31 一个会话的命中位置）、`search-roles`（09:37 更新导致的顺序）、`search-literal`（扫描数多 1）、`search-regex`（`eebbb767` 09:40 → 09:41）。
- 第二次 17/21：`list-code-30d`（`d8c78f7d`、`eebbb767` 09:42–09:43 的大小）、`search-grafana`（09:43 新写入的一条回复）、`search-bash`（扫描数多 1）、`search-roles`（同时间条目的顺序）。
- 除 `list-code-30d` 外每组至少一次通过。本任务对旧路径只动了 `iter_texts` 里抽出的 `tool_input_map`（行为不变，旧测试和 parity 覆盖）。

## 踩过的坑

- zsh 不按空白拆分未加引号的变量：`for a in "--tools --thinking"; do asm transcript X $a` 会把整串当成一个参数（exit 2）。要写 `${=a}`。
- `asm transcript … | head -1` 会在 stderr 打 `Broken pipe (os error 32)` 并 exit 1（既有行为，见交给协调者 1）。

## 交给协调者

1. **管道被提前关闭时报错**：所有命令写 stdout 遇 EPIPE 都打印 `Broken pipe (os error 32)` 并 exit 1（`asm transcript … | head`、`asm search … | head`）。agent 很常这样用，会误以为命令失败。建议在 `main` 里把 BrokenPipe 当成功退出。范围外，没改。
2. **按范围读与 app 的差别**（决定 1）：同 uuid 的重复记录、主记录里的 sidechain 记录，搜索算、app 和完整导出不显示、按范围读显示。若希望三者一致，应改的是索引 / 扫描（P1-2 / P1-3 的口径），不是这里。真实数据目前没有这类记录。
3. **给 P1-6（skill）**：推荐流程 `asm search` → `asm transcript ID --outline` → `asm transcript ID --turns N --tools --grep <同样的 TERM>`；大会话不要不带 `--turns` / `--grep` 就用 `--tools`（16 MB 的会话输出 576 KB）；以 `-` 开头的 TERM 用 `--grep=-rf` 且独占一个 `--grep`；看到 `M more hits` 就换更窄的 TERM；`<user>` 里的 `<command-name>`、`<system-reminder>` 是 Claude Code 加的。
4. 计划 `session-search.md` Tech Design「core 其他」第 1 条可同步：`turns` 是 `TurnRange`（支持 `N-`），截断规则以 `cli.md` 为准（前 500 + 窗口 + 4,000 上限 + 藏起命中计数）。
5. `real/index.sqlite`（189 MB）审阅或验收后可删。
