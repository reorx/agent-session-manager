---
created: 2026-11-03
updated: 2026-11-03
tags:
  - context-window
  - model-catalog
  - claude-code
  - pricing
  - agent-session-manager
  - plan
---

# Context 占用改为优先按 Claude Code 缓存的模型目录取窗口大小

## PRD

### 背景

表格的 Ctx 列和详情 Metadata 的 Context 行显示 session 结束时 context 用掉了模型窗口的多少，算法是「最后一次主线调用的输入 token ÷ 该模型的窗口大小」。窗口大小目前来自应用里写死的一张表，和价格表放在一起，2026-10-23 按官方页面整理。这有两个问题：

- Anthropic 发布新模型后，表里没有它，用了新模型的 session 在 Ctx 列显示 `—`，要等应用发新版补表才有数。
- 一个模型在 Claude Code 里用多大的窗口，由 Claude Code 从服务器取回的模型目录决定，还会受账号、beta 和环境变量影响。写死的表看不到这些信息。

Claude Code 会把这份模型目录缓存在本机的数据目录里，每个模型都写明了最大输入 token 数。应用直接读这份缓存，Claude Code 支持新模型时这里也就跟着有了，不需要应用发新版。

### 目标

Ctx 的百分比优先按 Claude Code 缓存的模型目录里的窗口大小计算。读不到目录时退回现有的内置表，用户看到的结果与现在一致。

### 功能与交互

- **窗口大小的来源与顺序**：先查 Claude Code 缓存的模型目录；目录里没有这个模型，再查内置表；两边都没有时 Ctx 显示 `—`，详情的 Context 行只写 token 数和峰值、不写窗口，与现在一致。
- **作用范围**：表格的 Ctx 列（包括按 Ctx 排序）、详情 Metadata 的 Context 行（`最终占用 / 窗口 · 峰值`）、命令行费用列表的 CTX 列，三处用同一个来源，同一个 session 的数字一致。
- **用户看到的变化很小**：
  - 正常情况下 Ctx 的数字不变：本机用过的模型，目录和内置表给的窗口一样。
  - 用了内置表里没有的新模型的 session，不再显示 `—`，改为显示百分比。
  - 目录和内置表对同一个模型给出不同窗口时，按目录的值显示。
  - 详情的 Context 行不标注窗口来自哪里。
- **何时生效**：升级后不需要重建索引。应用启动时读一次目录。Claude Code 更新目录以后，要等应用下次启动或下次扫描（如 Index 菜单的 Scan for Changes）才生效，这期间没有任何 session 变化也一样。应用开着、什么都不做的时候，不会自己发现目录变了。
- **出错与边界**：以下情况都静默退回内置表，不报错、不弹提示，界面上也没有"目录不可用"之类的状态：
  - 目录不存在：没装 Claude Code、从没登录过，或数据目录指向别处
  - 目录文件损坏，或字段缺失、取值不合理
  - 模型不在目录里

  同一个数据目录下留有多个账号的目录时都会读，同一个模型以最近一次获取的为准。
- **只读本机文件**：只读取 Claude Code 已经缓存在本机的文件，不联网，也不修改这些文件。

### 不做的事

- 不读 Claude Code 全局配置里的功能开关（GrowthBook），不判断 `CLAUDE_CODE_DISABLE_1M_CONTEXT` 等环境变量。Ctx 用的是目录上写的窗口，可能和 Claude Code 当时实际生效的窗口不一样。
- 不联网拉取目录；Claude Code 没有缓存时就用内置表。
- 不改价格表。目录里没有价格信息，费用仍按内置价格表计算。
- 窗口超限（峰值大于窗口、Ctx 超过 100%）时不加新的界面提示。
- 界面上不显示窗口来源，也不加"目录不可用"的提示。
- 应用运行期间不监听目录文件的变化。

### 验收标准

- 目录里有的模型，Ctx 和详情 Context 行用的是目录里的窗口。
- 目录里没有、内置表里有的模型，用内置表的窗口；两边都没有的模型显示 `—`。
- 内置表里没有、目录里有的新模型，Ctx 显示百分比，不再是 `—`。
- 目录和内置表对同一个模型给出不同窗口时，按目录的值显示。
- 目录不存在、文件损坏或缺字段时，所有 session 的 Ctx 与改动前完全一致，没有报错或提示。
- 同一个 session 在表格、详情和命令行费用列表里的 Ctx 一致。
- 升级后不需要重建索引。Claude Code 更新目录后，重启应用或执行一次 Scan for Changes，Ctx 就按新目录显示。
- 在本机真实数据上对比改动前后，Ctx 有差异的 session 数在可解释的范围内：每一处差异都对应到某个目录与内置表不一致的模型。按 2026-11-03 的数据，预期是 0 个。

## Tech Design

### 现状

