---
created: 2026-11-15
tags:
  - cli
  - skill
  - asm
  - search
  - agent-session-manager
---

# asm skill P1-3：会话级搜索（core）

## 协调者摘要（asmcli-p1-search-r1）

- 结果：F1–F9 都改了。F1：搜 tool-input 时说法在每个 `:`、`,`、`$`、`#` 处切开并去掉段首空格，`file_path:`、正则 `file_path:.*config`、单独的 `$` / `#` 不再被预过滤漏掉。F5：选行判断抽成 `parse.rs::may_be_user_row`，解析器和扫描共用。F6：TERM 编译失败（含合起来超出大小限制）返回 `QueryError::TooLarge`（exit 2），预过滤的字节正则编不过就放行。F8：`SessionHits.turns`，两条路径都从给片段编号的同一处数出；顺带 `SearchOptions` 加 `sort` / `limit`、`SearchOutcome.total`，index 路径只给显示的 session 取片段和回合。F9：符号链接按「最长的存在的祖先」解析。F7 及 F6、F9、`turns`、tool-input 文本格式写进 `cli.md`。
- 提交：见本提交。
- 证据：`cargo test -p agent-session-core` 263 过 0 败 4 忽略（R0 时 257 / 3；本次新增 1 个集成测试和 1 个忽略的真实数据测试，其余 5 个来自 4ac3aae）；`cargo test -p agent-session-manager` 151 过 0 败 2 忽略。F1 复现测试修前失败（`r1/f1-before.txt`，摘录见「返工 R1」），F9 新用例修前失败，修后都过。parity 16/21，5 组差异全在比对期间正在写入的会话上（`parity/`）。
- F2：最近 30 天 2,016 个会话（排除 1 分钟内写过的），12 组工具类查询（`file_path:`、`$ git`、`#`、`error`、中文、两个正则、两个 `--tool`、两概念等）预过滤开 / 关结果 0 处不同（`r1/f2-prefilter-tools.txt`）。index 与 scan 在 3,269 个未变会话上 0 处不同，现在连 `turns` 一起比（`r1/agree-r1.txt`）。
- F3（每组一个新进程，每次新连接，`-n 20`，负载 14–19）：热缓存中位数 28–55 ms，`的` 50 ms（1,813 个会话；`-n 0` 时 78 ms），`Ωx` 55 ms；首次运行最高 164 ms（基线那轮 241 ms），多出的是系统页缓存未命中，换新连接不影响。固定成本是 `select_sessions` 约 25 ms（`r1/f3-index*.txt`）。
- F4：30 天 `--in all` 单次扫描峰值 RSS 346–482 MB、footprint 最高 493 MB（5 次，`r1/f4-memory-before.txt`），未超 500 MB，按任务只记数字，没有加限制。
- 自己的决定：F1 在分隔符处一律切开并去段首空格（正则 TERM 不去空白，前导空格也会落在 needle 里）；F6 选报错而不是退回逐个高亮；F8 把排序和条数收进 `search`，index 路径的回合数改成每个显示的会话查一次 prompt 偏移表；F9 选做实现而不是改规范。
- 未验证：清空页缓存后的真冷读；F4 在 agent 并行调用时的叠加。
- 交给协调者：4 条（「返工 R1」末尾）。

## 协调者摘要（asmcli-p1-search）

