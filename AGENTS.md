# Agent Session Manager

macOS 桌面应用（Rust + GPUI / gpui-kit），以表格列出本机所有 Claude Code session：过滤、表头排序、全文搜索、详情面板、费用分析。核心诉求是界面响应与查询速度。

## 当前状态

M0–M4 和 1.0 发布已完成。下一个里程碑是 M5（主要剩实时监听、subagent 查看、设置页；打包和图标已随 1.0 做完），定义在 `kb/plans/2026-09-08-agent-session-manager-prd.md` §3（开发任务的总入口，需求 R1–R13 也在里面；§2 架构未随实现更新）。各功能的实现状态在 `kb/docs/product-prd.md` §2，未解决的问题在 `kb/known-issues.md`。

## 结构

- `crates/core`（`agent-session-core`，无 gpui 依赖）：JSONL 解析（`claude/`）、增量 SQLite 索引（`index/`）、FTS5 / 正则搜索（`search/`）、费用（`pricing.rs`、`cost.rs`）、收藏、`asm` CLI 与 `transcript-reader`（`bin/`）
- `crates/app`（`agent-session-manager`）：gpui-kit 界面，保持薄。界面向三条后台线程发请求，线程回事件：
  - `asm-store`（`store.rs`）独占 `Db`：启动先读索引出表，再流式增量扫描
  - `asm-search`（`search.rs`）自带只读连接：文本模式走 FTS，正则模式流式扫描 transcript
  - `asm-reader`（也在 `store.rs`）不碰索引：详情的 Conversation 直接读记录文件（core `claude/conversation.rs::read_conversation`），在本线程用 app `conversation.rs` 转成卡片和行表，按 `seq` 取代旧请求；搜索词变了才重读

各功能要先知道的事（对应文档在文末 Documentation）：

- 详情面板：`detail.rs` 用 GPUI 的 `list` + `ListState` 按行表渲染回合卡（`turn_cards.rs`），一行是卡片的一段
- 会话操作（复制 resume 命令 / ID、Finder、导出 TXT、收藏等）都在 `actions.rs`，可用性由纯函数按行的事实算出；标题行按钮、右键菜单、`⌘C` / `⇧⌘C` 派发同一组 action
- History 行读 `history.jsonl` 显示 prompt 卡片，不参与搜索（与 Python 脚本一致）
- 费用由 core 按 `message.usage` × `pricing.rs` 价格表自算，按 `message.id` 去重，subagent 计入父 session；Claude Code 记录的 `cost-state` 只作对照
- 成本分析：主模型是花费最高的模型。Cost tab 和 `asm cost --model … --as …` 把主模型的 token 按另一个模型重新计价，其他模型原价。计价在 core `pricing.rs::price_tokens` / `cost.rs::compare`，app 用 `SessionRow.model_usage` 现算，不走后台请求
- Context 窗口优先读 Claude 数据目录的 `cache/model-catalog/`，缺失时回退内置表；启动与扫描后重读，窗口变了刷新整表，不用重建索引
- 缓存失效：读 Conversation 时 core `conversation.rs::cache_check` 逐次比较主线调用，比上一次少读缓存 ≥ 10k token 记为一次失效
- 收藏的快照存到索引旁的 `favorites/claude/`；取消收藏是移到废纸篓
- 导出 TXT：CLI（`transcript-reader` / `asm transcript`）和界面共用 `transcript::read_transcript`，输出是 XML 风格标签分隔的精简文本，不要求是标准 XML
- 发布：打 `v*` tag 触发 GitHub Actions（构建、签名、公证、本仓库 Release、`reorx/homebrew-tap` 的 cask）。应用内更新用 Sparkle 2，`updater.rs` 运行时加载 bundle 里的 framework，debug 二进制没有更新器

## 开发命令

`make help` 列出全部命令。要点：