- **窗口表**：`crates/core/src/pricing.rs` 的 `ModelPrice.window`，`context_window(model)` 就是 `price(model).map(|p| p.window)`。`normalize()` 去掉结尾的 `[1m]` 和 `-YYYYMMDD`，`price()` 用它匹配。表里 `claude-opus-4-5`、`claude-opus-4-1` / `claude-opus-4`、`claude-sonnet-4-5` / `claude-sonnet-4`、`claude-haiku-4-5`、`claude-3-5-haiku` 的窗口是 200K，其余是 1M。
- **算占比**：`crates/core/src/model.rs` 的 `SessionSummary::context_ratio()` 是 `context_tokens / context_window(context_model)`。
- **索引不存窗口**：`crates/core/src/index/schema.sql` 的 `sessions` 表只存 `context_tokens`、`context_peak`、`context_model`，不存窗口或比例。所以 PRD 说的"不需要重建索引"成立：改窗口来源不用提升 `crates/core/src/index/db.rs` 的 `SCHEMA_VERSION`（当前是 3）。
- **调用点**：
  - core `crates/core/src/bin/asm.rs`：`context_cell()`（`asm cost` 的 CTX 列）和 `App::cost` 里 `CostSort::Context` 的排序键
  - app `crates/app/src/model.rs`：`context_detail()` 直接调 `context_window`；`SessionRow::from_summary` 调 `context_ratio()`，得到 `context_pct` / `context_fmt` / `context_detail`
  - `table.rs` 按 `context_pct` 排序，`detail.rs` 显示 `context_fmt` / `context_detail`，这两个文件不用改
- **app 里的缓存与重算时机**：
  - `context_pct` 等字段是 `SessionRow` 构造时算好的，只在 `crates/app/src/main.rs::on_store_events` 收到 `StoreEvent::Loaded`（整表）或 `StoreEvent::Session`（单行）时重建，此外不会重算。
  - `crates/app/src/store.rs` 的 store 线程：`run()` 先 `Worker::load()` 再 `Worker::scan(false)`；`scan()` 只在 `changed`（`files_seen != files_unchanged`、`files_missing > 0` 或 `history_sessions > 0`）时补发第二个 `Loaded`。
  - 扫描只在四种情况下发生：启动、Index 菜单的 `Rescan`（Scan for Changes）/ `RebuildIndex`、收藏与取消收藏、更换收藏目录。没有定时器，也不监听文件。
  - 结论：照现在的代码，记录文件没有变化的 Scan for Changes 不会重建行。要让 PRD 里"目录变了，下次扫描就生效"成立，需要本方案新增的"目录变了也重读"。
- **数据目录**：`crates/core/src/claude/paths.rs` 的 `ClaudeDir`，依次取 `ASM_CLAUDE_DIR`、`CLAUDE_CONFIG_DIR`、`~/.claude`。`asm` 另有 `--claude-dir`。
- **本机数据**（2026-11-03）：`context_model` 只有 8 种，分别是 opus-5-5（972 个 session）、opus-5（306）、fable-5-1（228）、sonnet-5（31）、sonnet-5-5（18）、fable-5（12）、haiku-5-5（10）、opus-4-8（4）。这 8 种在目录和内置表里都是 1M。`api_usage` 里所有模型名都是小写，都不带 `[1m]`。

### 目录文件实测（2026-11-03，Claude Code 2.1.311）

依据是本机缓存文件，以及 Claude Code 二进制里的字符串。凡是"来自二进制"的条目都只看了代码，没有实测。

**位置**：`<数据目录>/cache/model-catalog/`。Claude Code 的代码里写的是 `join(配置目录, "cache", "model-catalog")`，所以它跟着 `CLAUDE_CONFIG_DIR` 走，和本应用的 `ClaudeDir` 是同一个根。

| 文件 | 内容 |
|---|---|
| `<orgUuid>-<accountScope>-cc.json` | 按账号下发的目录（下文叫 served）。本机是 `b3f7e813-…-a9566faf701d-cc.json`，前半段等于 `~/.claude.json` 里的 `oauthAccount.organizationUuid`。另外几种登录方式的文件名来自二进制：`tok-<scope>-cc.json`、`key-<scope>-cc.json`、`prof-<scope>-cc.json`、`fed-<scope>-cc.json`；结尾的 `cc` 是 surface 名 |
| `published-<32 位十六进制>.json` | 签名的公开目录（下文叫 published），289KB；`<key>` 区分来源（hosted / custom_url / file） |
| `published-floor.json` | 防回滚的版本下限 `{version, sources: {<key>: {version, issuedAt, recordedAt}}}`，没有模型信息 |
| `masked-ids.json`、`masked-ids.unparseable.json`、`*.json.tmp.*` | 来自二进制：屏蔽的 id 列表、写入临时文件。本机没有，也不含窗口信息 |

**served 的结构**：