- 结果：core 新增 `search/query.rs`（TERM 解析、空匹配判定）、`search/sessions.rs`（种类、session 过滤、`search` 入口、计数与选片段、排序、`turn_count`、`tool_input_text`）、`search/scan.rs`（扫描）、`search/prefilter.rs`（字节预过滤）；`regex.rs::iter_texts` 改为 `pub(crate)` 并带块类别。`cli.md` 按复核 R1-2、R1-6、R1-7 和 prompt 定义（原 :111，现 :115）改了，「How it searches」的耗时说明按实测改了。
- 提交：见本提交。
- 证据：`cargo test -p agent-session-core` 257 过 0 败 3 忽略（P1-1 R2 时 226 过 1 忽略；新增 31 = 集成 27 + prefilter 单测 4，新增 2 个忽略的真实数据测量）；`cargo test -p agent-session-manager` 151 过 0 败 2 忽略；7 个变异都被测试抓到（见「测试」）。parity 两次各 20/21，两次失败的组不同，都是比对期间其他 agent 在写入（大小 / 更新时间变了、两次运行之间多了 1 个 session），每组至少一次通过；输出在 scratch 的 `parity/`。
- 测量（release，真实索引副本，3,332 个 session，机器负载约 4.5、空闲内存约 70 MB，各 3 次取中位数，方法见「测量」）：index 全时段默认种类 `error` 28.1 ms、`healthz` 24.6、中英两概念 24.6、两字 `网络`（走 LIKE）46.0、`session`+`cache` 26.7（都含 `select_sessions`；目标 100 ms）。scan 最近 30 天 `--in all`，2,019 个文件 5.68 GB：`database is locked` 2034 ms（跳过 99% 文件 / 96% 字节）、`connection refused` 1569（74% / 56%）、`error` 1551（1% / 0%）、`healthz`+`连不上|无法连接|offline` 856（84% / 71%）；目标 2 s，第一组持平，首次冷读最高 3.9 s。index 与 scan 在 3,294 个未变动的 session × 9 组查询（`--same-message` 开和关）上 0 处不一致。
- `\u` 普查：3,319 个主 transcript、7.2 GB，115,187 个 `\u` 转义全是 U+0000–U+001F，预过滤的前提成立，不降级。
- 自己的决定：scan 的回合号照解析器的行选择 + `RecordFilter` 逐行重算，和索引的 `kind='prompt'` 口径一致；R1-7 选在 Rust 里补过滤（含非 ASCII 大小写字母的短词不走 LIKE），规范不写例外；扫描用 `read()` 不用 mmap，预过滤用 `regex::bytes` 不用 aho-corasick（都有实测）；`SessionFilter` / `ProjectMatcher` / `KindSet::parse` 放在 core 给 P1-4 用；`--same-message` 下 hits 仍数含至少一个 TERM 的 item（规范如此）。理由见「决定」。
- 未验证：清空系统页缓存后的真冷读（要 sudo）；FTS5 trigram 和 Rust 的 Unicode 大小写折叠在少见字符上的差别；预过滤的两个理论缺口（「决定」第 6 条）没有在真实数据上遇到。
- 交给协调者：5 条（文末）。
- scratch：`tmp/2026-11-14-asm-skill-cli/asmcli-p1-search/`（`timings-final.txt`、`agree-final.txt`、`u-escapes.md`、`floor/`、`parity/`）。

## 实现

入口在 `search/sessions.rs`：

- `select_sessions(db, &SessionFilter)`：按 `cli.md` 的时间范围（`updated_at >= since` 且 `started_at`（无则 `updated_at`）`< until`）、`--project`（`ProjectMatcher`：目录形式按字面归一、canonicalize 两边都比，所以符号链接两头都认；其他按文本匹配编码后的项目键）、`--model`（主模型，补 `claude-` 前缀）、收藏、排除当前 session 选出要搜的 session，更新时间降序。
- `search(db, sessions, &Query, &KindSet, &SearchOptions)`：`Engine::for_query` 决定走哪条路——种类都在索引里（`prompt` / `reply` / `thinking` / `other`）且不是 `--regex` 时走 index，否则 scan；强制 index 但查询需要 scan 时报错。返回 `SearchOutcome { engine, sessions: Vec<SessionHits>, scan: Option<ScanStats> }`。
- `SessionHits`：`counts` 列出所有被搜种类（包括 0），`term_hits` 每个 TERM 的 item 数，`snippets` 是 `Hit { kind, tool, turn, ts, offset, terms, text: Highlighted }`。片段先保证每个 TERM 至少出现一次，再按排名（含 TERM 多的在前，然后种类、偏移）补满，输出按排名排序。
- `sort_results(&mut [SessionHits], SortBy::{Recent, Hits})`、`turn_count(db, id)`（数 `kind='prompt'` 的行，不用 `user_msg_count`）。（R1 起 `sort_results` 去掉，排序和条数由 `SearchOptions.sort` / `limit` 在 `search` 里做；结果的回合数用 `SessionHits.turns`。）
- `tool_input_text(&Map)`：tool-input 的搜索文本（`# description`、`$ command`、其余 `key: value`，嵌套值按 Python `json.dumps` 的格式），P1-5 的 `transcript --grep` 应该复用它。

