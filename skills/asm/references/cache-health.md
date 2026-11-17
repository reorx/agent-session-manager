# Checking prompt cache health

The user asks how well prompt caching worked ("看看最近的会话缓存命中得好不好", "这个会话为什么这么贵",
"why do my sessions keep rebuilding the cache?", "are we using prompt caching well?"). Answer
with the hit rate, the cache rebuilds by cause and what they cost, the sessions worth a look,
and what could be done differently, each point backed by what the records show and by the
official documentation. It usually takes three to six commands:

1. Overview of the range.
2. Groups and rankings: where the rebuilds and the low hit rates are.
3. One to three sessions: their rebuilds, and the turns around the ones that matter.
4. Match each cause with what the records show and what the documentation says about it.
5. Report.

Option details are in [cli.md](cli.md) (`### asm cache`). `asm cache` only reads the index; it
never changes Claude Code's settings, and neither should you: suggest, and let the user decide.

## Terms

- **Input tokens** of a call: uncached input + cache writes + cache reads, the three input
  fields of the API's `usage`.
- **Hit rate**: cache reads ÷ input tokens. The same number as the app's Hit% column and the
  `hit` in `asm session ID`.
- **Cache write share** and its `(1h N%)`: the part of the input written to the cache, and how
  much of that was written with the one-hour lifetime (the rest: five minutes). Every call
  writes its new tail; a high write share means large parts were written again.
- **Main-thread call**: one API response in the session's own conversation. Subagent calls
  count in the token totals and hit rates, but are not main-thread calls and are never judged
  as rebuilds.
- **Rebuild**: a main-thread call that read at least 10,000 fewer cached tokens than the call it
  continues (after `/rewind`, the point it went back to; a rewind alone is not a rebuild). Each
  rebuild has one cause, the first that applies in the order of the cause list below.
  `dropped` is how much less it read, `written` how much it wrote.
- **Rebuild cost**: the price of that call's cache writes, the amount the app shows on the turn
  card. It is not what a hit would have saved (a hit costs too, and the call would still have
  written its new tail): never present it as savings.
- **Range**: the overview counts calls made in the range, also in sessions that began earlier.
  `--days N` is N × 24 hours back from now (default 7); `--since` / `--until` are local dates,
  `--until` included. `--by day` uses calendar days from midnight, not the app's Usage days
  that start at 4:00.

## 1. Overview

```sh
asm cache            # last 7 days
asm cache -d 30      # a longer range, to see whether this week is unusual
```

```
last 7 days (default) · 412 sessions · 18233 main-thread calls
input 1420518337 tokens: cache read 97.9% · cache write 2.1% (1h 93.4%) · uncached 0.0%
rebuilds 96 · 9804120 tokens written · $61.92
  compaction  33 · $16.40
  idle  58 · $41.80 · median gap 102.5m
  model_switch  2 · $1.62
  request_settings  1 · $0.80
  system_or_tools  2 · $1.30
top sessions by rebuild-cost:
3c7e91a2-58d4-4b6f-a0e3-9f12c4d6b875  2026-11-14T17:09:55+08:00  fieldcam · hit 95.1% · rebuilds 7 · $5.34  录制任务停止排查
77f49eea-861d-40f4-88f8-ea14aeb8a4fb  2026-11-14T23:42:10+08:00  homehub · hit 96.8% · rebuilds 4 · $4.35  你是 hh-p2-relay，负责 P2 的设备配对和把中继服务交给 launchd 运行。…
…
showing 10 of 412 sessions, use -n for more
```

- Line 1: the range, the sessions with any call in it, the main-thread calls.
- Line 2: the input split. Token counts and times are printed in full (`1420518337` is 1.42
  billion; times are RFC 3339 with the local offset).
- Line 3 and the indented lines: rebuilds, tokens they wrote, their cost, then one line per
  cause that occurred (alphabetical), with its count and cost; `idle` adds the median gap
  between the two requests in minutes.
- Then the sessions with the most rebuild cost: ID, last update, project, the session's hit
  rate, rebuilds and rebuild cost within the range, title. Unknown values print `—`, the cost
  of a model without a price `unpriced`.
- `0 calls in range`: nothing was called in the range; widen it.
- The session you run `asm` from is left out; `--include-self` puts it back.

## 2. Find where it happens

```sh
asm cache --by day          # or --by project, --by model: one row per group, under the causes
asm cache -p fieldcam       # one project (directory or part of its path, repeatable)
asm cache --model opus-5-5  # only calls made with that model (subagents included)
asm cache --sort hit -n 20  # lowest hit rate first; --sort tokens: most input first
```