```text
{ version: 2, fetchedAt: <ms>, staleAt: <ms>,
  catalog: { surface: "cc", state: {…},
             config: { id: "cc", models: [ <model> … ], provider_alias_targets: {…}, settings_vocabulary: {…} } } }
```

**published 的结构**：

```text
{ version: 2, fetchedAt, staleAt, etag, source: "hosted", rootId,
  sidecar: { algorithm: "RSASSA-PKCS1-v1_5-SHA512", schema: 1, publicKeySha256, signature },
  documentBytes: "<base64>",
  document: { $schema, schema_version: 1, version: 3729, issued_at, expires_at, key_id,
              surfaces: { cc | ccd | ccr | chat | cowork:
                          { model_selector_state: [ … ], model_selector_config: [ { id, models: [ <model> … ], … } ] } } } }
```

`document` 与 `documentBytes` 解码后的内容逐字段相同（已核对）。`expires_at` 是 `issued_at` 之后 7 天。

**单个模型条目** `<model>`：`{id, name, short_name, section: "main"|"overflow", capabilities, thinking, fast_mode?, runtime: {max_input_tokens, max_output_tokens, effort_levels, default_effort, capabilities, family, image_limits}, min_claude_code_version?, offered_on?}`，其中 `offered_on` 只出现在 published 里。chat / cowork 两个 surface 的条目没有 `runtime.max_input_tokens`。

**本机的值**：

- served 有 12 个模型：`claude-haiku-4-5-20251001` 是 200K，其余 11 个（opus-5-5、fable-5-1、sonnet-5-5、haiku-5-5、sonnet-5、opus-5、fable-5、opus-4-8、opus-4-7、opus-4-6、sonnet-4-6）都是 1M。
- published 的 cc surface 多一个 `claude-opus-4-1-20250805`（`offered_on: ["bedrock"]`，200K）。published 的 cc、ccd、ccr 三个 surface 对同一模型给的窗口都相同。
- id 里没有 `[1m]` 变体，带日期后缀的有两个（haiku-4-5、opus-4-1）。别名（opus / sonnet / haiku / fable → 各 provider 的默认 id）只出现在 `provider_alias_targets` 里，不在 `models[]` 里。

**时间字段**：

- `fetchedAt` / `staleAt` 都是 Unix 毫秒。`staleAt` 比 `fetchedAt` 晚约 1 小时（本机两份文件分别晚 62、63 分钟；二进制里的写法是 1 小时 ±10%），表示该重新获取了，不是失效时间。
- Claude Code 运行时 served 文件会被频繁重写：本机 served 文件的获取时间是 11-03 13:19，published 是 11-02 11:10。

**Claude Code 自己怎么取窗口**（来自二进制）：

1. 先在当前使用的目录里按 id 匹配：两边都转成小写、去掉 `[1m]` / `[2m]` 后比较；匹配不上，再用一个内部的 identity spelling 函数换一种写法再比。
2. 窗口取 `runtime.max_input_tokens`，没有就取条目上的 `context_window`。只接受 100,000 到 10,000,000 之间的整数。
3. 上一步只是"声明值"。最终使用的窗口还要和 Claude Code 自带的各模型上限取较小值，并受 `CLAUDE_CODE_DISABLE_1M_CONTEXT`、`CLAUDE_CODE_MAX_CONTEXT_TOKENS`（配合 `DISABLE_COMPACT`）、针对 sonnet-4-6 的 GrowthBook 开关、`autoCompactWindow` 等因素调整。
4. 二进制里还内嵌了一份种子目录（version 3134，2026-10-26），本机没有缓存时用它。
5. Claude Code 会按清理周期删除这个目录下的旧文件（二进制里有这个清理函数，没有实测）。

### 方案

core 新增一个模块，读取 `<数据目录>/cache/model-catalog/` 下的 served 和 published 文件，合并成 `ModelCatalog`：一张"归一化模型 id → 窗口"的表。`pricing::context_window` 改为接收 catalog 参数：先查 catalog，查不到再查内置表（原来的函数改名为 `builtin_window`）。`SessionSummary::context_ratio` 也接收 catalog。

app 这边：store 线程启动时读一次 catalog，随 `Loaded` 事件发给主线程；每次扫描后重读，窗口有变化就补发 `Loaded`。主线程保存最新的 catalog，构造 `SessionRow` 时传进去。

`asm cost` 每次运行读一次 catalog。新增的 `--windows` 选项用来对比目录与内置表，列出 Ctx 会变化的 session，作为真实数据上的验收工具。

关键决策：

1. **catalog 作为显式参数传递，不用进程级全局变量。**
   - 被否决的备选一：在 core 里放一个全局 `RwLock`，由 app 和 asm 启动时写入。这样函数签名都不用改，但同一个测试二进制里的测试共享这个全局状态：装了 catalog 的测试会让假定内置表的测试时过时不过；行为也会依赖初始化顺序。
   - 被否决的备选二：把窗口存进索引。目录一变就得重建索引，和 PRD 冲突。
   - 显式传参的代价：`SessionRow::from_summary` 多一个参数，app 测试里约 20 处调用要跟着改。这是机械修改。