- `make dev`：编译并启动 app（`run` 会先 pkill 旧实例）。手动起的 app 验证后必须 kill，用 `pgrep -fl agent-session-manager` 检查
- 测试：`cargo test -p agent-session-core`、`cargo test -p agent-session-manager`（纯逻辑，加上无界面窗口里的键盘测试）、`make test` 全跑
- `make asm ARGS="…"` 跑 `asm` CLI，`make transcript-reader ARGS="SESSION_ID -o session.txt"` 导出精简 TXT；子命令和参数看 `--help`。改价格表或用量解析后跑一次 `asm cost --compare`（自算费用与 `cost-state` 的偏差）
- `make parity`：在真实 `~/.claude` 上把 `asm` 与 Python 脚本（`~/Code/scripts/claude_session_search.py`，本项目的行为基准）逐字比对；改解析器、清理逻辑、`regex.rs` 或输出格式后必须跑。两边的列表顺序一致（开始时间降序，并列时按更新时间降序、再按 id），改排序要两边一起改（脚本的测试在它旁边的 `tests/test_claude_session_search.py`）。比对期间有 session 正在写入时 search 组偶尔失败，重跑即过
- 截图：`make screenshot`、`screenshot-search`、`screenshot-detail`、`screenshot-cost` 由 app 的 autoplay 自己走流程；`screenshot-clicks` 用 cliclick 真点击，坐标写在 `scripts/screenshot-clicks.sh`，改布局后要校对。都跑在真实索引上，输出到 `tmp/<date>-screenshots/`（`SHOT_DIR=` 可改）。各脚本截哪些图、怎么用 `ASM_AUTOPLAY_*` 指定 session 和查询，见 `kb/docs/gpui-kit-guide.md`「脚本化交互与截图」
- 量耗时：搜索用 `ASM_BENCH_DB=<索引副本> cargo test -p agent-session-manager real_index_timings -- --ignored --nocapture`（`ASM_BENCH_QUERIES`、`ASM_BENCH_REGEX` 可换查询）；详情打开速度看 app 日志的 `detail:` 行（read / build / first frame），量的时候要停在 Conversation tab，停在 Metadata 上首帧要等切回来才打点。不要用截图脚本日志里的耗时，刚唤醒屏幕后的数字可以偏高 4–8 倍
- 构建产物在 `~/Library/Caches/cargo-target`（全局 cargo 配置），不在 `./target`；`make bin` 打印二进制路径。首次构建约 1.5 分钟，增量约 2 秒
- 环境变量：`ASM_CLAUDE_DIR`（默认 `$CLAUDE_CONFIG_DIR` 或 `~/.claude`）、`ASM_DB`（默认 `~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite`）、`ASM_STATE`（仅 app，默认为索引同目录的 `state.json`）、`ASM_FAVORITES_DIR`（覆盖共享配置中的收藏目录）、`ASM_TRASH_DIR`（取消收藏改用此目录，测试必设）。验收时把 `ASM_DB` / `ASM_STATE` 指到 `tmp/`，可反复测冷启动而不动真实索引
- Rust 由 mise 管理（版本锁在 `mise.toml`）；新机器先 `make setup`（含 Metal Toolchain）
- 发布：`make release-local` 在本地走完整条链（产物在 `dist/release/`，不上传），`make test-release` 是发布脚本的行为测试。名字、最低系统、Sparkle 公钥集中在 `scripts/release/config.sh`。Sparkle 私钥（`~/Sync/apple-developer/sparkle/agent-session-manager_ed25519.key`）永远不要重新生成
- `make icon`：把 `assets/icon/` 的两张 SVG 渲染成 `AppIcon.icns`；改了 SVG 要重跑，并提交生成的 `.icns`

## 验收流程

1. 熄屏时截图是黑的：截图脚本自带 `wake-screen`，手动截图前先跑一次
2. 截图必须用 `Read` 打开看过再下结论，不要只看文件大小
3. **不要用 System Events / osascript / cliclick 发按键，也不要动剪贴板**：全局按键只进最前台的 app，会打进用户正在用的窗口，GPUI 也收不到 cliclick 的按键。交互用环境变量驱动的 autoplay（app 每步稳定后打日志标记，脚本等到标记再按窗口 ID 截图）；cliclick 鼠标点击只用于无法程序化打开的弹出层
4. 通过后把截图归档到 `tmp/<date>-<task>/`（已 gitignore）并在报告里给出路径
5. 里程碑完成后更新本文件的「当前状态」和 `kb/docs/product-prd.md` 的状态表，然后 commit

## core crate 编码规则

改 core 前读 `kb/notes/2026-09-17-m1-core-crate.md`（M1 笔记：模块划分、下列规则的原因、Python 比对方法）。