A group row is `<key>  hit … · write … · rebuilds … · $…`; `--by project` keys are project
paths, and lists every project with a call in the range. A day with many rebuilds, or one
project with most of the cost, tells you where to look; a model row shows the rebuilds made with
that model (by the model of the call, not the session's main model).

**A low hit rate on a short session is normal.** Its first call writes the system prompt, the
tools and the project context, and a session of a few calls never reads them back many times.
`--sort hit` therefore starts with one-call sessions at 0%. To rank sessions of some length,
filter the JSON:

```sh
asm cache -n 0 --json | jq -r '.results[] | select(.stats.main_thread_calls >= 50)
  | [.session_id[0:8], .project, .stats.hit_rate, .stats.rebuilds, .stats.cost_usd, .title[0:40]] | @tsv' \
  | sort -t$'\t' -k3,3n | head
```

A session that runs many subagents can show a lower hit rate than its own conversation: each
subagent builds a cache of its own, with a five-minute lifetime unless configured otherwise.

## 3. Look at one session

```sh
asm cache 3c7e91a2            # ID or unique prefix; always the session's whole lifetime
asm cache 3c7e91a2 --calls    # adds every main-thread call
```

```
all time · 1 sessions · 412 main-thread calls
3c7e91a2-58d4-4b6f-a0e3-9f12c4d6b875  2026-11-14T17:09:55+08:00  fieldcam  录制任务停止排查
started 2026-11-13T07:40:12+08:00 · updated 2026-11-14T17:09:55+08:00
input 52310877 tokens: cache read 95.1% · cache write 4.9% (1h 100.0%) · uncached 0.0%
rebuilds 7 · 667500 tokens written · $5.34
  compaction  2 · $0.64
  idle  3 · $2.59 · median gap 181.4m
  model_switch  1 · $1.12
  system_or_tools  1 · $0.99
turn 9 · 2026-11-13T09:41:37+08:00 · model_switch from claude-sonnet-5-5 · dropped 136402 · written 140000 · $1.12
turn 11 · 2026-11-13T12:03:18+08:00 · idle · gap 7212334ms / TTL 3600000ms · dropped 118220 · written 120000 · $0.96
turn — · 2026-11-13T13:05:31+08:00 · compaction · dropped 182330 · written 41250 · $0.33
turn 12 · 2026-11-13T16:48:09+08:00 · idle · gap 10883021ms / TTL 3600000ms · dropped 96870 · written 98750 · $0.79
turn 14 · 2026-11-13T19:20:44+08:00 · system_or_tools · dropped 121644 · written 123750 · $0.99
turn — · 2026-11-14T01:12:06+08:00 · compaction · dropped 175912 · written 38750 · $0.31
turn 20 · 2026-11-14T13:41:27+08:00 · idle · gap 41255390ms / TTL 3600000ms · dropped 102317 · written 105000 · $0.84
```

- One line per rebuild, in transcript order: turn, time, cause, `dropped`, `written`, cost.
  `idle` gives the gap and the lifetime in milliseconds (÷ 60,000 for minutes: 7212334ms is
  2h 0m, 3600000ms is 1 h); `model_switch` names the model before; `effort a → b`;
  `fast_mode on=true|false`.
- `turn —`: the call opened no numbered turn of its own, either before the first prompt or in an
  exchange Claude Code continued without a prompt (after an automatic compact, a finished
  background task). Most compaction rebuilds are like this. Find it by time: it belongs after
  the last turn in `--outline` that started before it.
- The hit rate here is the session's whole lifetime, the same as `asm session ID` and the app's
  Hit%; in the overview it was only the calls in the range.
- `--calls` adds a line per call: `time · turn · model · prompt N · read N · write 5m N / 1h N ·
  uncached N`, and `· rebuild` on the rebuilds. Use it to see when the call before a rebuild
  was made and what it looked like, and which lifetime the session's writes used (the `5m` and
  `1h` columns). Each rebuild with the call before it:

```
$ asm cache 3c7e91a2 --calls | grep -B1 -- '· rebuild$' | head -2
2026-11-13T09:31:12+08:00 · turn 7 · claude-sonnet-5-5 · prompt 136402 · read 134950 · write 5m 0 / 1h 1450 · uncached 2
2026-11-13T09:41:37+08:00 · turn 9 · claude-opus-5-5 · prompt 140004 · read 0 · write 5m 0 / 1h 140000 · uncached 4 · rebuild
```