2. **什么时候读目录：store 线程每次扫描后重读一次，窗口有变化就补发 `Loaded`。**
   - 读目录在 `Worker::open` 时做一次，每次 `scan()` 结束后再做一次。重读结果只比较窗口表，不比较文件元数据：Claude Code 大约每小时重写一次 served 文件，`fetchedAt` 每次都会变，比较元数据会导致无意义的整表重读。
   - 被否决：只在启动时读。这样 Scan for Changes 无法让新目录生效，必须重启。
   - 被否决：每次查窗口都读文件。一次 `Loaded` 就有几千次查询。
   - 被否决：监听文件变化。要多一个依赖和一条监听线程；而 served 文件每小时重写，绝大多数改动和窗口无关。为了很少发生的窗口变化，代价不划算。
   - 读一次的开销：一个目录、两个文件，共约 300KB JSON，预计几毫秒，在 store 线程上做，不占主线程。验收时实测。
3. **读哪些文件，怎么合并：** 读所有 `*-cc.json`（served）和除 `published-floor.json` 之外的所有 `published-*.json`。
   - 优先级：served 在前，按 `fetchedAt` 从新到旧；published 在后，也按 `fetchedAt` 从新到旧。同一模型取第一个出现的值。
   - 为什么全读、不挑文件：session 记录里没有账号信息，同一个数据目录下可能留有多个账号的文件。窗口也不因账号而不同：served 只是从 published 里按账号筛出来的子集，本机重合的部分数值完全相同。
   - 为什么 served 在前：第一方登录时 Claude Code 实际用的是 served。published 补上被筛掉、但 session 可能用过的模型，例如只在 Bedrock 上提供的 opus-4-1。
   - 被否决：按 `~/.claude.json` 的 `oauthAccount.organizationUuid` 选文件。要多读一个 Claude Code 文件，token / API key 方式登录时对不上，而且多账号的 session 本来就混在一起显示。
   - 被否决：只读最新的一个文件。会丢掉只出现在旧文件里的模型。
   - 只看 `cc` 这个 surface：本应用索引的是 Claude Code 的 session。本机 ccd / ccr 的值与 cc 相同；chat / cowork 的条目没有窗口。
4. **忽略 `staleAt` 和 `expires_at`，`fetchedAt` 只用来排序。** `staleAt` 是约 1 小时后重新获取的时间点，不表示数据失效；模型窗口很少变化，过期的目录也比 2026-10-23 写死的内置表新。被否决：丢弃过期文件。Claude Code 没在运行时文件很快就会过期，结果是来源在目录和内置表之间来回切换，Ctx 跟着跳动。
5. **解析方式：** 用带类型的 serde 结构体，字段全是 `Option`，未知字段默认忽略，不使用 `RawValue`、`tag`、`untagged`、`flatten`（遵守 AGENTS.md 的 core 规则）。
   - 窗口：`runtime.max_input_tokens` 按 `f64` 读，再校验必须是 100,000–10,000,000 之间的整数，与 Claude Code 一致；没有时用条目上的 `context_window`。
   - 坏数据的处理：整份文件解析失败就整份跳过；单个条目缺 `id` 或缺有效窗口，只跳过这一条。
   - published 读已经解析好的 `document` 字段，不解码 `documentBytes`（两者相同，core 也没有 base64 依赖），不验签名：Claude Code 写入前已经验过，我们只读不写。
6. **模型名归一化：** 目录 id 先 `trim` 再转小写，然后过 `pricing::normalize`（去掉 `[1m]` 和日期后缀）作为键；查询时用 `normalize(model)` 当键。这样 `claude-haiku-4-5-20251001` 和 `claude-haiku-4-5` 都能查到同一条。同一份文件里两个 id 归一化后相同时，取与键完全相同（不带日期）的那个，否则取先出现的。查询时不转小写：JSONL 里的模型名实测全是小写，省掉每次查询的分配。`normalize` 不加 `[2m]`：JSONL 里从没出现过，而且这个函数计价也在用。
7. **错误在哪一层吞掉：** 低层函数（读文件、解析 served / published）都返回 `Result`。`ModelCatalog::load` 是唯一吞掉错误的地方：目录读不了就返回空 catalog；单个文件出错就记进 `CatalogFile.error`，然后跳过。app 和 asm 需要完全一样的退回行为，放在一处比两个调用方各写一遍可靠。界面不显示这些错误；app 往 stdout 打一行日志，`asm cost --windows` 列出被跳过的文件和原因。
8. **验收工具用 `asm cost --windows`。** 它在同一次运行里分别按"空 catalog"和"真实 catalog"算出每个 session 的 CTX 单元格，统计不同的有多少个。被否决：单独做一个子命令，那样要重写一遍项目和日期过滤。被否决：加一个关闭目录的环境变量，然后跑两次再 diff，不如在一次输出里直接对比方便。

