# Finding past sessions

The user describes a session in their own words ("帮我找一下近期关于某个 app 连不上网的问题解决会话",
"where did we fix the flaky ECONNRESET?"). Find it with `asm`, check what it says, and report
it with the command to resume it. It usually takes three to six commands:

1. Split the request into concepts.
2. Write each concept as one TERM with several ways of saying it.
3. Search with the default text kinds, then narrow or widen by what the output says.
4. Pick one to three candidates from the output alone.
5. Verify each in its transcript: outline, then only the turns with the hits.
6. Report: session, time, project, a one-line conclusion, the resume command.

Option details are in [cli.md](cli.md) (`### asm search`, `### asm transcript`).

## 1. Split the request into concepts

A concept is something that has to appear, in some wording, somewhere in the session: a
symptom, an error, a component, a command, a file, a device or service name. A session is a
result only when every concept is in it (in different messages is fine).

Not concepts:

- **Vague nouns**: "某个 app", "那个问题", "the bug". Leave them out and let the results show
  which app it was.
- **The outcome**: "解决", "修好", "fixed", "问题". Nearly every session has these words. Check
  the outcome when you verify.
- **Time words**: "近期", "recently" → `-d 30`; "上周", "last week" → `-d 14`; a date →
  `--since 2026-11-01`. Widen with `--days 0` (all time) when nothing fits.
- **The project or app the work happened in.** The user typed in that project's directory, so
  they rarely named it. Filter with `-p` instead: a directory (`-p ~/Code/homehub`) or part of
  its path (`-p homehub`). `asm projects -d 30` lists the candidates when the name is loose.
  Use a name as a TERM only when it is something people talk about inside a session: a
  device, a service, a host.

Start with one or two concepts. Every extra concept must be in the same session, and with three
or more you often lose the session you want because it phrased one of them differently.

## 2. Ways of saying each concept

Inside a TERM, `|` separates ways of saying one concept; any one of them counts. Quote the
whole TERM.

- **Both languages.** The user here writes mostly Chinese, the agent answers in the user's
  language, and code, commands, logs and error messages are English. Always give Chinese and
  English wordings, whichever language the request was in.
- **Synonyms and registers**: `连不上|连接不上|无法连接|连接失败|断网`.
- **What the problem prints**: error text and codes (`connection refused|ECONNREFUSED|URLError`,
  `ETIMEDOUT`, `502`), and the commands, endpoints and files involved (`curl`, `healthz`,
  `known-issues.md`).

Matching is literal and case-insensitive, on substrings: `连不上` also finds `连不上网`, and
`offline` also finds `dispatcher_offline` and `offlinePage`. Prefer the short stem in Chinese;
in English, watch for stems that are parts of common identifiers.

**One common alternative ruins the whole concept.** `error`, `fix`, `timed out`, `问题`, `网络`
are in hundreds of sessions, so a TERM that lists one of them matches almost everything.
Start with the specific wordings and add broad ones only when results are thin. A broad English
word does the same on a smaller scale: `offline` also brings in HTML templates and offline-mode
features, so prefer the phrase the problem would be told with (`设备离线|device offline`).

Literal TERMs have no wildcards: `can.t` looks for a dot. Write `"can't connect"` (double
quotes because of the `'`), or use `--regex` (`'connect(ion)? refused'`), which scans files
instead of the index: slower, and the last 30 days by default.

## 3. Search, then adjust

```sh
asm search '<concept A>' ['<concept B>'] -d 30
```

The first line says how many sessions matched, which kinds were searched, the range and the
time taken; `showing 20 of 222 sessions` means the list was cut.

**Too many** (dozens or hundreds):

- `--in prompt`: only where the user wrote it. Best for problems the user reported ("X 连不上了").
- Add a concept, or `-p` for the project.
- `--sort hits`: the sessions that talk about it most first.
- `--same-message`: every TERM in the same message.
- A shorter range.