The calls are in transcript order, so after a `/rewind` the call a rebuild continues can be
further up.

Then read the turns around the rebuilds that matter. The cause says what changed; the transcript
often says why:

```sh
asm transcript 3c7e91a2 --outline      # local commands such as /model, /compact, /cd are turns of their own
asm transcript 3c7e91a2 --turns 8-9    # the turn of the rebuild and the one before
```

A model or setting change usually shows up as a prompt like `/model opus` in the turn just
before the rebuild. Keep the reading short (see the rules in SKILL.md).

## 4. Causes

Each cause is a fact found in the records; the "usually from" lines are the documented actions
that produce it. The records do not say which action it was: name it only when the transcript
shows it (a `/model` prompt, say), and otherwise say "usually from …". Sources were checked on
2026-11-17; where a page has changed since, the page wins:

- Claude Code, [Prompt caching](https://code.claude.com/docs/en/prompt-caching) (links below
  point to its sections).
- Claude API, [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
  and [Pricing](https://platform.claude.com/docs/en/about-claude/pricing#prompt-caching).

Background that applies to every cause: the API caches by exact prefix, in the order tools →
system → messages, so a change anywhere re-reads everything after it. A cache entry lives for
its lifetime (five minutes or one hour) from the start of the last request that wrote or read
it. Writing costs 1.25× the base input price for five minutes and 2× for one hour; reading costs
0.1× on most models (0.05× on Opus 5.5 and Sonnet 5.5, 0.025× on Fable 5.1 / 5.2).

Sessions recorded by Claude Code before 2.1.300 have no request records: for them `asm` can
name only compaction, model_switch, effort, fast_mode and idle, and everything else is
`unknown`.

**Check the gap before you call a rebuild avoidable.** The causes are tried in the order of the
sections below, and compaction, model_switch, effort, fast_mode, sign_in and request_settings
come before idle. So a change is named even when the call came long after the one before it,
when the cache had expired anyway: a session resumed days later is often a `request_settings`
rebuild, and a `/model` typed after a long break a `model_switch`. Only idle rebuilds print the
gap; for the others, compare the rebuild's time with the call above it in `--calls` (or the
turns in `--outline`). Longer than the lifetime (one hour with one-hour writes): the change
cost nothing extra, and the rebuild belongs with the idle ones. On the machine this skill was
written on, 6 of the 8 such rebuilds in one week were like that.

### compaction

- **Seen**: the first rebuild after a compact (`/compact`, automatic compaction, idle compaction)
  in the session. When the call right after the compact still read the cache, the next real
  rebuild carries it.
- **Documented**: compaction replaces the history with a summary, which by design invalidates
  the conversation layer; the project context is cached with the conversation, so it is
  rebuilt too. The rebuild after it is for the much shorter history, so it is cheap next to the
  context it replaced.
  ([Compacting the conversation](https://code.claude.com/docs/en/prompt-caching#compacting-the-conversation))
- **What helps**: usually nothing; one rebuild per compaction is the expected price. Worth a
  remark only when one session compacts many times. `/compact` at a natural break chooses when
  the cost lands, instead of an automatic compaction mid-task; to abandon a path, `/rewind`
  keeps the earlier cache instead. Claude Code can compact a long conversation left idle on the
  one-hour lifetime; `idleCompaction: false` turns that off (Claude Code 2.1.290 or later).

### model_switch (`from` the previous model)

- **Seen**: the call used another model than the call it continues.
- **Usually from**: `/model`; the `opusplan` setting (Opus in plan mode, Sonnet outside it, so
  every plan-mode toggle switches); automatic model fallback after a safety classifier flag; a
  skill or command whose frontmatter names a `model` (the session model returns on the next
  prompt, a second switch); `/fast` on a model without fast mode. Each model has its own cache.
  ([Switching models](https://code.claude.com/docs/en/prompt-caching#switching-models))
- **What helps**: choose the model at the start of the session; for another model on a large
  context, a new session with a short handoff writes less than switching in place.

### effort (`from → to`)

- **Seen**: both calls recorded an effort, they differ, and the cache was lost. An effort change
  that kept the cache is not listed anywhere.
- **Documented**: on most models each effort level has its own cache. With an API key or a
  Claude subscription, Fable 5.1 and later, Opus 5.5, Sonnet 5.5 and Haiku 5.5 keep the cache
  across effort changes, except on Amazon Bedrock, Google Cloud's Agent Platform, a Claude apps
  gateway, with `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS`, or under a HIPAA configuration.
  ([Changing effort level](https://code.claude.com/docs/en/prompt-caching#changing-effort-level))
- **What helps**: set the effort at the start of the session.

### fast_mode (`on=true` turned on, `on=false` turned off)

- **Seen**: one call ran in fast mode and the other at standard speed.
- **Documented**: the two speeds keep separate caches; Claude Code's own fallback to standard
  speed after a rate limit, and the return, count as toggles too.
  ([Turning on fast mode](https://code.claude.com/docs/en/prompt-caching#turning-on-fast-mode))
- **What helps**: turn fast mode on at the start of a session, not in the middle of a long one.

### sign_in

- **Seen**: one of the two requests carried the subscription sign-in marker in its beta list and
  the other did not.
- **Documented**: caches are isolated between organizations, and between workspaces on the
  Claude API; requests signed in to different ones cannot share a cache.
  ([Cache storage and sharing](https://platform.claude.com/docs/en/build-with-claude/prompt-caching#cache-storage-and-sharing))
- **Likely from** (an inference, not documented as such): switching between a Claude
  subscription and an API key (an `ANTHROPIC_API_KEY` set in one shell, `/login`). The records
  do not say which way; say "the sign-in changed".
- **What helps**: keep one sign-in for a session.

### request_settings

- **Seen**: the beta list of the request changed in another way.
- **Usually from**: the betas Claude Code sends depend on its version, the sign-in, the model
  and settings; the one-hour lifetime, for one, travels partly in the `anthropic-beta` header
  ([Choose the TTL yourself](https://code.claude.com/docs/en/prompt-caching#choose-the-ttl-yourself)).
  The index does not keep which beta changed.
- **What helps**: ask the user what they changed around that time (a setting, an environment
  variable, a Claude Code restart); there is no general fix.

### idle (`gap … / TTL …`)

- **Seen**: the time between the start of this request and the start of the request it
  continues (reply times when there are no request records) is longer than the lifetime of
  the part of the cache that was lost.
- **Documented**: Claude Code asks for one hour on the main conversation with a Claude
  subscription within the plan's usage, and five minutes with an API key, usage credits, a
  cloud provider, or once the plan's usage limit is passed. `promptCacheTtl` or
  `CLAUDE_CODE_PROMPT_CACHE_TTL` (`5m` / `1h`, Claude Code 2.1.242 or later) choose it;
  `FORCE_PROMPT_CACHING_5M=1` overrides everything. One hour costs more per write and pays
  off when sessions sit idle and are picked up again; on short bursts it costs more.
  ([Cache lifetime](https://code.claude.com/docs/en/prompt-caching#cache-lifetime),
  [Which TTL each request gets](https://code.claude.com/docs/en/prompt-caching#which-ttl-each-request-gets))
- **What helps**:
  - Gaps a bit over five minutes (up to an hour) with five-minute writes: the one-hour lifetime
    would have kept the cache. Check the writes first (below), then mention the setting with
    its higher write price.
  - Gaps of hours, or anything over an hour with one-hour writes: no setting keeps a cache that
    long. This is the cost of coming back to a long conversation; worth saying, rarely worth
    fixing. If the user would compact the session anyway, compacting before a long break,
    while the cache is warm, costs less than after it.
- **Check the lifetime before you talk about it.** When a rebuild lost (almost) everything, the
  lifetime shown is that of the start of the session's cache, which is often read from an
  earlier session in the same directory; its lifetime is not in the records, and `asm` then
  assumes five minutes. So `TTL 300000ms` alone does not mean the session used five-minute
  caching: look at the `(1h N%)` on the input line and the `write 5m / 1h` columns of `--calls`.
  If the session wrote only one-hour entries, describe the gap, not the setting.

### system_or_tools

- **Seen**: the fingerprint of the system prompt and tool definitions in the request records
  changed.
- **Usually from**: an MCP server connecting or a dynamic tool update adding tools while the
  tools are loaded up front (with tool search, the default on supported models, connecting a
  server mid-session changes nothing); removing a tool on purpose (a bare deny rule such as
  `Bash`, disabling a server in `/mcp`) under the same condition; enabling or disabling a
  plugin that provides MCP servers; `/cd` to another directory, which applies that directory's
  plugins; a remote MCP server reconnecting, which can add `WaitForMcpServers` once.
  ([Connecting or removing an MCP server](https://code.claude.com/docs/en/prompt-caching#connecting-or-removing-an-mcp-server),
  [Enabling or disabling a plugin](https://code.claude.com/docs/en/prompt-caching#enabling-or-disabling-a-plugin),
  [Denying an entire tool](https://code.claude.com/docs/en/prompt-caching#denying-an-entire-tool))
- **What helps**: set up servers, plugins and deny rules before starting the work; for another
  directory, a new session there.

### earlier_messages

- **Seen**: the request kept fewer of its leading messages unchanged than the previous request
  had: Claude Code rewrote messages it had already sent. The index does not keep which ones.
- **Documented**: when a request would pass the limits on images and PDFs, Claude Code removes a
  batch of the oldest ones, and the conversation is processed again from the earliest of them.
  ([Accumulating many images](https://code.claude.com/docs/en/prompt-caching#accumulating-many-images))
- **What helps**: fewer or smaller screenshots in a long session; a new session for image-heavy
  work.

### request_unchanged

- **Seen**: by the request records nothing changed (same fingerprint and settings, the leading
  messages kept) and the cache had not expired, yet the call did not read it.
- **Documented**, as possible reasons, not findings: a cache entry becomes readable only after
  the first response that writes it starts, so requests sent in parallel miss it
  ([Cache limitations](https://platform.claude.com/docs/en/build-with-claude/prompt-caching#cache-limitations));
  caches are isolated by organization and workspace.
- **What helps**: usually nothing; mention it only when it repeats in one project, and say that
  the records do not explain it.

### unknown

- **Seen**: no request records for the call (Claude Code before 2.1.300), and none of the causes
  that need no request record applies. Report the count; there is nothing more to read.

## 5. What is normal

Measured on the machine this skill was written on, 30 days to 2026-11-17 (1,669 sessions,
107,219 main-thread calls, Claude subscription). Compare the user's numbers with their own
`asm cache -d 30` as well; another setup can differ:

- Hit rate 98.3%, cache writes 1.7% of input (90% of them one-hour), uncached input under 0.1%.
- 0.6 rebuilds per 100 main-thread calls. Idle 51% of the rebuilds and 76% of their cost
  (median gap about 3 hours), compaction 43% and 18%; the other causes together about 6%.
- Session hit rate by size (median, with the 10th percentile): under 10 calls 88% (36%);
  10–49 calls 96% (92%); 50–199 calls 98.3% (96.7%); 200 calls or more 98.7% (98.0%).
  More than half the sessions with 50 calls or more had no rebuild.

Worth raising:

- A session with 50 calls or more and a hit rate clearly under its size's 10th percentile.
- Any `model_switch`, `effort`, `fast_mode`, `sign_in`, `request_settings`, `system_or_tools` or
  `earlier_messages` that came within the cache lifetime of the call before it: these come from
  something that changed, and the turn before usually shows what.
- Idle rebuilds with five-minute writes and gaps under an hour.
- Several rebuilds of one cause in one session, or a day or project far above the rest.

Not worth raising on their own: a low hit rate on a short session, one rebuild after each
compaction, idle rebuilds after breaks of hours, a change-caused rebuild that came after such a
break, a handful of `request_unchanged` or `unknown`.

## 6. Report

Answer in the user's language, numbers from the commands (round them, keep the cost to the
cent):

```
**<range>: hit rate N%** (input N tokens: read N% · write N% (1h N%) · uncached N%)
**Rebuilds N · $N**: <cause> N · $N (<what it was>); … (every cause with a rebuild)
**Sessions worth a look** (at most about five):
- `<ID prefix>` <project> "<title>" · hit N% · rebuilds N · $N: <which causes, at which turns, what the transcript shows>
**What could change** (only where the records show it; each with its source):
1. <suggestion> (<doc section link>)
```

- Say which sessions you looked into and which you only judged from the overview.
- Give the rebuild cost as what the rebuilds cost, never as what the user would have saved.
- Name a setting with its effect and trade-off (`promptCacheTtl: "1h"`: writes cost 2× instead
  of 1.25×); do not change it yourself.
- When the numbers are within the normal ranges above, say so; not every report needs advice.

## Example: 「看看最近 7 天缓存命中得好不好，哪些重建本可以避免」

The commands and the shape of their output are what `asm` printed in a real run (2026-11-17),
trimmed; the projects, sessions, work and numbers are made up.

Overview first (output in section 1): 97.9% hit, 96 rebuilds for $61.92, mostly idle and
compaction, five rebuilds from changes. The top session holds a model switch and a system
change, so look at it (output in section 3), at the calls before those two rebuilds (the
`grep -B1` in section 3 and the same for 19:20), then at the turns before them:

```
$ asm transcript 3c7e91a2 --outline
3c7e91a2-58d4-4b6f-a0e3-9f12c4d6b875 · 21 turns · 2026-11-13 07:40 to 2026-11-14 17:09
   1  2026-11-13 07:40  帮我看看录制服务的日志
   2  2026-11-13 07:52  继续
   5  2026-11-13 08:01  为什么录制停了，healthz 报错
   …
   8  2026-11-13 09:40  /model opus
   9  2026-11-13 09:41  换 Opus 继续查：为什么第 12 轮的健康检查连不上摄像头网关
   …
  13  2026-11-13 19:18  /cd ~/Code/fieldcam-web
  14  2026-11-13 19:20  把面板上的「录制中」状态改成读新的健康检查接口
   …
```

Turn 8 is `/model opus` and turn 13 `/cd`: both rebuilds came from the user's own commands, 10
and 8 minutes after the call before them, while the cache was still there. The idle rebuilds
wrote one-hour entries only (`1h 100.0%`) and came after gaps of 2 to 11 hours.

The report:

```
最近 7 天（11-10 14:09 起）：

**命中率 97.9%**：输入 14.2 亿 token，读缓存 97.9%、写缓存 2.1%（其中 93% 是 1 小时缓存），几乎没有未缓存的输入。
**重建 96 次，写入 980 万 token，$61.92**
- 空闲超过缓存时长 58 次 · $41.80：间隔中位数约 1 小时 42 分，缓存时长 1 小时。
- compact 之后 33 次 · $16.40：compact 本来就会重建对话缓存，重建的只是变短后的历史。
- 换模型 2 次 · $1.62，system 或工具变了 2 次 · $1.30，请求设置变了 1 次 · $0.80。

**最值得看的会话**（看过 3c7e91a2，其余只看了总览）
- `3c7e91a2` fieldcam「录制任务停止排查」· 命中 95.1% · 重建 7 次 · $5.34：第 8 回合 `/model opus` 从 Sonnet 5.5 换到 Opus 5.5，第 9 回合重建（$1.12，距上一次调用 10 分钟，缓存本来还在）；第 13 回合 `/cd` 到另一个目录后 system 变了，第 14 回合重建（$0.99，距上一次调用 8 分钟）；另有 3 次空闲（间隔 2–11 小时）、2 次 compact。

**本可以避免的**
1. 3c7e91a2 里换模型和换目录的两次（$2.11）：模型在会话开头定好；换目录做别的事时开新会话。依据：Claude Code 文档 [Switching models](https://code.claude.com/docs/en/prompt-caching#switching-models)、[Enabling or disabling a plugin](https://code.claude.com/docs/en/prompt-caching#enabling-or-disabling-a-plugin)（`/cd` 会应用新目录的插件）。
2. 空闲重建都在离开超过 1 小时之后，1 小时已是最长的缓存时长，设置帮不上；如果离开前本来就打算 compact，趁缓存还在时做更便宜（[Compacting the conversation](https://code.claude.com/docs/en/prompt-caching#compacting-the-conversation)）。
3. compact 之后的重建是正常代价，不用改。

另外 1 次换模型、1 次 system 或工具变了、1 次请求设置变了在别的会话里，没有逐个查间隔；如果它们发生在离开 1 小时以上之后，缓存本来就过期了，不算额外的花费。需要的话我再看。

金额是这些重建写缓存的花费，不是「全部命中能省下的钱」。
```

## Pitfalls

- **The range filters calls, the session view does not**: `asm cache ID` covers the whole
  session and rejects range, project, model, group and ranking options (exit 2).
- **`TTL 300000ms` on a full miss may be an assumption**, see idle above.
- **Subagents count in tokens, not in rebuilds**: their calls are not judged, and their own
  short caches can pull a session's hit rate down.
- **The current session is left out** of the overview; add `--include-self` when the user asks
  about this very session.
- **Old sessions** (Claude Code before 2.1.300) have no request records; `unknown` there is not a
  finding.
- **No savings estimate**: report what rebuilds cost, not what perfect caching would have cost.
- **Claude Code shows its own view**: `/usage` has a `Prompt cache (main)` line with the session's
  hit ratio and, on 2.1.260 or later, the likely cause of the last miss
  ([Check cache performance](https://code.claude.com/docs/en/prompt-caching#check-cache-performance)).
  It covers the running session only; `asm cache` covers past sessions.