### 数据变动

- 索引 schema、`SCHEMA_VERSION`、`state.json`、`config.json` 都不变，不需要迁移，也不需要重建索引。
- 不新增环境变量。目录位置由 `ClaudeDir` 决定，测试和验收通过 `ASM_CLAUDE_DIR` / `--claude-dir` 指定。
- 新增内存中的类型 `ModelCatalog`，不落盘。
- `StoreEvent::Loaded` 增加字段 `catalog: Arc<ModelCatalog>`。
- 新增测试数据目录 `crates/core/tests/fixtures/model-catalog/`，由 `gen_fixtures.py` 生成。

### 逻辑变动

**core**

- `claude/paths.rs`：新增 `ClaudeDir::model_catalog_dir()`，返回 `root/cache/model-catalog`。
- `claude/model_catalog.rs`（新文件，并在 `claude/mod.rs` 里加 `pub mod model_catalog;`）：
  - 公开类型：
    - `ModelCatalog { windows: BTreeMap<String, u64>, files: Vec<CatalogFile> }`，derive `Clone, Debug, Default`
    - `CatalogFile { path, kind: CatalogKind, fetched_at: Option<i64>, models: usize, error: Option<String> }`
    - `CatalogKind { Served, Published }`
  - 公开方法：
    - `ModelCatalog::load(&ClaudeDir) -> ModelCatalog`：按决策 3 的顺序合并文件
    - `window(&self, model) -> Option<u64>`：只查目录，不回落内置表
    - `windows()`、`files()`：只读访问
    - `from_windows(impl IntoIterator<Item = (&str, u64)>)`：给测试用，与 `load` 走同一套归一化和校验逻辑
  - 文件分类（私有）：
    - 文件名以 `.json` 结尾才处理
    - `published-floor.json` 跳过
    - `published-` 开头的按 published 解析
    - 以 `-cc.json` 结尾的按 served 解析
    - 其余文件（`masked-ids*.json`、其他 surface、临时文件）直接忽略，不算错误
  - 解析（私有，返回 `anyhow::Result`，同时保留 JSON 错误和缺结构字段的说明）：
    - served 读 `catalog.config.models`
    - published 读 `document.surfaces.cc.model_selector_config[*].models`
    - 两种都顺带读 `fetchedAt`
    - 单个条目的取值规则见决策 5
- `pricing.rs`：
  - 原来的 `context_window(model)` 改名为 `builtin_window(model)`
  - 新增 `context_window(catalog: &ModelCatalog, model: &str) -> Option<u64>`，实现为 `catalog.window(model).or_else(|| builtin_window(model))`。窗口来源的优先顺序只在这里定义一次
  - 模块注释补一句窗口来源的顺序
- `model.rs`：`SessionSummary::context_ratio(&self, catalog: &ModelCatalog)`。
- `bin/asm.rs`：
  - `context_cell(s, catalog)`
  - `App::cost` 开头 `ModelCatalog::load(&self.claude)` 读一次，排序键和输出都用它
  - `CostArgs` 新增 `--windows`，与 `--compare` 互斥（clap 的 `conflicts_with`）
  - 新增 `compare_windows(&mut self, sessions, catalog, limit)`，输出格式大致如下：

    ```text
    model catalog: ~/.claude/cache/model-catalog
      served     b3f7e813-…-cc.json          fetched 2026-11-03 13:19  12 models
      published  published-dd94e277….json    fetched 2026-11-02 11:10  13 models
      skipped    broken-cc.json              <错误原因>
    MODEL                       CATALOG  BUILT-IN  SESSIONS  CHANGED
    claude-opus-5-5                  1M        1M       972        0
    …
    sessions with context data: 1581 · Ctx changed: 0
    ```

    - 模型行取"目录里的模型"和"范围内 session 的 `context_model`"的并集，按 SESSIONS 降序排。
    - CHANGED 统计用空 catalog 算出的 `context_cell` 与用真实 catalog 算出的不同的 session 个数，比较的是格式化后的单元格，即用户实际看到的数字。
    - 之后按更新时间列出前 `limit` 个变化的 session（短 id、模型、变化前 → 变化后）。
    - 路径、时间和数字的格式复用 `format.rs` 的 `fmt_datetime` / `fmt_tokens`。

**app**

- `model.rs`：
  - `SessionRow::from_summary(s, catalog: &ModelCatalog)`，`context_detail(s, catalog)` 改用新的 `context_window(catalog, …)`
  - 测试里的调用点（`model.rs`、`table.rs`、`actions.rs` 的测试模块，以及 `main.rs`）都要跟着改。测试模块里可以加一个只在测试中编译的辅助函数，用空 catalog 构造行，免得每处调用都写一长串
