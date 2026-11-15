---
created: 2026-11-15
tags:
  - cli
  - skill
  - release
  - agent-session-manager
  - plan
---

# P4 分发：`asm` 随 app 安装，skill 可以一键装上

总 plan 见 [index.md](index.md)；在 P1–P3 的 CLI 参数稳定之后做。

## PRD

### 背景

`asm` 现在只能在开发机上从源码构建；1.0 的 app 安装包和 Homebrew cask 里都没有它。别的用户装了 skill，agent 找不到命令，skill 就用不了。skill 本身也只在本仓库里可用。

### 目标

用户装好 app，就能在终端和 agent 里使用 `asm`；skill 可以用一条命令装进 Claude Code，版本与 CLI 匹配。

### 功能与交互

- 通过 Homebrew 安装 app 时，`asm` 自动出现在 PATH 里；升级 app 时一起升级。
- 通过 dmg 安装的用户，在 app 菜单里选「Install Command Line Tool…」，把 `asm` 装到 PATH 里；已装过则提示已安装、位置在哪里；需要管理员权限时给出明确提示。
- 安装 skill：`asm skill install` 把 skill 装进 Claude Code 的用户 skill 目录（已存在时提示并可覆盖）；`asm skill path` 显示 skill 文件所在位置。也可以从公开仓库的 `skills/asm/` 直接安装。
- skill 开头检查 `asm` 是否可用、版本是否够；不够时告诉用户怎么装或升级。
- `asm` 与 app 共用同一份索引；app 没运行时 `asm` 也能独立使用。

### 不做的事

- 不支持 Intel Mac 和其他系统（与 app 的发布范围一致）。
- 不发布到 Claude Code 插件市场之外的渠道。

### 验收标准

- 在一台干净的机器上用 Homebrew 装 app 后，终端里 `asm --version` 与 app 版本一致；升级后同步。
- 用 dmg 安装后，从菜单安装命令行工具，`asm` 可用。
- `asm skill install` 后，新开的 Claude Code 会话能加载 skill，完成总 plan 里的两个端到端例子。

## Tech Design

To be determined。等 CLI 稳定后再写。已知要处理的：`asm` 作为 `.app` 里的第二个可执行文件参与签名和两轮公证（`scripts/release/sign-app.sh`、`notarize.sh`），cask 用 `binary` 指向它，skill 文件随 `.app` 一起打包供 `asm skill install` 使用；发布流程见 `kb/docs/release.md`。
