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
- `sort_results(&mut [SessionHits], SortBy::{Recent, Hits})`、`turn_count(db, id)`（数 `kind='prompt'` 的行，不用 `user_msg_count`）。
- `tool_input_text(&Map)`：tool-input 的搜索文本（`# description`、`$ command`、其余 `key: value`，嵌套值按 Python `json.dumps` 的格式），P1-5 的 `transcript --grep` 应该复用它。

index 路径：每个 TERM 的每种说法，≥ 3 个字符用 FTS5 短语 MATCH；更短的，没有非 ASCII 大小写字母时用 `LIKE`（`fts::escape_like` 改为 `pub(crate)`），有的话取该 session 的候选行在 Rust 里用 TERM 的正则判定。命中行的回合号是「偏移不超过它的 `kind='prompt'` 行数」。

scan 路径（`search/scan.rs`）：rayon 并行，文件按大小降序派发；整份读入后先看每个 TERM 是否都在文件里，不在就跳过整份；再逐行，行里一个 TERM 都没有就不解析。被解析的行按解析器的规则（`classify`、`user_line_may_have_text` / `media`、`RecordFilter::for_file` + `keep` + `opens_turn`）决定是否开新回合、是 prompt 还是 other，所以回合号和索引一致，未被预过滤放行的行也要参与回合计数（只看字节分类，不解析 JSON）。item 来自 `regex.rs::iter_texts`（isMeta 跳过、user 文本按 `\n` 拼、thinking 去空、tool-output 每个 `tool_result` 一项）。tool-output 的工具名用 `"id":"<tool_use_id>"` 在文件里找 tool_use 行解析得到；有 `--tool` 时所有输出都要解出工具名，解不出的不算。

`search/query.rs`：字面 TERM 按 `|` 切、去两边空白、丢空说法，编译成转义后的交替（长的在前），大小写不敏感（Unicode）；`--regex` 时整条编译，编不过按字面整条匹配（`regex_fallback()`），`regex-syntax` 的 `minimum_len() == Some(0)` 判空匹配（拦 `\b`、`^`、`foo|\b`、`\B`、`(?:)`）。最多 64 个 TERM（`TermMask = u64`）。错误信息的 TERM 编号从 1 开始。

`search/prefilter.rs`：见模块注释和「决定」第 3、6 条。

## 决定

1. **scan 的回合号逐行重算**（不是去索引里按偏移查）：scan 读的是文件当前的内容，索引可能落后（`--no-scan`，或扫描后文件又被写入），按偏移查索引在新写入的部分会数错；照搬解析器的行选择，代价只是对每行做字节分类。`both_ways_number_turns_alike_on_the_turn_rule_fixtures` 用 P1-2 的回合规则 fixture 对两条路径断言回合号相同。
2. **R1-7 在 Rust 里补过滤**：规范写的是 Unicode 大小写折叠，`LIKE` 只折 ASCII；短词含非 ASCII 大小写字母（`ÉÀ`、`Ωx`）很少见，取候选行再判定的成本可以接受，规范不必多一条例外。没有这类字母的短词（含 CJK 两字词）仍走 LIKE。
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
2. P1-4：用 `select_sessions` → `search` → `sort_results`，`turn_count` 给每个结果的回合数；`QueryError` / `KindError` 的 `Display` 就是 exit 2 的消息；`Query::regex_fallback()` 给 stderr 提示和 `meta.regex_fallback`；`SearchOutcome.scan` 有跳过比例，可放进 `meta`。
3. P1-5：`transcript --grep` 判定 tool-input 时复用 `tool_input_text`，tool-output 复用 `regex.rs::tool_result_text` / `iter_texts`，这样搜得到的片段在 transcript 里也能 grep 到。
4. AGENTS.md「结构」里 core 的 `search/` 可补一句：会话级搜索（`sessions.rs`，index 或 scan）、TERM（`query.rs`）、扫描与字节预过滤（`scan.rs`、`prefilter.rs`）。
5. scratch 的 `index-1.sqlite`（193 MB）复核完可以删。