**Too few or none**: the `try:` line lists what can be widened (`--days 0 for all time`,
`--in all for every kind`, `fewer terms`, …). Also add wordings, or drop a concept.

**Searching tool calls** (`--in tool-output`, `--in tool`, `--tool Bash`, `--in all`) is for
words that only appear there: an exact error message, a command line, a path, a stack trace.

- Tool output includes every file the agent read: plans, notes, issue lists, earlier
  summaries. A hit there often means the session read a document that mentions the error, not
  that it ran into it (see example 2). Search the conversation first.
- It reads every transcript in range, about half a second per GB and a few seconds when the
  files are not in the OS cache, and parses the files that contain every TERM. Keep the range
  short (`-d 7` to `-d 30`) and the words specific: a common word gets every file parsed and
  floods the results.
- `--tool Bash` searches only that tool's input and output.

Many sessions here are orchestrated: a coordinator session hands tasks to worker sessions whose
first prompt reads like "你是 xxx，负责……" (handed over in a new session, not a subagent). The
coordinator mentions every problem in passing in its status reports, so it collects many reply
hits; the worker has the details. Both are real results: the worker usually holds the fix, the
coordinator the decisions and the outcome.

## 4. Pick candidates from the output

Each result gives enough to judge it without opening it:

- `hits:` per kind. Prompt hits: the user raised it. Many reply hits: it was discussed at
  length. Thinking only: the agent noticed it in passing (only about one thinking block in ten
  still has text, so thinking rarely decides anything). Tool output only: maybe a document it
  read.
- The snippets: which turn, when, in what words. `«»` marks the hits, `⏎` a line break (or a
  literal `⏎` in the text).
- `title:` and `prompt:`: what the session was for. A session about something else with one
  hit is usually a passing mention.
- Project, dates, turns and cost: where it happened and how big it is.

Pick one to three to verify; do not open ten.

## 5. Verify in the transcript

```sh
asm transcript ID --outline
asm transcript ID --turns N --grep '<the same TERM>' ['<other TERM>']
```

- `--outline` lists every turn's first line: often enough to see where the problem was raised
  and whether a later turn reports it fixed.
- Take N from the snippet (`[reply · turn 6 · …]`) and grep the same TERMs that found it. A
  turn shows its prompt, every item with a hit and the item before and after each; the rest is
  folded into lines like `…[skipped 14 items, 9120 chars; --turns 6 without --grep shows them]`.
- A `tool-input` or `tool-output` snippet needs `--tools`; a thinking hit is shown without
  `--thinking`. When stderr says `note: 2 turns have hits only in tool calls; add --tools to read
  them`, add `--tools` if you need those hits.