- `store.rs`：
  - `StoreEvent::Loaded { sessions, catalog, elapsed }`
  - `Worker` 增加 `catalog: Arc<ModelCatalog>`：`Worker::open` 时读一次，`load()` 把它带进事件
  - `scan()` 发完 `ScanDone` 后重读 catalog，窗口表和当前持有的比较（`windows()` 相等即视为没变）。如果变了，先替换持有的 catalog，再按 `if changed || catalog_changed { self.load()? }` 决定是否补发 `Loaded`。Rebuild 走的也是 `scan()`，不需要另外处理
  - 首次读到 catalog 和之后每次变化时，往 stdout 打一行 `model catalog: N models from M files (K skipped) in X ms`，和现有的 `scan: …` 日志一样
- `main.rs`：
  - `AppRoot` 增加 `catalog: Arc<ModelCatalog>`，初始值为空 catalog
  - `on_store_events` 收到 `Loaded` 时先替换 `self.catalog`，再构造行；`Session` 事件用当前的 `self.catalog`。同一批事件按顺序处理，所以 `Loaded` 之后到达的 `Session` 用的就是新 catalog
  - 只因目录变化而补发的 `Loaded`，照现有逻辑也会让正在进行的搜索重跑一次（`reloaded && self.searching()`）。这种情况很少见，重跑也不会出错，所以不另加判断
- `table.rs`、`detail.rs` 不改。

### 实施步骤

- [x] 0. 留基线（必须在改代码之前做）：
  - 用当前 HEAD 构建 release 版 `asm`，复制到 `tmp/2026-11-03-context-window-catalog/asm-before`
  - 用 `sqlite3 <真实索引> ".backup tmp/2026-11-03-context-window-catalog/index.sqlite"` 备份索引
  - 跑 `asm-before --db <备份> cost --sort context -n 100000 --no-scan > before.txt`
  - `make dev` 启动 3 次，记下日志里 `sessions on screen …ms` 的数字，记完 kill 掉 app
- [x] 1. 测试数据：在 `gen_fixtures.py` 里加一个函数，生成 `crates/core/tests/fixtures/model-catalog/`。`uv run` 重新生成后连同输出一起提交。内容：
  - 一份 served 文件（文件名用假 uuid）：结构与实测一致，只保留 3–4 个真实模型和少量无关字段（`thinking`、`capabilities` 等，用来证明它们被忽略），另加一个内置表里没有的假想新模型 `claude-opus-6`（1M）
  - 一份 published 文件：`documentBytes` 用占位字符串；cc surface 含 `claude-opus-4-1-20250805`（200K）；chat surface 放一个带 `runtime.max_input_tokens` 的 `claude-opus-4-5-20251101`（500K），用来证明只读 cc
  - 一份 `published-floor.json`
  - 不提交用户的真实文件，也不放真实的 uuid、签名
- [x] 2. 先写 core 的行为测试：
  - 新建 `crates/core/tests/model_catalog.rs`
  - 把 `crates/core/tests/pricing.rs` 里窗口相关的两个测试改成 `builtin_window`，再加上 `context_window(&ModelCatalog::default(), …)` 的对照
  - 此时测试应当编译失败或断言失败
- [x] 3. 实现 core：`paths.rs`、`model_catalog.rs`、`pricing.rs`、`model.rs::context_ratio`。验证：`cargo test -p agent-session-core` 全部通过。
- [x] 4. asm：
  - 先在 `crates/core/tests/cli.rs` 加测试（见下面"测试与验证"）
  - 再实现 `context_cell`、`App::cost` 和 `--windows`
  - 验证：core 测试全部通过，`make asm ARGS="cost --windows"` 在本机能跑
- [x] 5. app 行模型：先在 `crates/app/src/model.rs` 的测试里加目录相关的断言，再实现 `from_summary(s, catalog)`，并修改所有调用点。验证：`cargo test -p agent-session-manager` 编译通过，model 测试通过。
- [x] 6. app 数据流：先在 `crates/app/src/store.rs` 的测试里加两个用例，再实现 `Worker` 和 `on_store_events` 的改动。验证：`make test` 全部通过，包括键盘测试。
- [x] 7. 真实数据验收：
  - 跑 `make asm ARGS="--db <备份> cost --windows --no-scan"`，结果应为 `Ctx changed: 0`；如果不是 0，逐个模型解释差异
  - 用新的 `asm` 生成 `after.txt`（参数与 `before.txt` 相同），和 `before.txt` 做 diff，应当没有差异
  - `make dev`：确认日志里有 `model catalog: …` 一行，并且加载耗时在 5ms 以内；行上屏时间和步骤 0 的基线持平
  - `make screenshot`，用 Read 打开第 3 张图，确认 Ctx 列正常显示
  - 截图和 before / after 输出归档到 `tmp/2026-11-03-context-window-catalog/`；最后用 `pgrep -fl agent-session-manager` 确认 app 已经退出