index 路径：每个 TERM 的每种说法，≥ 3 个字符用 FTS5 短语 MATCH；更短的，没有非 ASCII 大小写字母时用 `LIKE`（`fts::escape_like` 改为 `pub(crate)`），有的话读出所搜类别的**全部**行（不是候选行，R1 更正）在 Rust 里用 TERM 的正则判定。命中行的回合号是「偏移不超过它的 `kind='prompt'` 行数」。

scan 路径（`search/scan.rs`）：rayon 并行，文件按大小降序派发；整份读入后先看每个 TERM 是否都在文件里，不在就跳过整份；再逐行：可能含 TERM 的行要解析；有文字或媒体的 user 行（以及字节分不出类别的行）即使没有 TERM 也要解析 JSON，因为回合由它们决定（R1 更正：原来写的「只看字节分类，不解析 JSON」不对）。这些行按解析器的规则（`classify`、`may_be_user_row`、`RecordFilter::for_file` + `keep` + `opens_turn`）决定是否开新回合、是 prompt 还是 other，所以回合号和索引一致。item 来自 `regex.rs::iter_texts`（isMeta 跳过、user 文本按 `\n` 拼、thinking 去空、tool-output 每个 `tool_result` 一项）。tool-output 的工具名用 `"id":"<tool_use_id>"` 在文件里找 tool_use 行解析得到；有 `--tool` 时所有输出都要解出工具名，解不出的不算。

`search/query.rs`：字面 TERM 按 `|` 切、去两边空白、丢空说法，编译成转义后的交替（长的在前），大小写不敏感（Unicode）；`--regex` 时整条编译，编不过按字面整条匹配（`regex_fallback()`），`regex-syntax` 的 `minimum_len() == Some(0)` 判空匹配（拦 `\b`、`^`、`foo|\b`、`\B`、`(?:)`）。最多 64 个 TERM（`TermMask = u64`）。错误信息的 TERM 编号从 1 开始。

`search/prefilter.rs`：见模块注释和「决定」第 3、6 条。

## 决定