- To see how a turn ended, read it whole: `--turns N` without `--grep`. Check its size first on
  big sessions (`asm session ID` shows the turns and the transcript's size); use `--context 1`
  or `--turns N-M` for the turns around it.
- Long tool bodies are cut around the hits; `…[cut 13288 chars, 3 more hits]` means some hits
  were not shown: grep a narrower TERM.
- An `<other>` element can be a compact summary: Claude Code's recap of everything before it in
  the session. It is often the best one-place account of a cause and its fix, but it repeats
  earlier text, so its hits duplicate earlier ones.
- Put the ID before `--grep`, which takes every following word. A TERM that starts with `-` is
  attached and alone in its `--grep`: `--grep=-rf --grep other`.

Verified means: you read the turn where the problem is described and the one where it was
resolved (or saw that it was not), and you can say in one sentence what happened.

## 6. Report

Answer in the user's language. Best match first, at most about five:

```
**<one-line conclusion: what the problem was and how it ended>**
- Session: `<full ID>` · <project> · <started> → <last update> · <N> turns
- Where: turn N (<date>): <what happened there>
- Resume: `<the resume command exactly as asm printed it>`
```

- Say which sessions you verified and which you only judged from snippets.
- When the results cover several different apps or problems, group them and ask which one the
  user meant.
- When nothing fits, say what you searched (TERMs, kinds, range) and offer to widen it.
- `resume: none (…)` means the session cannot be resumed; say why.

## Example 1: 「帮我找一下近期关于某个 app 连不上网的问题解决会话」

Concepts: "连不上网" is the only one. "某个 app" is unknown (let the results say which),
"问题解决" is checked when verifying, "近期" is `-d 30`. The commands and the shape of their
output are what `asm` printed in a real run (2026-11-17), trimmed; the projects, devices, work
and session IDs are made up.

Search the conversation first:

```
$ asm search '连不上|连接不上|无法连接|连接失败|断网|离线|connection refused|ECONNREFUSED|unreachable|URLError' -d 30
222 sessions (showing 20) · terms [连不上|连接不上|无法连接|连接失败|断网|离线|connection refused|ECONNREFUSED|unreachable|URLError] · in prompt,reply,thinking · last 30 days · 0.85s
…
```

222 is too many: agents report connection trouble in passing all the time. The user reported a
problem, so look where the user wrote it:

```
$ asm search '连不上|连接不上|无法连接|连接失败|断网|离线|connection refused|ECONNREFUSED|unreachable|URLError' -d 30 --in prompt
26 sessions (showing 20) · terms […] · in prompt · last 30 days · 0.10s
…
8301138e-cefb-4438-91e5-8056683c6c78
  updated 2026-11-17 00:09 · started 2026-11-16 12:09 · notekeeper (master) · 4 turns · $8.71
  title: 我们有没有做过跨设备同步，以及有没有列出过相关的 plan？…
  hits: prompt 1
  [prompt · turn 2 · 11-16 12:15] …首先，这个扩展仍然是一个以«离线»阅读为主的东西。…
  resume: …
…
77f49eea-861d-40f4-88f8-ea14aeb8a4fb
  updated 2026-11-14 23:42 · started 2026-11-14 22:41 · homehub (master) · 4 turns · $9.77
  title: 你是 hh-p2-relay，负责 P2 的设备配对和把中继服务交给 launchd 运行。…
  hits: prompt 1
  [prompt · turn 2 · 11-14 23:07] … 第 3 项有问题：控制面板里点「启动场景」没反应，设备列表显示「找不到设备」或「设备«离线»」，提示刷新后重试，刷新也没用。请先查清原因…
  resume: cd '/Users/me/Code/homehub' && claude --resume 77f49eea-861d-40f4-88f8-ea14aeb8a4fb
…
showing 20 of 26 sessions, use -n for more
```

The first is a passing use (`离线` as in "offline-first"). The second is a user's report that the
homehub control panel showed its devices as offline, sent to a worker session to investigate.
Verify it:

```
$ asm transcript 77f49eea --outline
77f49eea-861d-40f4-88f8-ea14aeb8a4fb · 4 turns · 2026-11-14 22:41 to 2026-11-14 23:40
   1  2026-11-14 22:41  你是 hh-p2-relay，负责 P2 的设备配对和把中继服务交给 launchd 运行。…
   2  2026-11-14 23:07  用户的核对结果（经协调者转达）：1 ok；2 ok；3 有问题——…「设备离线」…
   3  2026-11-14 23:10  协调者：用户人不在 Kestrel 旁边，只能经 SSH 连到会话，今晚点不了授权框。…
   4  2026-11-14 23:33  用户的结果（经协调者转达）：第 3 项重试成功，书房的灯和传感器都重新上线了，…

$ asm transcript 77f49eea --turns 2 --grep '离线|offline|找不到设备'
<transcript session_id="77f49eea-861d-40f4-88f8-ea14aeb8a4fb" turns="4" matched="1">

<user turn="2" ts="2026-11-14 23:07">
用户的核对结果（经协调者转达）：…设备列表显示「找不到设备」或「设备离线」…
</user>

<turn-summary>
Called Bash 23 times, Read 1 time.
</turn-summary>
<agent>
先查日志，找中继服务发现设备失败的原因。
</agent>
<agent>
原因查清了，需要你到 Kestrel 前点一次授权。
**原因**：「设备离线」不是设备掉线，是中继服务在局域网里发现不了设备，控制面板就把它们都标成离线。…
- 现在中继由 launchd 运行，macOS 第一次放行它的局域网请求要你在「本地网络」权限框里点允许，没人点就一直被拦着。…
…
</agent>

</transcript>
```

Turn 2 has the cause, and the outline's turn 4 says the retry worked: verified (3 KB read).

Problems the agent ran into without the user typing them show up in replies instead. Add a
second concept to get at one of them, here a health check:

```
$ asm search 'healthz|健康检查|health check' '连不上|连接不上|无法连接|连接失败|断网|离线|connection refused|ECONNREFUSED|unreachable|URLError' -d 30
24 sessions (showing 20) · terms [healthz|健康检查|health check] […] · in prompt,reply,thinking · last 30 days · 0.14s
…
50414f9c-beca-471d-ace4-c28d0492db05
  updated 2026-11-14 18:21 · started 2026-11-12 23:38 · fieldcam (master) · 37 turns · $29.25
  hits: prompt 0 · reply 10 · thinking 1
  [reply · turn 4 · 11-13 08:06] …48 小时录制任务今天 04:02 就自己停了。** 第 9 轮开始前«健康检查»«连不上»摄像头网关（`«healthz»_error:«URLError»`），触发了停止条件。…原因还没查…

f67360e0-0f70-49e8-9a68-8d2c700fe519
  updated 2026-11-13 22:38 · started 2026-11-13 16:02 · fieldcam (master) · 4 turns · $3.64
  hits: prompt 0 · reply 2 · thinking 1
  [reply · turn 1 · 11-13 16:17] …*：不是网关的问题，是路由器每天 04:00 自动重启，重启的两分钟里对网关 `/«healthz»` 的请求报了 `«URLError»`…
…
```

The first session reported the failure ("原因还没查"); the second found the cause. Check the
second the same way (`--outline`, then `--turns 1 --grep 'healthz|健康检查' 'URLError|连不上'`, 4 KB).

The report:

```
近期「app 连不上」的会话里，核实过的有两个：

**homehub 控制面板显示「设备离线」、场景启动不了：中继服务改由 launchd 运行后，macOS 要一次「本地网络」授权，没人点就发现不了设备；点了允许后重试成功。**
- 会话：`77f49eea-861d-40f4-88f8-ea14aeb8a4fb` · homehub · 2026-11-14 22:41 → 23:42 · 4 回合
- 位置：第 2 回合查清原因，第 4 回合确认重试成功
- 恢复：`cd '/Users/me/Code/homehub' && claude --resume 77f49eea-861d-40f4-88f8-ea14aeb8a4fb`

**fieldcam 录制任务的健康检查连不上摄像头网关：原因是路由器每天 04:00 自动重启，重启期间请求报 URLError，不是网关的问题。**
- 会话：`f67360e0-0f70-49e8-9a68-8d2c700fe519` · fieldcam · 2026-11-13 16:02 → 22:38 · 4 回合
- 位置：第 1 回合（调研结论第 6 条）；失败最早在 `50414f9c…` 第 4 回合报告
- 恢复：`cd '/Users/me/Code/fieldcam' && claude --resume f67360e0-0f70-49e8-9a68-8d2c700fe519`

另外 24 个只在提示里提到「连不上 / 离线」的会话没有逐个核实；你说的如果是别的 app，告诉我名字我再缩小范围。
```

## Example 2: "Where did we fix the flaky ECONNRESET in the orderdesk tests?"

Concepts: the error is the one concept (`ECONNRESET|socket hang up`); "orderdesk" is the project,
so `-p orderdesk`; "flaky" and "fix" are outcomes. The error is something a test prints, so
searching tool output looks natural; it is the wrong first step:

```
$ asm search 'ECONNRESET' --in tool-output -p orderdesk -d 14
28 sessions (showing 20) · terms [ECONNRESET] · in tool-output · project orderdesk · last 14 days · 1.26s

253b480c-e454-474a-bcd1-4b88c4535ecd
  updated 2026-11-16 23:20 · started 2026-11-16 19:53 · orderdesk (master) · 2 turns · $2.79
  hits: tool-output 2
  [tool-output Bash · turn 1 · 11-16 20:02] …report-export 的 «ECONNRESET»（测试客户端复用了服务端已经关掉的空闲连接，改为让两边的 keep-alive 超时对齐）…⏎240-- 处理方向：…
…
```

Those hits are agents reading the known-issues file, which records the fix. The conversation
finds the session that did it, from all time, in half a second:

```
$ asm search 'ECONNRESET|socket hang up' -p orderdesk
1 session · terms [ECONNRESET|socket hang up] · in prompt,reply,thinking · project orderdesk · all time · 0.50s

f40ec708-cb69-4092-a58d-698adba30710
  updated 2026-11-11 16:38 · started 2026-11-11 15:15 · orderdesk (master) · 5 turns · $18.67
  title: 你负责 orderdesk 测试整理后续的任务 A（偶发失败的用例与死代码）。…
  hits: prompt 0 · reply 2 · thinking 0
  [reply · turn 1 · 11-11 15:18] 先确认 report-export 的 «ECONNRESET» 是不是 keep-alive 连接复用的竞态。
  [reply · turn 3 · 11-11 16:12] …服务端的 keep-alive 只有 5 秒，测试客户端的连接池把空闲连接留 15 秒，复用到服务端刚关掉的连接时就报 «ECONNRESET»。把服务端的超时调到…
  resume: cd '/Users/me/Code/orderdesk' && claude --resume f40ec708-cb69-4092-a58d-698adba30710

$ asm transcript f40ec708 --turns 1 --grep ECONNRESET
…
<agent>
先确认 report-export 的 ECONNRESET 是不是 keep-alive 连接复用的竞态。
</agent>
…[skipped 31 items, 1393 chars; --turns 1 without --grep shows them]
<other>
This session is being continued from a previous conversation that ran out of context. …
   - report-export ECONNRESET: mechanism — the test client's agent keeps idle sockets for 15 s while the server's keepAliveTimeout is 5 s, so a reused socket can already be closed; …
…[cut 7694 chars]
</other>
…
```

The `<other>` is a compact summary: the mechanism and the commits in one place (4 KB read).
Report the session, turn 3 for the fix and turn 1 for the summary, and the resume command.

## Pitfalls

- **The current session is not in the results.** If the user asks about this conversation, add
  `--include-self`.
- **Project names are rarely in the text**: use `-p`, not a TERM.
- **Thinking is mostly empty**: about nine blocks in ten kept only a signature.
- **`other` repeats**: compact summaries restate earlier turns, so the same fact can hit twice.
- **Snippets are cut** (140 characters from the hit) and one line: `⏎` is a line break (the
  original text can also contain the character itself), and the text around may change the
  meaning. Verify before you conclude.
- **Sessions still being written** (including other agents' running now) can change between two
  commands. A range keeps every session active in it, so a long session that started earlier
  and is still written today is in every `--until`; get past it with `-p` or `--sort hits`.
- **Subagent conversations are not searched.** What a subagent was asked and what it answered
  are in the parent's tool call: `--tool Agent --tool Task` (`Task` in older sessions).
- **Text in `<user>` is not always typed by the user**: `<command-name>` (a slash command),
  `<system-reminder>` and similar tags come from Claude Code, `<pasted_content>` from a paste,
  and in orchestrated work a coordinator writes the worker's prompts.