- [x] 8. 文档，并一次性 commit：
  - `AGENTS.md`：
    - "当前状态"加一条：窗口来源与生效时机，链接本 plan
    - `make asm` 的命令列表改成 `cost [--compare|--windows]`
    - Documentation 索引加入本 plan
  - `kb/docs/claude-session-data-format.md`：
    - §1 数据源总览加一行 `cache/model-catalog/`
    - 新增一小节记录上面"目录文件实测"的要点
    - 第 102 行附近 `[1m]` 那条，把"按模型查表"改为"先查目录、再查内置表"
    - 更新 frontmatter 的 `updated`
  - `kb/docs/product-prd.md`：第 92 行 Ctx 的定义补上窗口来源；第 44 行状态表的"费用与 context"一行补一句
  - `kb/docs/app-architecture.md`：
    - 事件表里 `Loaded` 的内容加上 catalog
    - "启动与扫描"一节写明：目录窗口有变化时，没有文件改动的扫描也会补发 `Loaded`；修改"没有变化的扫描……也不触发最后的 `Loaded`"这句的适用范围
    - 更新 frontmatter 的 `updated`
  - `kb/plans/2026-10-23-cost-context-columns.md`：在"实测依据"的窗口那条、价格表的"窗口"列说明、逻辑变动里 `context_window(model)` 那句，各加一句"2026-11-03 起窗口优先取 Claude Code 模型目录，见 `2026-11-03-context-window-from-catalog.md`"
  - `kb/known-issues.md`：
    - 新增一条有意的简化：Ctx 用的是目录声明的窗口，不反映 `CLAUDE_CODE_DISABLE_1M_CONTEXT`、Claude Code 自带的上限等因素
    - "价格表写在代码里"那条注明窗口已不再只靠写死的表，价格仍然是写死的
  - `crates/core/src/pricing.rs` 的模块注释
  - 写 session 笔记 `kb/sessions/2026-11-03-<slug>.md`，勾选本 plan 的步骤

### 测试与验证

按 PRD 验收标准逐条列出，括号里是测试所在位置。core 测试把 fixture 复制到 `tempdir/cache/model-catalog/`，用 `ClaudeDir::new(tempdir)` 加载；边界情况在测试里用辅助函数直接写小 JSON 文件（`serde_json::json!`）。

1. **目录里有的模型用目录值**（`tests/model_catalog.rs`）：加载 fixture 后，`claude-opus-5-5` 是 1M；`claude-haiku-4-5-20251001` 和 `claude-haiku-4-5` 都是 200K；`claude-opus-5[1m]` 与 `claude-opus-5` 相同；只在 published 里的 `claude-opus-4-1-20250805` 是 200K；chat surface 的 `claude-opus-4-5-20251101` 不被读取，仍按内置表的 200K。
2. **目录里没有的用内置表，两边都没有的是 None**（同上）：`context_window(&catalog, "claude-sonnet-4-5")` 是 200K，`"claude-opus-9"` 和 `"<synthetic>"` 是 `None`。
3. **新模型不再显示 `—`**：
   - core：`claude-opus-6` 能查到 1M
   - app `model.rs`：`context_model` 为 `claude-opus-9`，配 `from_windows([("claude-opus-9", 1_000_000)])`，`context_fmt` 应为 `41%`，`context_detail` 应为 `412k / 1M · peak 655k`（原测试 `context_is_a_share_of_the_models_window` 用空 catalog 时得到 `—`，两者对照）
4. **冲突时以目录为准**：
   - core：写一份 served 文件，把 `claude-opus-5` 设为 500,000，结果应为 500K
   - app：`context_detail` 里出现 `/ 500k`
   - CLI（`tests/cli.rs`）：在 sandbox 的 `cache/model-catalog/` 写一份 served 文件，把 `claude-opus-5` 设为 100,000（允许的下限），`asm cost -p alpha --since 2026-09-01 --until 2026-09-03` 里 S1 的 CTX 按 100K 计算，不再是原来的 `1%`。具体数值由实现时算出后写进断言
5. **目录缺失或损坏时与改动前一致**（`tests/model_catalog.rs`）：
   - 没有这个目录时，`windows()` 为空，并且对内置表里的每个模型，`context_window(&catalog, m) == builtin_window(m)`
   - 截断的 JSON 文件被跳过，`files()` 里有它的 `error`，另一份文件照常生效
   - published 文件缺 `document` 时整份跳过
   - 条目缺 `runtime`、`max_input_tokens` 为 99,999 / 10,000,001 / 150000.5 时，这一条被忽略，回落内置表
   - 只有条目上的 `context_window`、没有 `runtime` 时，用 `context_window`
   - `published-floor.json`、`masked-ids.json`、`x-cc.json.tmp.1` 被忽略，也不记为错误
   - 已有的 CLI 测试不需要改：它们的 sandbox 里没有目录，输出保持原样，这一点本身就是验证