1. **scan 的回合号逐行重算**（不是去索引里按偏移查）：scan 读的是文件当前的内容，索引可能落后（`--no-scan`，或扫描后文件又被写入），按偏移查索引在新写入的部分会数错；照搬解析器的行选择，代价只是对每行做字节分类。`both_ways_number_turns_alike_on_the_turn_rule_fixtures` 用 P1-2 的回合规则 fixture 对两条路径断言回合号相同。
2. **R1-7 在 Rust 里补过滤**：规范写的是 Unicode 大小写折叠，`LIKE` 只折 ASCII；短词含非 ASCII 大小写字母（`ÉÀ`、`Ωx`）很少见，读出全部行再判定的成本可以接受（R1 实测 `Ωx` 约 55 ms，其中 `select_sessions` 约 25 ms），规范不必多一条例外。没有这类字母的短词（含 CJK 两字词）仍走 LIKE。
3. **扫描用 `std::fs::read`，预过滤用 `regex::bytes`**：mmap 版本 30 天约 8.5 s（`floor/prof-1.txt`，页没在缓存里时按页缺页），改 read 后 2.3–3.5 s；aho-corasick 带 ASCII 大小写折叠的重叠匹配占 7,938 ms 线程时间，`(?i-u:…)` 的字节正则 784 ms、memmem 218 ms（`floor/matchers.txt`），换成字节正则后 1.0–1.8 s。现在的瓶颈是读 5.7 GB，20 和 10 个 rayon 线程没有差别。
4. **`--same-message` 下 hits 的口径**：session 要有一个 item 同时含全部 TERM 才算结果，但 `hits` / `term_hits` 仍数含至少一个 TERM 的 item（规范 Counting 一节）。
5. **过滤和参数解析放在 core**：`SessionFilter`、`ProjectMatcher::parse`、`KindSet::parse`（含决定 12：`--tool` 无 `--in` 等于 `--in tool` 且只限该工具；有 `--in` 时只收窄工具类）都有行为测试，P1-4 只做 CLI 参数到这些类型的映射。
6. **预过滤的切分**：Claude Code 只转义 `"`、`\` 和控制字符（普查确认），所以每种说法在这些字符、U+FFFD、非 ASCII 大小写字母处切开，取最长的一段做字节搜索；搜 tool-input 时再在 `: `、`, `、`$ `、`# ` 处切（这些是 `tool_input_text` 加的）。正则 TERM 用 `regex-syntax` 的 Extractor 取前缀或后缀字面量（≤ 64 个），各自同样切分，取最短一段较长的那组。理论缺口：(a) 会折叠成 ASCII 的非 ASCII 字符（开尔文符号 K→k、长 s ſ→s）出现在原文里时，ASCII TERM 在原始字节里找不到；(b) tool-input 里嵌套的浮点数，`json.dumps` 的写法可能和原始 JSON 不同。真实数据上「带预过滤 == 不带预过滤」的比较没有差异。

## 测试

`crates/core/tests/search_sessions.rs` 27 个行为测试，`run()` 每次都断言 index == scan、scan == 关掉预过滤的 scan。覆盖：`|` 切分与空白（中英文）、空 TERM 和空匹配正则报错、64 上限、Unicode 折叠、正则大小写与回退、种类默认值和 `--tool` / `--in` 组合、概念分散在不同 item、计数、片段内容与顺序、片段上限下每个 TERM 仍出现、短词、JSON 转义里的引号 / 反斜杠 / 换行、正则的字面量预过滤、整份跳过、回合号两路一致、首个 prompt 之前无回合、时间范围、项目（文本 / 目录 / 符号链接）、收藏 / 模型 / 排除当前 session、排序、引擎选择。`prefilter.rs` 有 4 个单测。

变异检查（改坏后测试失败，再改回）：扫描忽略 `RecordFilter`、媒体 prompt 不开回合、tool-input 不切分隔符、非 ASCII 短词走 LIKE、`--same-message` 判定去掉、isMeta 不跳过、文件级检查改成「任一 TERM」——7 个都有测试失败。

## 测量

- 索引副本：`ASM_DB=tmp/2026-11-14-asm-skill-cli/asmcli-p1-search/index-1.sqlite asm index` 冷建 18.7 s（3,319 个文件 7.1 GB，库 184.5 MB）。
- 耗时：`ASM_BENCH_DB=<副本> cargo test --release -p agent-session-core --test search_sessions real_search_timings -- --ignored --nocapture`，查询可用 `ASM_BENCH_INDEX` / `ASM_BENCH_SCAN` 换（`;` 分查询、`,` 分 TERM）。index 计时含 `select_sessions` + `search`，默认种类全时段；scan 计时选最近 30 天、`--in all`。每组跑 3 次取中位数，结果在 `timings-final.txt`。scan 三次的波动很大（如 `connection refused` 664–1602 ms），是因为机器空闲内存只有约 70 MB，页缓存被别的进程挤出。
- 一致性：`real_index_and_scan_agree` 对比两路的 session、counts、term_hits，只看测量期间没变的 session，结果在 `agree-final.txt`（另有早一轮的 `agree-1.txt`，3,304 个 session，同样 0 处不一致）。

