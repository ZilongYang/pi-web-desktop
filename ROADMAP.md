# Pi Web Desktop — 路线图 / Backlog

> 这里是「现在做到哪、接下来做什么」。**文档索引**见 [docs/README.md](docs/README.md)；
> **发版流程与版本号规则**见 [docs/RELEASE-PROCESS.md](docs/RELEASE-PROCESS.md)。
> 已批准方案的原文在 `docs/PLAN-*.md`（不修改，只在文末追加事后补记）。

## 一句话

把 [pi-web](https://github.com/agegr/pi-web)（Next.js 服务端 + 浏览器 UI）包成原生桌面 App：
**自带 Node 运行时 + 内置 pi-web**，双击即用，与 `pi` CLI / 浏览器版共享 `~/.pi/agent`。

- 仓库：<https://github.com/ZilongYang/pi-web-desktop>（MIT）
- 最新正式版：**v0.3.3**（内置 pi-web **0.15.0** / pi SDK 1.5.0 / Node 22.23.4）
- 测试通道最新：**v0.3.4-beta.2**（Pre-release；内置 pi-web **0.19.0** / pi SDK 1.6.1 / Node 24.22.3，子龙已验收）
- 平台：macOS 14+ arm64（Windows 代码路径已写、未验证；Linux 未做）

### 两条硬规则

1. **发布前必须由子龙在本机验收通过**（`cargo test` / `npm run smoke` / `npm run smoke:app` 只是前置条件，
   不能代替本机验收；beta 也一样，不许"先发再验"）。
2. **pi-web 版本随壳固定**（2026-11-04 取消 L3 热更新）：每次发版一次性固定 pi-web + Node；
   版本号默认只递增 patch（详见 [RELEASE-PROCESS](docs/RELEASE-PROCESS.md) 与
   [PLAN-2026-11-04-drop-l3](docs/PLAN-2026-11-04-drop-l3.md)）。

## 能力矩阵

| 能力 | 状态 | 备注 |
|---|---|---|
| 原生窗口 + 全中文菜单 + 页面缩放 | ✅ | 缩放 50%–300%，记忆在 `config.json` |
| 侧车管理（启动/探活/优雅退出/清孤儿） | ✅ | 进程组 SIGTERM→SIGKILL |
| 共享 `~/.pi/agent`、同版本自动附着 | ✅ | 版本不一致时弹窗让用户选 |
| PATH 继承登录 shell（nvm/pnpm/mnemon 可用） | ✅ | 缓存 + 兜底目录始终并集；陈旧自动重探测（10 分钟节流 / 24h 刷新 / 7 天 TTL）；菜单可手动重新载入 |
| 启动页 + 失败态（日志尾部/重试/打开日志） | ✅ | 失败弹窗在 headless 下不弹 |
| 应用自更新（Tauri updater + minisign） | ✅ | 进度窗 + 重启确认 |
| 更新通道（正式版 / 测试版 beta） | ✅ 0.3.0 | 菜单切换；允许降级重装 |
| 菜单栏托盘 + 关窗隐藏到菜单栏 | ✅ 0.3.1 | π 模板图标；托盘菜单复用主菜单动作；点 Dock 唤回 |
| 退出确认（⌘Q / 菜单 / 托盘） | ✅ 0.3.1 | 有会话在跑时提示"会中断 N 个" |
| 运行期定期检查 | ✅ 0.3.2 | 启动查一次 + 每 6 小时再查（`PIWEB_PERIODIC_CHECK_SECS` 可覆盖） |
| 导出会话 HTML | ✅ 0.3.2 | 0.3.4 起 pi-web 0.16+ 改为调内置 pi CLI 的 `dist/core/export-html`，导出补丁已删 |
| 窗口位置/大小记忆 | ✅ 0.2.10 | 物理像素；拔屏后自动居中 |
| ~~L3 pi-web 热更新~~ | ❌ 已取消 | 避免壳/侧车兼容矩阵 |
| Windows 支持 | ❌ 未验证 | 最大缺口 |
| Linux 支持 | ❌ 未做 | 预留 |

## 待办 Backlog

### P1 — 平台与验收

- **F3 功能手测清单**：终端面板、文件预览、拖拽上传、外部链接、导出、⌘Q 无残留、IME 中文输入
  （清单见 [QA](docs/QA-2026-11-01.md)）。
- **F4 Windows 支持**：先加 Windows CI job 验"能构建"，再找真机装一次；
  `fetch-sidecar --target win32-x64`、`taskkill /T /F`、`CREATE_NO_WINDOW`、NSIS 安装包。

- **扩展宿主能力兼容（上游）**：pi-core 的扩展契约只有 `tui`/`rpc` 两档，pi-web 是"rpc + 实现了 custom"的超集宿主，
  导致按契约写法的插件在本 App 里静默降级。本仓库现有兜底：插件级补丁（`scripts/patch-rpiv-ask-user-question.mjs`）、
  只看不动的审计（`scripts/check-extension-host-compat.mjs`）、宿主 `mode="tui"` 可回滚实验（`scripts/patch-pi-web-tui-mode.mjs`）。
  真正根除要上游 pi-core 增加能力协商、pi-web 如实声明 —— **未做，待子龙决定是否提 issue**（[BUG 记录](docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md) §12/§13）。

### P2 — 体验打磨

- **F6 下载可取消**：进度窗加「取消」，中断时清理临时文件
- **F7 检查中反馈**："正在检查…"（手动检查的结果反馈已于 0.3.2 补上）
- **F8 更新失败给建议**：检查网络/代理、打开日志
- **F9 缩放档位可调**：5% 步进或自定义
- **F11 Developer ID + 公证**：消掉首次"右键打开"（需 $99/年证书）
- **退出确认补全**：Dock 右键「退出」/ 系统关机注销走 AppKit terminate，目前拦不住
  （要拦需自己实现 `applicationShouldTerminate`）
- **CI 提速**（可选）：beta 跳过 dmg、侧车目录缓存，预计省 1–2 分钟

## 已修复记录（细节见 [QA](docs/QA-2026-11-01.md) 与对应 PLAN）

| 编号 | 内容 | 版本 |
|---|---|---|
| F1/F2 | 每次启动都检查更新（同版本只提醒一次）；App 菜单加更新入口 | 0.2.7 |
| B1 | 点「完整历史」崩溃（新窗口必须沿用发起方 webview configuration） | 0.2.8 |
| B2 | 导出 HTML 报 `sortChildren expected 1 match`：先打补丁容忍 0 匹配；0.15.0 上游自修后补丁自动跳过 | 0.2.9 / 0.3.2 |
| B3 | 0.2.9 启动即崩（`state() called before manage()`：菜单不能早于 setup 构建） | 0.2.10 |
| B4 | PATH 缓存自愈：缓存与兜底目录始终并集 + 陈旧自动重探测（10 分钟节流 / 24h 刷新 / 7 天 TTL）+ 菜单「重新载入登录环境」 | 0.3.3-beta.1 / 0.3.3 |
| F14 | L3 清单独立到固定 Release（后随 L3 一起取消） | 0.3.1 |
| F15 | 关于面板显示内置 pi-web / SDK / Node 版本 | 0.2.9 |
| F16 | 菜单栏托盘 + 关窗隐藏 + Dock 唤回 | 0.3.1 |
| F17 | 菜单文案区分「应用更新」与「pi-web 更新」 | 0.2.9 |
| F18 | 冒烟测试加导出断言；并修正"假通过"（现在会校验侧车真在服务） | 0.2.9 / 0.3.1 |
| F19 | 记住窗口上次的位置和大小 | 0.2.10 |
| F20 | 缩放/窄窗口下输入框横向滚动条压住文字 | 0.2.11 |
| F21 | 更新通道（正式版 / 测试版 beta） | 0.3.0 |
| F22 | 退出前一律确认 | 0.3.1 |
| — | 取消 L3 + pi-web 升到 0.15.0 + 运行期定期检查 | 0.3.2 |
| B5 | `ask_user_question` 在 App 内退化为「一题一个原生对话框」（不能回退改答案）：给插件 `@juicesharp/rpiv-ask-user-question` 打「TUI 路径」补丁，离线宿主模拟 51/51 通过；根因是 pi-core 契约缺能力协商 + pi-web 为超集宿主（附同类插件审计与宿主 mode=tui 可回滚实验）（[BUG 记录](docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md)） | 插件 2.15.0 本地补丁（非 App 版本；待真机验收） |
| — | 内置 pi-web 0.15.0 → **0.18.0**（pi SDK 1.6.0）+ Node 24.22.3；导出改用上游实现，删除 `scripts/patch-pi-web-export.mjs`（[PLAN](docs/PLAN-2026-11-25.md)） | 0.3.4-beta.1 |
| — | 内置 pi-web 0.18.0 → **0.19.0**（pi SDK 1.6.1），Node 保持 24.22.3；0.19 的 `frame-ancestors 'self'` / `X-Frame-Options: SAMEORIGIN` 只限制被嵌入，壳是顶层加载不受影响（[PLAN](docs/PLAN-2026-11-28.md)） | 0.3.4-beta.2 |

## 版本历史

| 版本 | 内容 |
|---|---|
| 0.3.4-beta.2 | 内置 pi-web 升 0.19.0（pi SDK 1.6.1），Node 保持 24.22.3；上游新增 CSP `frame-ancestors`/`X-Frame-Options`（不影响顶层加载）；新增引导/后续消息、网络代理、notices 设置 |
| 0.3.4-beta.1 | 内置 pi-web 升 0.18.0（pi SDK 1.6.0）+ Node 24.22.3；导出 HTML 改上游实现，删导出补丁；扩展组件 `render()` 返回 readonly 的兼容性核查 |
| 0.3.3 | B4 转正：PATH 缓存自愈（缓存+兜底并集、陈旧自动重探测、菜单「重新载入登录环境」） |
| 0.3.3-beta.1 | 测试版：B4 PATH 缓存自愈（缓存+兜底并集、陈旧自动重探测、菜单「重新载入登录环境」） |
| 0.3.2 | 取消 L3（pi-web 随壳固定）+ pi-web 0.15.0 + 运行期每 6 小时检查更新 |
| 0.3.1 | 托盘 + 关窗隐藏、L3 清单独立、退出确认 |
| 0.3.0 | 更新通道转正（正式版 / 测试版） |
| 0.2.11 | 修 F20（输入框横向滚动条压字） |
| 0.2.10 | 修 B3（启动崩溃）+ F19（窗口几何记忆）+ 启动冒烟进 CI |
| 0.2.9 | **已撤回**（B3 启动崩溃）；内容并入 0.2.10 |
| 0.2.8 | 修「完整历史」崩溃（B1） |
| 0.2.7 | L3 进度窗 + 独立入口；每次启动都检查更新 |
| 0.2.6 | **已撤回**（消耗掉当时的 L3 验证场景），内容并入 0.2.7 |
| 0.2.5 | 菜单全中文 + 视图/缩放 |
| 0.2.0–0.2.4 | 首个开源版、签名修复、更新流程重做 |

## 下次继续

当前状态：**正式版 v0.3.3 已发布**；**beta 通道 `v0.3.4-beta.2` 已发布**（pi-web 0.19.0 / SDK 1.6.1 / Node 24.22.3，子龙已在 30341 隔离实例验收通过）。

1. **beta 通道真实升级一遍**：菜单「更新通道 → 测试版 beta」→ 检查更新 → 升到 0.3.4-beta.2
2. 稳定后出 `0.3.4` 正式版（只改版本号）；届时 0.3.4-beta.2 用户自然升级
3. 后续候选：Windows（F4）、下载可取消（F6）、检查中反馈（F7）、失败建议（F8）、缩放档位（F9）、Dock/系统退出确认
4. 每次发版同步：本页「最新正式版 / 版本历史」+ [QA](docs/QA-2026-11-01.md) + [发版清单](docs/RELEASE-CHECKLIST-0.2.0.md)