- 对外类型在 `model.rs`（`SessionSummary` = `sessions` 表一行，`MessageRow` = `messages` 表一行）；UI 层只消费这些类型，不自己解析 JSONL
- 解析器先写行为测试：fixture 在 `crates/core/tests/fixtures/`，由该目录的 `gen_fixtures.py` 生成（`uv run` 重跑并提交结果）；新记录类型先加进生成脚本再改 `parse.rs`
- 改 `parse.rs::classify`（字节预过滤）前，按 M1 笔记的方法在真实数据上全量验证
- `&RawValue` 不能放在 `#[serde(tag)]` / `untagged` / `flatten` 之下；记录和 block 都用「大结构体 + Option 字段 + match type」
- 改 `schema.sql` 必须同步 `db.rs` 的 `SESSION_COLUMNS` 和 `row_to_session` 的列序。索引没有迁移：打开时版本号不符，或磁盘上的 schema 与本版本建出的不一致，都会整库重建。schema 的变化会被自动发现；schema 看不出的变化要提升 `SCHEMA_VERSION`，包括改 `pricing.rs` 的价格和改解析结果（费用在索引时算好存进 `api_usage`，不重建就不会变）
- 用量按 API 响应一行存在 `api_usage`（主键 `session_id, agent, msg_id`，subagent 文件的 `agent` 为文件名 stem）。`sessions` 上的费用 / token 列只由 `refresh_session_usage` 从这张表汇总写入，`upsert_session` 不写它们。同一次响应会被增量扫描切成两半，所以去重只能靠这张表，不能在 `SessionDelta` 里累加
- `session_model_usage`（用量按模型 × fast × prompt 是否超过 `LONG_PROMPT` 分组）也只由 `refresh_session_usage` 重写，`SessionSummary::set_usage` 在内存里按同样规则分组（有测试断言两者一致）。启动时直接读这张表，不要改回读 session 时对 `api_usage` 现做 GROUP BY（冷缓存多 350ms）
- 一个 `Db` 只在一个线程用；rayon 只用于解析，写库在调用线程的单个事务里
- `SessionDelta::apply` 必须保持「全量解析 == 多次增量解析」（有测试断言）；新字段要归类为 first（只设一次）、last（覆盖）、count（累加）或 max（取较大值，如 `context_peak`）

## GPUI / gpui-kit 编码规则

改界面代码前先读 `kb/docs/gpui-kit-guide.md`。不读也不能违反的：

- 只依赖 `gpui-kit = "=0.6.1"`（`[dev-dependencies]` 里同一版本加开 `test-support`），**不要**再声明 `gpui` 或 `gpui-pre`，否则两份 GPUI 类型不匹配
- **API 以本地源码为准**，在线文档和 docs.rs 都有偏差。查签名直接 grep `~/.cargo/registry/src/index.crates.io-*/{gpui-component-0.6.1,gpui-base-0.6.1,gpui-pre-0.3.5}/src`
- 数据只经 `store.rs` 进出（UI 发 `StoreRequest`，后台线程回 `StoreEvent`）；`SessionRow::from_summary` 是 core 类型到界面行的唯一转换点
- 搜索只经 `search.rs` 进出：每次搜索取代上一次，`on_search_events` 丢弃旧 `seq` 的事件；搜索对象是通过当前过滤条件的 session，所以过滤变化或整表 reload 后要重搜
- 只有 SQLite 查询和文件扫描走后台，过滤 / 排序在主线程做；`render_td` 只取预格式化的字符串，不做计算
- 色值集中在 `style.rs`，不要在别处写 hex
- `Task` drop 即取消：`cx.spawn` 的结果要 `.detach()` 或存进字段；`this.update(cx, ..)` 返回 Result，用 `.ok()` 不要 unwrap
- 测试模块里要写 `use core::prelude::v1::test;`，否则 `use gpui_kit::*` 带进的 `test` 宏会让 `#[test]` 展开递归超限
- 表格的 `TableEvent::SelectRow` 是延后送达的通知：`on_table_event` 只处理与 `selected_row()` 一致的那个。照做旧事件会让两次选中互相触发、主线程空转（原因见 `kb/docs/app-architecture.md`「选中」）
- 焦点必须始终在应用的某个元素上：没有焦点时快捷键和菜单动作到不了 `AppRoot`。启动时焦点给表格；新增弹出层，关闭后要把焦点还回去
- 改快捷键、焦点或搜索框的按键处理，先在 `keyboard_tests.rs` 写行为测试（整个应用跑在无界面窗口里，按键走 GPUI 的分发）
- 程序化 `InputState::set_value` 不发 `InputEvent::Change`，之后要自己触发后续逻辑（如 `AppRoot::set_query` 直接调 `run_search`）

## 关键决策

- UI 走 gpui-kit 精确锁版（提交 Cargo.lock）：官方 `gpui` 在 crates.io 停更，Zed 的 `ui` / `theme` crate 是 GPL 且未发布，只读源码不依赖。理由见 `kb/docs/gpui-framework-research.md` §1
- 数据层：SQLite（rusqlite bundled，WAL，单写线程）+ FTS5 trigram，只索引 user / assistant 文本；正则模式用 memmap2 + memchr + `regex` 扫原始文件，在 JSON 解码后的文本上匹配
- SDK 与后台 session 不隐藏，用 Kind 列徽章区分
- bundle id 是 `com.reorx.agent-session-manager`；数据目录保持 `dev.reorx.AgentSessionManager`，不迁移
- 发布只出 Apple Silicon（arm64），macOS 14 起；正式发版只打 tag，版本号只来自 git tag，构建号是 CI run number
- 画稿用 Artifact（HTML 页面），以当前界面的截图和 `style.rs` 的色值为基准；不再使用 Claude Design，2026-09-16 的旧设计稿不再是依据