## 交给协调者

1. `cli.md`「How it searches」原来写「每 GB 不被跳过的数据约 1 秒」，实测时间几乎都花在读文件上（被跳过的文件也要读一遍），改成「每 GB 约半秒，文件不在缓存时更多，给工具搜索加时间范围」。写 skill 的 `finding-sessions.md` 时按这个口径。
2. P1-4（R1 起改为第 R1 节的第 2 条）：用 `select_sessions` → `search` → `sort_results`，`turn_count` 给每个结果的回合数；`QueryError` / `KindError` 的 `Display` 就是 exit 2 的消息；`Query::regex_fallback()` 给 stderr 提示和 `meta.regex_fallback`；`SearchOutcome.scan` 有跳过比例，可放进 `meta`。
3. P1-5：`transcript --grep` 判定 tool-input 时复用 `tool_input_text`，tool-output 复用 `regex.rs::tool_result_text` / `iter_texts`，这样搜得到的片段在 transcript 里也能 grep 到。
4. AGENTS.md「结构」里 core 的 `search/` 可补一句：会话级搜索（`sessions.rs`，index 或 scan）、TERM（`query.rs`）、扫描与字节预过滤（`scan.rs`、`prefilter.rs`）。
5. scratch 的 `index-1.sqlite`（193 MB）复核完可以删。

## 返工 R1

审阅：`tmp/2026-11-14-asm-skill-cli/reviews/asmcli-p1-rv-search.md`；scratch 输出在 `tmp/2026-11-14-asm-skill-cli/asmcli-p1-search/r1/`。

### F1 tool-input 的预过滤漏掉以 `key:` 结尾的 TERM

搜索文本是 `file_path: /x`，原始行是 `"file_path":"/x"`：文本多出来的只有分隔符后的空格（`: `、`, `、`$ `、`# `）、`$` / `#` 本身，以及行之间的 `\n`；原始行多出来的是引号。原来只在「分隔符 + 空格」处切，`file_path:` 这样以分隔符结尾的说法就整段去找，原始行里没有。现在 tool-input 模式下在每个 `:`、`,`、`$`、`#` 处切开，每段再去掉段首空格（文本里多出的空格都紧跟在分隔符后面；正则 TERM 不去空白，所以 ` /x` 这样的前导空格也要去）。

复现（修前，`r1/f1-before.txt`）：

```
test kinds_choose_the_text_searched ... FAILED
assertion `left == right` failed: the prefilter changed the scan's results
  left: []
 right: [SessionHits { … counts: {ToolInput: 1}, … text: "file_path: /x/quagga.conf⏎limit: 20⏎pages: [1, 2]" … }]
test search::prefilter::tests::tool_inputs_cut_at_the_separators_their_text_adds ... FAILED
assertion `left == right` failed: the raw line has `"file_path":`
  left: Some("file_path:")
 right: Some("file_path")
```

新增断言：`kinds_choose_the_text_searched` 加了一个 Read 调用，查 `file_path:`、`limit: 20`、`pages: [1, 2]`、`$`、`#`、正则 `file_path:.*quagga`、` /x/quagga`、`\[1, 2\]`（`run()` 自动比较预过滤开 / 关）；`prefilter.rs` 单测加了 `file_path:`、`$`、`#`、前导空格、`limit:20` 和正则前缀的用例。

### F2 工具类别预过滤开 / 关的真实数据对比

