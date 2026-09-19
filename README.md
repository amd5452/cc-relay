# CC Relay





公益AI微信交流群


> A fork of [farion1231/cc-switch](https://github.com/farion1231/cc-switch) that adds
> **subscription-quota aware relay** for the local proxy.
>
> 基于 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 的二次开发分支，
> 新增「订阅配额本地透明接力」，并把本地路由从 4 个智能体扩展到 7 个。

**English TL;DR** — When a provider's per-cycle token quota runs out, the local proxy
transparently relays the request to the next provider that still has quota. Every decision
happens **in memory, before the request is sent** — no agent config file is rewritten at
runtime, and a single request always goes to exactly one upstream, so a streamed response is
never cut in half.

---

## 为什么需要它 / Why

编程订阅大多按**滚动时间窗口**计量，而不是按余额：Claude Code 的 5 小时窗口、Kimi / GLM /
OpenCode Go 的套餐等等，每个周期都有 token 上限。

额度用尽那一刻，请求开始报错，而你要**自己发现**、再**手动**去 CC Switch 里换一个供应商。
这会打断正在跑的会话，而且失败得很安静——往往是你发现代码不生成时才意识到。

原版 CC Switch 已有本地路由和故障转移，但**故障转移只对网络/HTTP 错误生效**，它不理解
「这个周期的额度花完了」这件业务层面的事。

本分支补上了这一层。

---

## 和上游的差异 / What's different

| | 上游 upstream | 本分支 this fork |
|---|---|---|
| 本地路由覆盖 | Claude Code、Claude Desktop、Codex、Gemini、Grok Build | **+ OpenCode、OpenClaw、Hermes** |
| 配额感知 | — | **按供应商的周期额度、自动接力、按量付费兜底** |
| 数据库 | — | `providers` 表新增 4 列，schema v18 → v19 |

---

## 接力是怎么工作的 / How the relay works

判定完全发生在**本地代理里、请求转发之前**。整个过程只重排内存中的供应商链路，
**绝不回写任何智能体的配置文件**。

每个候选供应商按下面的顺序判定：

1. **标记按量付费** → 进兜底档，永不优先使用
2. **未配置周期额度** → **不参与接力**（见下方「为什么这样设计」）
3. **无周期起点 / 窗口已过期** → 记为「待重置」，进首选档
4. **本周期已消耗 < 上限** → 进首选档
5. 其余跳过

首选档为空则使用兜底档；两档皆空返回 **429**。

### 三个刻意的设计取舍

**① 只在请求发出前决策。** 一次请求只打一个上游，所以**不存在「流式输出到一半换 API」**。
切换只可能发生在两次请求之间，即上一次输出完整结束之后。

**② 未配置额度 = 不参与接力。** 而不是「不限量」。否则池子会被没配额度的供应商无限撑大，
导致按量兜底永远轮不到、429 也永远不可达。**要无条件兜底，请显式勾选「按量付费」。**

**③ 惰性重置，没有后台定时器。** 窗口过期后在**下一次请求**里重置，并且**只给最终被选中的
那个供应商**写新的周期起点——备用供应商不会在闲置时白白消耗自己的窗口时长。

### 额度按什么口径算

```
已消耗 = 干净输入 + 缓存读 + 缓存写 + 输出
```

「干净输入」复用 CC Switch 已有的 `fresh_input_sql`，把上游口径里**本身就含缓存**的
`input_tokens` 归一化，因此不会重复计数，数字与「用量统计」页一致。

> ⚠️ 注意：**缓存读也计入**。Claude Desktop 这类客户端每轮都会重读整个上下文，缓存读是
> 大头，所以长会话消耗会很快。如果你的订阅方不把缓存读按全价计入，这个口径会偏严。

---

## 怎么用 / Usage

### 1. 配置供应商额度

设置页打开「启用本地路由」总闸 → 任意供应商的编辑框 → 底部「订阅配额接力」：

| 字段 | 说明 |
|---|---|
| **周期 Token 上限** | 单位 **M（百万）**。填 `1` = 100 万。**留空表示不参与接力** |
| **周期时长** | 小时。如 `5`。**必须和上限同时填才生效** |
| **按量付费兜底** | 勾选后，所有周期额度耗尽时才会使用它；额度恢复后自动切回 |

**上限和时长必须同时填**，只填一项不会生效（日志里会有提示）。

### 2. 各智能体的代理地址

开启本地路由后，各智能体请求会走这些路径：

| 智能体 | 代理地址 |
|---|---|
| Claude Code | `http://127.0.0.1:15721` |
| Claude Desktop | `http://127.0.0.1:15721/claude-desktop` |
| Codex | `http://127.0.0.1:15721/v1` |
| Gemini | `http://127.0.0.1:15721` |
| Grok Build | `http://127.0.0.1:15721/grokbuild/v1` |
| OpenCode / OpenClaw / Hermes | `http://127.0.0.1:15721/<应用名>/v1` |

**这些地址由软件自动写入，不需要手工配置。** 上表用于排查问题时核对。

### 3. 看日志确认接力是否生效

日志在 `~/.cc-switch/logs/cc-switch.log`（Windows：`C:\Users\<你>\.cc-switch\logs\`）。

搜索 `配额接力` 关键字，对照下表：

| 看到 | 含义 |
|---|---|
| 完全没有相关行 | 请求没经过本地代理，或没开本地路由 |
| `未启用配额接力` | 没有任何供应商同时填了上限与时长 |
| `配额配置不完整` | 只填了一项，不生效 |
| `本周期已用 0/上限` 且从不上涨 | 请求没成功，用量没落库（上游报错时 token 恒为 0） |
| `需要重置周期` | 窗口已过期，即将重置 |
| `本周期额度已满（x/y），尝试下一个` | 触发接力 |
| `配额接力改变了选路：原首选 X → 实际使用「Y」` | **接力确实生效** |
| `接力池内无可用供应商` | 所有候选耗尽且无兜底 → 返回 429 |

---

## 支持的智能体 / Supported agents

| 智能体 | 本地路由 | 配额接力 |
|---|---|---|
| Claude Code | ✅ | ✅ |
| Claude Desktop | ✅（它自带的路由体系） | ✅ |
| Codex | ✅ | ✅ |
| Gemini CLI | ✅ | ✅ |
| Grok Build | ✅ | ✅ |
| OpenCode | 🆕 | ✅ |
| OpenClaw | 🆕 | ✅ |
| Hermes | 🆕 | ✅ |
| Pi | ❌ | ❌（无代理适配器，不支持） |

**Claude Desktop 的两点特殊要求**：它的 gateway 有独立鉴权；且每个供应商必须配置
「模型路由映射」，否则无法承接请求——接力选路会自动跳过这类供应商。

---

## 构建 / Build

```bash
pnpm install
```

**只出便携 exe**（推荐，不需要签名私钥）：

```bash
pnpm tauri build --no-bundle
# 产物：src-tauri/target/release/cc-switch.exe
```

前端资源已打进 exe 内部，**单独拷走就能跑**。

**出安装包**需要 Tauri 更新签名私钥，而私钥只在官方维护者手里，从源码构建时无法生成：

```bash
pnpm build            # 会报 TAURI_SIGNING_PRIVATE_KEY 缺失
```

想本地出安装包，可临时把 `src-tauri/tauri.conf.json` 里的
`createUpdaterArtifacts` 改成 `false`（改完记得还原，否则会污染你的工作区）。

### 独立运行数据（不污染现有配置）

CC Switch 支持用环境变量覆盖数据目录：

```powershell
$env:CC_SWITCH_TEST_HOME = 'D:\cc-switch-test'
.\cc-switch.exe
```

该目录下会生成独立的 `.cc-switch\`（数据库、日志）以及 `.claude\`、`.codex\` 等智能体配置。
**但 Claude Desktop 的配置目录例外**——它读 `%LOCALAPPDATA%\Claude-3p`，不受此变量影响。

---

## 已知限制 / Known limitations

- 被拒绝的请求（429）**不消耗额度**，但客户端会看到一次失败。如果你希望"宁可超支也不中断"，
  可以给一个供应商勾上「按量付费兜底」。
- `sync_live_config_to_provider` 尚未覆盖 OpenCode / OpenClaw / Hermes——这三个应用的
  live 配置改动不会回同步到数据库。**不影响接力**（接力只读数据库用量）。
- 额度统计包含缓存读，见上文口径说明。

---

## 致谢与许可 / Attribution & License

本项目的全部基础功能来自 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)，
由 **Jason Young** 开发并开源。本分支只新增了「订阅配额接力」及本地路由的覆盖面扩展。

依照 **MIT License** 分发，完整许可证见 [LICENSE](./LICENSE)。

```
MIT License

Copyright (c) 2025 Jason Young
```

---

# English Section

## What this adds to upstream

**Subscription-quota aware relay.** Coding-plan subscriptions are metered by rolling time
windows, not by balance — Claude Code's 5-hour window, Kimi / GLM / OpenCode Go plans, and so
on. When a provider's per-cycle token cap is reached, the original proxy keeps sending requests
to it and they start failing; the user has to notice and switch providers manually, which breaks
an in-flight agent session.

This fork makes the local proxy aware of those cycles. On every request, **before forwarding**,
it walks the provider list and picks the first one that still has quota in its current cycle. If
everything is exhausted, an explicitly flagged pay-as-you-go provider is used; if there is none,
the request is rejected with 429.

## Design notes

- **Decision happens before the request is sent.** One request goes to exactly one upstream, so a
  streamed response is never truncated. Switching can only happen between requests.
- **No config file is rewritten at runtime.** The agent's `base_url` already points at the local
  proxy (written once when you enable local routing); the relay only reorders an in-memory list.
- **A provider with no quota configured does not participate.** A provider meant as an
  unconditional fallback must be flagged `pay-as-you-go` explicitly.
- **Lazy cycle reset, no background timer.** An expired window is reset on the next request, and
  only the provider actually selected gets its window started.
- **Quota accounting** uses the same normalized formula as the Usage page
  (`fresh_input + cache_read + cache_creation + output`), so cache tokens are not double-counted.
  Note that cache reads **are** included — long agent sessions with prompt caching will consume
  quota quickly.

## Local routing coverage

Extended from 4 to 7 agents: Claude Code, Claude Desktop, Codex, Gemini, Grok Build, plus
**OpenCode**, **OpenClaw** and **Hermes** (each under its own path prefix so provider namespaces
stay isolated). Pi is excluded — it has no proxy adapter.

## Building

```bash
pnpm install
pnpm tauri build --no-bundle     # portable exe, no signing key needed
```

`pnpm build` (with bundling) requires the Tauri updater signing key, which only upstream holds.

## License

MIT — see [LICENSE](./LICENSE). All upstream work is by
[Jason Young](https://github.com/farion1231); this fork adds the relay layer.