## Documentation

`kb/docs/`（描述现状）：

- `kb/docs/app-architecture.md` — 线程划分、冷 / 热启动的事件顺序、扫描后整表重读的原因、搜索何时重跑、键盘焦点、选中与视图状态保存的规则；改 `store.rs`、`search.rs`、`main.rs` 的事件处理或 `settings.rs` 前读，加线程或加事件后要更新它
- `kb/docs/gpui-kit-guide.md` — 表格、主题、菜单、搜索框与键盘、命中高亮、长列表的做法，已核实的 API，键盘测试的写法，截图脚本；改界面代码前读
- `kb/docs/claude-session-data-format.md` — `~/.claude` 数据源与 JSONL 记录类型实测、用量与费用的字段；改解析器、派生字段、history 回退逻辑前读
- `kb/docs/data-layer-design.md` — 开工前的设计稿：存储选型论证、DDL、增量索引；类型草案与线程方案已与实现不符（顶部有说明）。改 schema、索引器、增量更新、搜索前读
- `kb/docs/release.md` — CI 流程、`.app` 的组成、签名与公证、Sparkle 与密钥、Homebrew cask、本地出包、发布后验收、失败处理；发版、改打包或更新逻辑前读
- `kb/docs/product-prd.md` — 产品视角的功能描述与实现状态表；核对功能状态或向他人介绍应用时读
- `kb/docs/gpui-framework-research.md` — 开工前的调研：GPUI 心智模型、组件目录、选型理由；用到还没用过的组件或重新评估依赖时读

`kb/refs/` — 官方文档副本（2026-11-12）：Claude API 价格与缓存倍率、prompt caching、compaction、Claude Code 的 prompt caching；改 `pricing.rs`、用量解析或缓存失效判定前读，与线上页面不一致时以线上为准

plans / notes / sessions（当时的方案、实测与取舍），改到对应部分前读：

- `kb/notes/2026-09-21-m2-real-data.md` — store 线程与事件流、流式扫描时序；改 `store.rs` / `main.rs` 数据流前读
- `kb/notes/2026-09-21-m3-search.md` — 搜索的取代机制、两段式 FTS 的原因、`Matcher`、Matches 列；改 `search.rs`、`fts.rs`、`regex.rs::scan_sessions` 或搜索界面前读
- `kb/plans/2026-10-26-m4-detail-panel.md` — 详情面板总 plan：回合、AUTO、步骤、命中位置等概念，决策 D1–D12；改详情面板前先读它，再读 `kb/plans/2026-10-26-m4a-turn-cards-spec.md`（回合卡、行模型、命中跳转、读取线程）或 `kb/plans/2026-10-26-m4b-actions-and-extras-spec.md`（操作入口、导出、History、`[` / `]`、代码块）
- `kb/sessions/2026-10-27-asm-m4a-accept.md`、`kb/sessions/2026-10-27-asm-m4b-accept.md` — M4 验收证据、打开速度实测、与 spec 的差异；resume 命令的依据在 `kb/sessions/2026-10-27-asm-m4b-resume.md`
- `kb/plans/2026-10-23-cost-context-columns.md` — 费用口径的实测依据、价格表、自定义时间范围；改费用口径、价格表或时间过滤前读
- `kb/plans/2026-11-10-cost-analysis.md` — 主模型定义、换模型计价口径、分组用量存表的原因；改 Cost tab、Model 过滤或 `asm cost --as` 前读
- `kb/plans/2026-11-03-context-window-from-catalog.md` — 模型目录格式、合并优先级、刷新；改模型目录读取或窗口来源前读
- `kb/plans/2026-11-12-columns-and-cache-breaks.md` — 列开关、命中率、缓存失效的判定与原因；改表格列、`conversation.rs::cache_check` 或回合卡说明行前读
- `kb/plans/2026-11-16-cache-break-accuracy.md` — 失效判定的新规则：用量明细（`iterations`）、按 rewind 的祖先调用比较、请求记录（`api-request`）给出的原因、分段的缓存时长；改用量解析、失效判定（实现后在 `claude/cache.rs`）或回合卡失效文案前读
- `kb/plans/2026-10-26-favorites.md` — 收藏快照、目录迁移、去重优先级、索引根目录守卫；改收藏流程前读
- `kb/notes/2026-10-10-transcript-reader-xml.md`（导出格式）、`kb/notes/2026-10-10-transcript-reader.md`（压缩规则）；改导出内容前读
- `kb/sessions/2026-10-28-release-1.0.md` — 1.0 发布的决定、升级测试证据、Sparkle 延后弹提示的原因
- `kb/notes/2026-10-28-app-icon.md` — 图标定稿、小尺寸的像素对齐、打包时的核对项；改图标前读