新测试 `real_prefilter_keeps_tool_results`（`#[ignore]`，`ASM_BENCH_DB=<索引副本> cargo test --release -p agent-session-core --test search_sessions real_prefilter -- --ignored --nocapture`，`ASM_BENCH_TOOLS` 可换查询）：最近 30 天、1 分钟内没写过的 2,016 个会话，默认 `--in tool`，逐个比较整个 `SessionHits`（计数、每 TERM 计数、5 个片段的文本 / 回合 / 工具名）。12 组查询全部 0 处不同：

| 查询 | 会话 | 整份跳过 |
|---|---|---|
| `file_path:` | 876 | 32 / 2,016 |
| `limit: 2000` | 0 | 20 |
| `$ git` | 685 | 0 |
| `#` | 985 | 0 |
| `error` | 864 | 22 |
| `connection refused` | 61 | 1,494 |
| `连不上\|无法连接` | 407 | 1,317 |
| 正则 `file_path:.*config` | 48 | 32 |
| 正则 `\$ cargo (test\|build)` | 14 | 1,969 |
| `--tool Bash` `git push` | 100 | 1,861 |
| `--tool Read` `file_path:` | 719 | 32 |
| `old_string` + `new_string` | 606 | 182 |

审阅估计关掉预过滤要 8 秒，实测关掉和开着都是 0.7–2.9 s：时间几乎都花在读文件上，解析不是瓶颈。

### F3 index 路径的首次运行、`的`、`Ωx`

`real_search_timings` 改为每次新开只读连接，分别报首次和中位数，并单独计 `open + select_sessions`；默认按 CLI 的 `-n 20`（`ASM_BENCH_LIMIT` 可改）。每组查询单独起一个进程跑 3 次（`r1/f3-index.txt`；改前的基线在 `r1/f3-index-baseline.txt`）。

| 查询 | 找到 | `-n 20` 首次 / 中位数 | `-n 0` 中位数 |
|---|---|---|---|
| `error` | 394 | 119 / 32 ms | 35 ms |
| `healthz` | 30 | 38 / 28 | 27 |
| 中英两概念 | 24 | 56 / 48 | 28 |
| `网络`（LIKE） | 180 | 164 / 54 | 62 |
| `session` + `cache` | 142 | 42 / 33 | 38 |
| `的`（LIKE） | 1,813 | 50 / 50 | 78 |
| `Ωx`（Rust 全表） | 0 | 55 / 55 | 57 |

- 首次运行慢在系统页缓存：同一个查询第二轮（`-n 0` 那轮）首次只有 40 / 65 ms，每次换新连接也不慢。测时负载 14–19，空闲内存约 190 MB，184 MB 的索引常被挤出缓存。目标 100 ms 在热缓存下都达到；冷缓存下 `error`、`网络` 这类要读大量页的查询会超过。
- 固定成本是 `select_sessions` 约 25 ms（`list_sessions` 读全部 3,332 个会话，`attach_details` 再整表读 `session_tool_counts`、`session_files`、`session_model_usage`），在热缓存的查询里占一半。
- `的`：`LIKE` 扫全表加 1,813 个会话的计数；改成只给显示的会话取片段和回合后从 78 ms 降到 50 ms。
- `Ωx`：读出默认三类的全部行在 Rust 里匹配，扣掉 `select_sessions` 约 30 ms。

### F4 扫描的峰值内存

`/usr/bin/time -l` 跑 release 测试二进制（`ASM_BENCH_INDEX="" ASM_BENCH_SCAN=<查询> ASM_BENCH_RUNS=1`，进程里还有索引连接和会话列表，比真正的 `asm search` 略多），30 天 `--in all`，2,019 个文件 5.68 GB：

| 查询 | 最大 RSS | peak footprint |
|---|---|---|
| `error`（三次） | 442 / 482 / 445 MB | 427 / 470 / 466 MB |
| `#`（两次） | 454 / 387 MB | 447 / 493 MB |
| `database is locked`（整份跳过 99%） | 346 MB | 348 MB |

