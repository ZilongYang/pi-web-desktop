# Pi Web Desktop — 路线图 / Backlog

> 这里是「现在到哪了、接下来做什么」。已批准方案的原文在 `docs/PLAN-*.md`（不修改，只在文末追加事后补记）；验收与发版记录在 `docs/` 下另外两份。
> 文档索引见 [docs/README.md](docs/README.md)。

## 一句话

把 [pi-web](https://github.com/agegr/pi-web)（Next.js 服务端 + 浏览器 UI）包成原生桌面 App：**自带 Node 运行时**，双击即用，与 `pi` CLI / 浏览器版共享 `~/.pi/agent`。

- 仓库：<https://github.com/ZilongYang/pi-web-desktop>（MIT）
- 最新版本：**v0.2.5**（内置 pi-web 0.14.1 / pi SDK 1.4.0 / Node 22.23.4）
  - v0.2.6 已**撤回**：它把 pi-web 0.14.2 打进了包，正好消耗掉 L3（pi-web 热更新）的真机验证场景。0.2.7 会带上 F1/F2 再发。
- 平台：macOS 14+ arm64（Windows 代码路径已写、未验证；Linux 未验证）

## 能力矩阵

| 能力 | 状态 | 备注 |
|---|---|---|
| 原生窗口 + 全中文菜单（含视图/缩放） | ✅ 已验证 | 缩放档位 50%–300%，记忆在 `config.json` |
| 侧车管理（启动/探活/优雅退出/崩溃清孤儿） | ✅ 已验证 | 进程组 SIGTERM→SIGKILL；Windows `taskkill /T /F` 待验证 |
| 与 CLI/浏览器共享数据、同版本自动附着 | ✅ 已验证 | 版本不一致时弹窗让用户选 |
| PATH 继承登录 shell（nvm/pnpm/mnemon 可用） | ✅ 已验证 | 缓存 7 天，`inherit_login_shell_path` 可关 |
| 启动页 + 失败态（日志尾部/重试/打开日志） | ✅ 已验证 | |
| 启动即检查更新（应用 + pi-web） | ✅ 已验证 | 每次启动都查；同一版本只提醒一次 |
| L2 应用自更新（Tauri updater + minisign） | ✅ 真机闭环 | 进度窗 + 重启确认（0.2.3 起） |
| L3 pi-web 热更新（只换 pi-web，不动 Node） | 🟡 代码完成 + 演练通过 | 真实"有新版本可装"未走过（等上游发版） |
| 开源发布流水线（ci / release workflow） | ✅ 绿 | macOS arm64；Windows job 未加 |
| Windows 支持 | ❌ 未验证 | 最大缺口 |
| Linux 支持 | ❌ 未做 | 预留 |

## 待办 Backlog

### P0 — 用户明确要求（2026-11-03 记录）

#### F1. 每次启动都静默检查一次更新，有更新就提醒 —— ✅ 代码完成（随 0.2.7 发布）

- **现状**：启动后 20s 会检查应用更新（`update::silent_app_check`），但有 **24 小时节流**（`update-state.json` 的 `lastAppCheckedAt`）；pi-web 的检查（`silent_check`）同样 24h 节流。所以同一天多次重启只会检查一次。
- **目标**：**每次启动**都静默检查一次；发现新版直接提醒（弹「下载并安装 / 稍后」，不自动安装）。24h 节流只用于其他后台轮询（如果需要）。
- **实现**（0.2.6）：
  - `silent_app_check()` / `silent_check()` 去掉 24h 节流，每次启动都查；
  - 新增「同一版本只提醒一次」：`update-state.json` 记 `notified_app_version` / `notified_piweb_version`，避免每次开机弹同一个窗；
  - pi-web 有新版时的文案改为指向「菜单 → 检查更新…」热更新（而不是只建议重建）。
- **验证**：连续两次冷启动，日志两次都有检查记录（实测：两次都输出 `应用已是最新` 与 pi-web 检查结果）。

#### F2. 「检查更新…」菜单放到「关于」后面 —— ✅ 代码完成（随 0.2.7 发布）

- **现状**：检查更新只在「Pi Web」菜单里；macOS 的 App 菜单（Pi Web Desktop）里只有 关于/服务/隐藏/退出。
- **目标**：App 菜单「关于 Pi Web Desktop」下面加一项「检查更新…」（macOS 惯例位置）；「Pi Web」菜单里的入口**保留**（两处都能进）。
- **实现**（0.2.6）：`src-tauri/src/menu.rs` 的 app 子菜单在 `about` 之后插入
  `MenuItemBuilder::with_id("check_update", "检查更新…")`，复用现有 `manual_check_all`。
- **状态**：App 菜单第二项即「检查更新…」；「Pi Web」菜单里也保留入口。

### P1 — 功能验收与平台

#### F3. 功能手测清单（决定能不能日常用）

需要人工点一遍（做不到脚本化）：终端面板（能起 shell、中文正常、找得到命令）、文件预览（源码/Markdown/图片/PDF）、拖拽上传、外部链接跳系统浏览器、会话"新标签页"、导出 HTML 下载、⌘Q 后无残留 node 进程、IME 中文输入。清单见 [docs/QA-2026-11-01.md](docs/QA-2026-11-01.md)。

#### F4. Windows 支持

- 侧车：`fetch-sidecar --target win32-x64`（Node zip + win32 prebuild 已有）
- 进程收尾：`taskkill /PID <pid> /T /F`（已写，未验证）
- 无控制台窗口：`CREATE_NO_WINDOW`（已写，未验证）
- 安装包：NSIS（`bundle.targets` 加 `nsis`）
- 建议先加 Windows CI job 验证"能构建"，再找真机装一次

#### F5. L3 真实演练

等 pi-web 发新版（如 0.15）后，用菜单「检查更新…」走一次"下载并切换 pi-web"；或临时发一个假的 0.14.2 侧车包演练（测完删掉）。

### P2 — 体验打磨

- **F14. L3 清单改位置（重要）**：现在 L3 清单 `piweb-sidecar.json` 挂在"应用 Release"上，有两个坑：① CI 在 tag push 时会用 `--clobber` 重新上传资产，把手工替换的清单冲掉（0.2.5 就发生过）；② GitHub 的 `releases/latest/download/...` 有约 5 分钟的 CDN 缓存，替换资产后 App 可能仍拿到旧清单。方案：清单放进仓库（master 分支的 `sidecar/piweb-sidecar.json`，App 用 raw.githubusercontent 取），或放到独立的、只由侧车流水线更新的 release tag。
- **F12. L3 下载也要有进度**：应用更新（L2）已有进度窗；pi-web 热更新下载 79MB 时只有日志，界面静默十几秒。做法：复用 `ui/update.html`，把进度事件也发给它。
- **F13. 「更新 pi-web…」独立入口**：已加（`Pi Web` 菜单，代码在 0.2.7），避免"应用更新/热更新"混在一个入口里。

- **F6. 下载可取消**：进度窗口加「取消」按钮；updater 下载中断要能清临时文件
- **F7. 检查中反馈**：点「检查更新…」到弹窗之间有 1–2 秒无反馈，加一个「正在检查…」的状态
- **F8. 更新失败给建议**：现在只说"失败"，加「检查网络/代理」「打开日志」按钮
- **F9. 缩放档位可调**：现在固定 12 档（50%→300%），可按用户偏好改成 5% 步进或自定义
- **F10. 托盘常驻 / 关窗不退出**：pi-web 有后台任务时可能想要
- **F11. Developer ID + 公证**：消掉首次"右键打开"这一步（需要 $99/年证书）

## 版本历史

| 版本 | 内容 |
|---|---|
| 0.2.6 | 启动即检查更新（同一版本只提醒一次）；「关于」下方加「检查更新…」；内置 pi-web 0.14.2 / pi SDK 1.5.0 |
| 0.2.5 | 菜单全中文；新增「视图」菜单（放大 ⌘= / 缩小 ⌘- / 实际大小 ⌘0 / 全屏）；缩放记忆；复制服务地址进剪贴板；帮助菜单加 GitHub |
| 0.2.4 | 验证用版本（与 0.2.3 同功能），用于真机验证更新流程 |
| 0.2.3 | 更新体验重做：下载进度窗（实时 MB/%）+ 安装后询问重启；修进度累计 bug |
| 0.2.2 | 修「检查更新…」菜单只查 pi-web 的 bug（改走完整编排） |
| 0.2.1 | 修整包签名（下载后不再报「已损坏」）；L2 链路验证版本 |
| 0.2.0 | 首个开源版：L1/L2/L3、开源发布流水线、内置 pi-web 0.14.1 |