6. **多份文件的优先级**（`tests/model_catalog.rs`）：两份 served 给出不同值时取 `fetchedAt` 较新的；published 即使 `fetchedAt` 更新，也排在 served 后面；`fetchedAt` 缺失的排在最后。
7. **三处显示一致**：app 和 asm 都只通过 `pricing::context_window(catalog, …)` 取窗口（代码审查时确认）。CLI 加一个 `--windows` 测试：sandbox 里放入改了值的目录，输出里列出文件、模型行和 `Ctx changed: N`，再加一个 `--windows` 与 `--compare` 同时使用时报错的用例。
8. **不重建索引，扫描时生效**（`crates/app/src/store.rs` 与 `keyboard_tests.rs` 测试）：
   - 用一个只含 `cache/model-catalog/` 的临时数据目录启动 store，第一个 `Loaded` 带着这份 catalog
   - store 测试同步调用 `Worker::scan`，改写窗口后确认 `ScanDone` 紧跟带新 catalog 的 `Loaded`；只改获取时间再扫描，只有 `ScanDone`，没有 `Loaded`；删除目录后刷新到空 catalog
   - 无界面窗口测试实际发 `StoreRequest::Scan`，等待主线程采用新 catalog，确认表格、选中行 Metadata 和比例排序键同时更新；没有改记录文件
   - 现有的 `until_settled` 遇到"没有变化的 `ScanDone`"就返回，因此事件断言采用同步 worker 扫描后排空队列，避免依赖超时证明没有事件
   - 现有用例用的 `fixture_claude()` 里没有目录，行为不变
9. **真实数据前后对比**：实施步骤 7 的 `--windows` 输出和 `before.txt` / `after.txt` 的 diff，结果写进 session 笔记。

不需要跑 `make parity`：解析器、清理逻辑、`regex.rs`、`list` / `session` / `search` 的输出都没有改。

### 风险与待定

- **目录格式是 Claude Code 的内部缓存**，随时可能变化，本文依据的是 2.1.311。
  - 应对：解析尽量宽松（字段全部可选，未知字段忽略），不校验文件的 `version`（结构没变就照常读）；失败时退回内置表；`asm cost --windows` 能看到哪些文件被跳过、原因是什么。
  - 已知的风险点：哪天 published 文件只留 `documentBytes`、不再写 `document`，published 就会整份失效，只剩 served 和内置表。
- **文件命名规则来自二进制**（`-cc.json` 结尾、`published-` 开头），没有官方文档。改名后读不到任何文件，结果退回内置表，和现在一样。
- **目录给的是声明值，不一定是 Claude Code 当时实际用的窗口**：Claude Code 还会和自带上限取较小值，并受环境变量和 GrowthBook 影响，这些按 PRD 不做。本机现在两边一致，没有影响。将来某个模型的声明值小于实际用量时，Ctx 会超过 100%，比如显示 `150%`。PRD 不做超限提示，但实现时要确认 Ctx 列 44px 的宽度放得下 4 个字符。
- **`staleAt` 过期照样使用**（决策 4）。如果 Claude Code 很久没运行，目录可能落后于新模型，这时新模型仍按内置表算，或者显示 `—`，和现在一样。
- **多账号和多个数据目录**：
  - 同一数据目录下多个账号的文件合并读取（决策 3），窗口不随账号变化的前提只在本机核对过。
  - 应用只读当前 `ClaudeDir` 下的目录，和显示的 session 来源一致。
  - 从 Finder 启动的 `.app` 读不到 `CLAUDE_CONFIG_DIR`（已登记在 known-issues），会读 `~/.claude` 下的目录，和它显示的 session 也是一致的。
- **"下次刷新"的含义**：指重启或下一次扫描。应用开着不扫描时不会更新。扫描过程中流式到达的单行，用的还是上一次 `Loaded` 时的 catalog，要到扫描结束后补发的 `Loaded` 才统一换成新的。
- **加载耗时**没有实测，按文件大小估计是几毫秒，在实施步骤 7 量。超过 5ms 时，改为比较文件的 mtime，没变就不重新解析。
- **执行范围**：本计划包含新增 `asm cost --windows`，以及读取 published 和所有账号留下的 served 文件；按整份计划实施。

### 完成与验收记录

2026-11-03 完成，见 [session 记录](../sessions/2026-11-03-context-window-catalog.md)。真实索引副本 1581 个有 context 数据的 session，`Ctx changed: 0`，新旧 CLI 输出无差异。目录加载 0.24–1.10ms；真实界面与合成 `150%` 样本截图已检查，证据归档于 `tmp/2026-11-03-context-window-catalog/`。