没超过 500 MB，按任务没有加限制（见「交给协调者」第 1 条）。

### F5–F9

- F5：`parse.rs::may_be_user_row(kind, line)`（user 行可能有文字或媒体，或字节分不出类别），`process_line` 的门控和 `scan.rs::find_items` 的 `turn_line` 都调用它；`user_line_may_have_text` / `media` 改回私有。
- F6：`\w{200}` 和 `x\w{200}` 各自能编译，合成高亮正则时超出 10 MiB 限制，修前 `Query::parse` panic（用临时测试确认过）。现在 `union_matcher` 返回 `Option`，失败报 `QueryError::TooLarge`；字面 TERM 和正则回退成字面时编译失败也报它。预过滤的 `any_of` 编不过就当作该 TERM 没法预过滤（放行）。测试在 `a_term_with_nothing_to_match_is_an_error`。
- F7：`cli.md`「How it searches」改为「整份跳过的文件不解析；其他文件解析可能含 TERM 的行和有文字或媒体的 user 行（它们决定回合）」；本总结「实现」一节同步更正，「决定」第 2 条的「候选行」改为「全部行」。
- F8：`SessionHits.turns`。scan 在读文件时数到文件末尾；index 对每个显示的会话查一次 prompt 偏移表，片段的回合号用二分查同一张表（原来每个片段一条 `count(*)`）。新测试 `the_scan_counts_turns_in_the_transcript_it_read`：索引后再往文件追加两个 prompt，scan 报 3 个回合、片段回合号 1 和 3，index 报 1 个回合。另外 `SearchOptions` 加了 `sort`、`limit`（0 为不限），`SearchOutcome.total` 是限制前的会话数；index 路径先排序截断再取片段，scan 路径片段随读取产生、之后排序截断。`sort_results` 去掉，测试改为两条路径都按 `sort` / `limit` 断言。
- F9：`resolve_symlinks` 对最长的存在的祖先目录 canonicalize 再接上剩下的部分，参数和会话的 `project_path` 两边都这样做。测试加了「目录已删除、经链接询问」和「经链接写入、按真实路径询问」两例（修前失败）。`cli.md` 的 Projects 写明了。

### parity

16/21。5 组差异都在比对期间正在写入的会话上：`list-code-30d` 是 3 个会话的大小 / 更新时间，`search-regex` 是 1 个会话的更新时间，`search-grafana`（两边都是 328 条、2,031 个会话）和 `search-traceback` 是会话按更新时间排序后位置移动，`search-cjk` 是 asm 后跑、多出一个 00:03 新写入的命中。

### 交给协调者（R1）

1. F4 的峰值离 500 MB 只差 2–4%，transcript 会越来越大，agent 也会并行调用。要限制的话，做法是 `scan_file` 读文件前向一个共享的字节预算（例如 256 MB，`Mutex` + `Condvar`，超过预算的单个文件占满预算）申请、读完归还；代价是开头最大的几份文件不能 10 个线程同时读。要不要做请协调者定。
2. P1-4：`search(db, &sessions, &query, &kinds, &SearchOptions { snippets, sort, limit, .. })`，`SearchOutcome.total` 给 `meta.total` 和 `(showing M)`，`sessions.len()` 给 `meta.shown`；结果头的 `turns` 用 `SessionHits.turns`，不要再用 `turn_count`（索引落后时会小于片段的回合号）；`QueryError::TooLarge` 也是 exit 2。
3. `select_sessions` 约 25 ms 是 index 搜索的大头：`attach_details` 给全部会话整表读工具计数、会话文件和模型用量。若要再快，可以先按 SQL 能判断的条件筛会话，`attach_details` 只给剩下的（输出只需要显示的那些，`--model` 才需要全部的模型用量）；这要动 `Db` 的接口，本任务没做。
4. scratch 里 `index-1.sqlite`（193 MB）和 `parity/index.sqlite`（186 MB）复核完可以删。
