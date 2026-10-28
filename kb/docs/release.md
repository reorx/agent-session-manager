---
created: 2026-10-28
tags: [release, signing, notarization, sparkle, homebrew, ci]
---

# 发布

> 写给要发新版本、排查发布失败、或在新机器上本地出包的人。通用流程见 `~/.claude/skills/macos-app-release/`（本项目不是 Xcode 项目，脚本从它的模板改写而来，差异见文末）；Apple 凭据的权威说明在 `~/Sync/apple-developer/AGENTS.md`。

用户安装：

```sh
brew install --cask reorx/tap/agent-session-manager
```

或从 [Releases](https://github.com/reorx/agent-session-manager/releases) 下载 `AgentSessionManager-<version>.dmg`。只支持 Apple Silicon，macOS 14 及以上。

## 发一个版本

在 master 上打 `vMAJOR.MINOR.PATCH` tag 并 push，其余全部由 CI 完成：

```sh
git tag v1.1.0
git push origin v1.1.0
```

版本号只来自 tag（`v1.1.0` → Info.plist 的 `CFBundleShortVersionString` 1.1.0），构建号（`CFBundleVersion`）取 CI 的 run number，Sparkle 按构建号比较新旧。Cargo.toml 的 `version` 与产品版本无关，发版不用改。

## 产物

`make build-release` 产出 `<cargo target>/release/Agent Session Manager.app`：

```
Contents/
  Info.plist                       bundle-app.sh 写入：版本、bundle id、图标、Sparkle 配置
  MacOS/agent-session-manager      cargo build --release 的二进制（arm64）
  Resources/AppIcon.icns           assets/icon/AppIcon.icns
  Frameworks/Sparkle.framework     Sparkle 2.10.0 官方发布包里的 framework
```

- bundle id `com.reorx.agent-session-manager`；数据目录沿用 `~/Library/Application Support/dev.reorx.AgentSessionManager/`（`directories` crate 算出来的，与 bundle id 无关，没有迁移）
- 只有 hardened runtime，没有 entitlements：不沙盒，不用 JIT，只读写普通文件
- dmg 文件名用 `AgentSessionManager-<version>.dmg`：GitHub 会把 Release 资源名里的空格换成点，所以不能用带空格的 app 名
- 所有名字、最低系统、架构、Sparkle 公钥集中在 `scripts/release/config.sh`；workflow 的 `env` 重复了 `DMG_NAME` 和 `CASK_TOKEN`

## CI 流程

`.github/workflows/release.yml` 有两个 job：

**release**（macos-15，arm64）
1. mise 装 `mise.toml` 里的 Rust，`make setup` 补 Metal Toolchain，`Swatinem/rust-cache` 缓存 `target/`
2. `make test` 与 `make test-release`
3. 临时 keychain 导入 Developer ID Application 证书
4. `make build-release VERSION=<tag> BUILD_NUMBER=<run number>`：`cargo build --release` → `bundle-app.sh` 拼 .app → `sign-app.sh` 由内到外重签全部嵌套代码（主程序和 Sparkle 的 Autoupdate、Updater.app、两个 XPC 服务、framework）→ `verify-app.sh` 逐个自查 Developer ID、hardened runtime、时间戳、无 `get-task-allow`，另查主程序含 arm64
5. `make notarize-app dmg notarize`，公证两轮：
   - app 打 zip 公证，票据钉进 app。Homebrew 装的是从 dmg 里拷出来的 app，只钉 dmg 的话装好的 app 没票据，离线首次启动会被 Gatekeeper 拦
   - 用已带票据的 app 打 dmg → 签 dmg → 公证 → staple → 终验：挂载后对 app 评估要求 `source=Notarized Developer ID`，且 app 自带票据
6. staple 之后算 `SHA256SUMS`，`make appcast` 对最终 dmg 签名并生成 `appcast.xml`
7. 上传 artifact，在本仓库创建 Release（安装说明 + 自动生成的变更列表），附件是 dmg、`SHA256SUMS`、`appcast.xml`

**homebrew**（ubuntu，只在 tag 触发时跑）
1. 从刚发布的 Release 下载 dmg 与 `SHA256SUMS` 并核验
2. `scripts/release/render-cask.sh` 生成 `Casks/agent-session-manager.rb`
3. 拒绝降级（tap 里已有更高版本就失败），用 deploy key 推到 `reorx/homebrew-tap`

`workflow_dispatch` 手动触发只跑 release job 到 artifact 为止（版本 0.0.0），不发 Release、不动 tap，用来验证链路。

Homebrew 7 默认只加载 trust 过的第三方 tap，新机器上先 `brew trust --tap reorx/tap`。cask 声明了 `auto_updates true`，`brew outdated` 和普通 `brew upgrade` 会跳过它，日常走应用内更新。要用 Homebrew 主动升级：

```sh
brew update
brew upgrade --cask --greedy reorx/tap/agent-session-manager
```

## Sparkle 应用内更新

- app 不链接 Sparkle，`crates/app/src/updater.rs` 在启动时用 `NSBundle` 加载 `Contents/Frameworks/Sparkle.framework`，再用 `objc2` 创建 `SPUStandardUpdaterController`。裸跑的 debug 二进制和测试没有这个 framework，就没有更新器，菜单里也没有「Check for Updates…」
- 应用菜单：About Agent Session Manager（系统标准面板，显示版本和构建号）、Check for Updates…（Sparkle 自己的窗口）
- 配置都在 Info.plist（`bundle-app.sh` 写入）：feed 为 `https://github.com/reorx/agent-session-manager/releases/latest/download/appcast.xml`；每天检查一次（`SUScheduledCheckInterval` 86400）；不静默安装（`SUAutomaticallyUpdate` / `SUAllowsAutomaticUpdates` 为 false），由用户点安装并重启
- appcast 每次只有当前正式版一项，下载地址是版本化的 `releases/download/v<version>/AgentSessionManager-<version>.dmg`
- Sparkle 的「Install and Relaunch」通过 `NSApp terminate:` 退出，和菜单 Quit 走同一条路（GPUI 的 `applicationWillTerminate:` 回调）
- 本地测试可以换 feed：`FEED_URL` 改 Info.plist 的 `SUFeedURL`，`DOWNLOAD_URL` 改 appcast 的下载地址
- 实测（2026-10-28，本机 feed）：首次启动（偏好里没有 `SULastCheckTime`）就会自动检查，不等一天；「Install and Relaunch」后 Sparkle 重启的进程不继承启动时的环境变量，`ASM_DB` / `ASM_STATE` 会丢，重启后的 app 用的是真实索引

### 不打 tag 测更新流程

改了 `updater.rs`、Info.plist 的 Sparkle 键或 Sparkle 版本时，先在本机走一遍，再发正式版。已发布的版本更新器坏了，装了它的人就收不到后续更新。脚本在 `tmp/2026-10-28-release/e2e-setup.sh`（tmp 不入库，做法如下）：

1. `make build-release VERSION=0.9.0 BUILD_NUMBER=1 FEED_URL=http://127.0.0.1:8765/appcast.xml`，把产物 `ditto` 到临时目录作为旧版
2. `make release-local VERSION=0.9.1 BUILD_NUMBER=2 FEED_URL=… DOWNLOAD_URL=http://127.0.0.1:8765/AgentSessionManager-0.9.1.dmg`，把 dmg 和 `appcast.xml` 拷到 feed 目录，`python3 -m http.server 8765 --bind 127.0.0.1 --directory <feed 目录>`
3. 先备份 `~/Library/Application Support/dev.reorx.AgentSessionManager/state.json`，再带 `ASM_DB` / `ASM_STATE` 启动旧版。弹出更新提示后点 Install Update → Install and Relaunch（Sparkle 的窗口用 AX 点，不发按键），核对同一路径的 Info.plist 变成 0.9.1 (2)、About 面板显示 0.9.1 (2)、再「Check for Updates…」提示 up to date
4. 清理：按 PID 结束测试进程，恢复 `state.json`，`defaults delete com.reorx.agent-session-manager`，删 `~/Library/Caches/com.reorx.agent-session-manager` 和 `~/Library/HTTPStorages/com.reorx.agent-session-manager`，停掉 feed 服务

密钥：

- 私钥备份在 `~/Sync/apple-developer/sparkle/agent-session-manager_ed25519.key`（600），同一把在登录钥匙串里，account 为 `agent-session-manager`；CI secret 是 `SPARKLE_ED_PRIVATE_KEY`
- 公钥写在 `scripts/release/config.sh` 的 `SPARKLE_PUBLIC_KEY`
- **永远不要重新生成**：已安装的版本只接受这把钥匙签的更新，丢了或换了，它们就再也收不到更新。重配 secret 用 skill 的 `setup-secrets.sh --sparkle`，它只会复用备份

## secrets

| Secret | 内容 |
|---|---|
| `CERT_P12` | Developer ID Application 证书 + 私钥的 p12，base64 |
| `CERT_P12_PASSWORD` | p12 密码 |
| `APPLE_TEAM_ID` | Team ID |
| `ASC_PRIVATE_KEY` | App Store Connect API key（Developer 角色）p8，base64 |
| `ASC_KEY_ID` / `ASC_ISSUER_ID` | 上面那把 key 的 ID 与 Issuer |
| `TAP_DEPLOY_KEY` | `reorx/homebrew-tap` 上标题为 `release: reorx/agent-session-manager` 的可写 deploy key 的私钥 |
| `SPARKLE_ED_PRIVATE_KEY` | Sparkle EdDSA 私钥（导出文件原样，不再次 base64） |

全部由 skill 脚本一次配好（可重复执行，deploy key 会轮换，Sparkle 密钥只复用）：

```sh
~/.claude/skills/macos-app-release/scripts/setup-secrets.sh --repo reorx/agent-session-manager --sparkle --cask-token agent-session-manager
```

## 本地出包

```sh
make build-release     # .app：cargo build、拼 bundle、签名、自查
make notarize-app      # 公证 app 并钉票据（凭据从 ~/Sync/apple-developer/secrets.env 读）
make dmg               # → dist/release/AgentSessionManager-<version>.dmg
make notarize          # 签 dmg + 公证 + staple + 终验
make appcast           # 对最终 dmg 签名，生成 dist/release/appcast.xml（默认用私钥备份文件，不弹钥匙串）
make release-local     # 以上步骤连跑，不上传、不改 tap
make release-local VERSION=0.9.0 BUILD_NUMBER=1   # 指定版本号和构建号
make test-release      # 发布脚本的行为测试，不碰钥匙串、网络和签名
```

需要本机钥匙串里有 Developer ID Application 证书和 `~/Sync/apple-developer/secrets.env`。产物在 `dist/release/`，与 `make build-transcript-reader` 的 `dist/transcript-reader` 分开（`release-local` 会先清空 `dist/release/`）。

## 发布后验收

确认两个 CI job 都成功后，核对 Release 上有 dmg、`SHA256SUMS`、`appcast.xml`，tap 里 cask 的版本和 sha256 与 Release 一致。

用户安装通道：`brew fetch --cask reorx/tap/agent-session-manager` 验下载，再 `brew install --cask --appdir=<临时目录> reorx/tap/agent-session-manager` 安装；`spctl -a -t execute -vv <app>` 应显示 `source=Notarized Developer ID`，`syspolicy_check distribution <app>` 应通过。清理前先 `pgrep -fl "Agent Session Manager.app"`，有别的实例在跑就不要 `brew uninstall`（cask 的 `uninstall quit` 会按 bundle ID 把它也退出），直接删临时 app 和 `$(brew --prefix)/Caskroom/agent-session-manager`。

Sparkle 的分发核对：下载 Release 上的三个文件，核对 appcast 的版本、构建号、长度、版本化下载地址，用备份密钥跑 `sign_update --verify <dmg> <signature>`，确认 `releases/latest/download/appcast.xml` 与本次 Release 的 appcast 字节一致、安装包的 `SUPublicEDKey` 与 `config.sh` 一致。这些只证明分发链路；真实更新要用同 bundle id、同公钥、同 feed 的低版本（`make build-release VERSION=0.9.0 BUILD_NUMBER=1`）在临时目录运行，从真实 feed 完成发现 → 下载 → 安装 → 重启。每日自动检查要等装着上一正式版的机器自然触发，登记在 `kb/next-up.md`。

## 失败了怎么办

- **只有 homebrew job 失败**（deploy key 失效、tap 冲突）：修好后在 Actions 页面 Re-run failed jobs。它只读已发布的 Release，不会重新签名公证
- **release job 失败**：看日志里的步骤。公证被拒时脚本会自动打印 `notarytool log`。瞬态网络问题先重跑原 run；需要改代码时，修好后删掉 tag 重打：`git push --delete origin vX.Y.Z && git tag -d vX.Y.Z`，前提是 Release 还没建出来
- **stapler 报 Error 68 / network connection was lost**：`notarize.sh` 已重试 5 次；仍失败就重跑原 run
- Release 已存在时发布步骤失败，不使用 `--clobber`。发布后修复发新版本，不替换已发布字节：cask 的 sha256 和 Sparkle 签名都绑定旧文件

## 与 skill 模板的差异

- 没有 XcodeGen / xcodebuild：`bundle-app.sh` 拼 bundle，Info.plist 由它生成；没有 `Config/Release.xcconfig`，签名全在 `sign-app.sh`
- 只出 arm64：`verify-app.sh` 查 `config.sh` 的 `ARCHS`，不查 Universal
- Sparkle 不走 SPM，用 `sparkle-tools.sh` 下载的官方 tar 包里的 framework；Swift 的 `UpdateController` 换成 `updater.rs`，没有 Debug 专用 delegate（Debug 二进制根本没有 Sparkle）
- 名字从 `config.sh` 读，不靠 apply-templates 替换占位符；dmg 名与 app 名分开
- 仓库公开，cask 只有普通 URL，没有私有仓库分支

## 相关文件

- `.github/workflows/release.yml`
- `scripts/release/`：`config.sh`、`bundle-app.sh`、`list-code.sh`、`sign-app.sh`、`verify-app.sh`、`make-dmg.sh`、`notarize.sh`、`render-appcast.sh`、`sparkle-tools.sh`、`render-cask.sh`、`release-notes.sh`、`test-release.sh`
- `crates/app/src/updater.rs`
- `Makefile`：`build-release` / `notarize-app` / `dmg` / `notarize` / `appcast` / `test-release` / `release-local`
