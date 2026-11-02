# Pi Web Desktop — 路线图 / Backlog

> 这里是「现在到哪了、接下来做什么」。已批准方案的原文在 `docs/PLAN-*.md`（不修改，只在文末追加事后补记）；验收与发版记录在 `docs/` 下另外两份。
> 文档索引见 [docs/README.md](docs/README.md)。

## 一句话

把 [pi-web](https://github.com/agegr/pi-web)（Next.js 服务端 + 浏览器 UI）包成原生桌面 App：**自带 Node 运行时**，双击即用，与 `pi` CLI / 浏览器版共享 `~/.pi/agent`。

- 仓库：<https://github.com/ZilongYang/pi-web-desktop>（MIT）
- 最新版本：**v0.2.8**（内置 pi-web 0.14.2 / pi SDK 1.5.0 / Node 22.23.4）
  - v0.2.6 已撤回（它把 0.14.2 打进包，消耗掉当时的 L3 验证场景），其内容并入 0.2.7。
  - v0.2.8 修复「完整历史」崩溃（B1）。**本机已装 0.2.8，线上 latest 也是 0.2.8。**
- 平台：macOS 14+ arm64（Windows 代码路径已写、未验证；Linux 未验证）

## 下次继续（2026-11-03 下午）

当前状态：**v0.2.8 已发布、本机已安装**；L2/L3 都真机闭环；只剩一个上游引起的导出问题和一个待定方案。

1. **先定 B2 方案**（导出 HTML 报 `sortChildren expected 1 match` 那个）：
   - 选项：A 内置降级 pi-web 0.14.1/SDK 1.4.0；B 给导出路由打"容忍 0 匹配"补丁；C 等上游；D 临时走 CLI 导出。
   - 倾向：**B 为长期方案**（先验证 SDK 1.5.0 模板已是迭代实现），A 可作立即止血。
2. **顺手做 F18**（smoke 加导出接口断言）—— 以后这类"首页正常但某接口 500"能在发版前发现。
3. 然后按用户 2026-11-03 的新需求排期：**F15 关于面板显示 pi-web 版本**、**F16 菜单栏图标 + 菜单**（F16 需先定关窗行为：A 保持关窗即退出 / B 关窗隐藏到菜单栏）。
4. 再往后：F17 菜单文案区分、F14 L3 清单位置（CI 覆盖 + CDN 缓存）、Windows 支持（F4）。

> 恢复上下文只需要读这一节 + 「缺陷修复记录」+ 最近一条 `git log`。

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
| L3 pi-web 热更新（只换 pi-web，不动 Node） | ✅ **真机闭环** | 0.2.5 上真实从 0.14.1 热更到 0.14.2；进度窗/回滚已实拍验证 |
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

#### F15. 「关于」里显示 pi-web 版本（2026-11-03 记录）

- **现状**：菜单 `Pi Web Desktop → 关于` 只有 App 名称 + 版本（`menu.rs` 里的 `AboutMetadata { name, version }`），看不到它内置的 pi-web / pi SDK / Node 版本，排障时要翻日志或看文件。
- **目标**：关于面板里显示 **`Pi Web Desktop 0.2.x` / `pi-web 0.14.x` / `pi SDK 1.5.x` / `Node 22.23.x`**。
- **落点**：`src-tauri/src/menu.rs` 的 `AboutMetadata` 加 `credits: Some("内置 pi-web 0.14.2 · pi SDK 1.5.0 · Node 22.23.4")`（macOS 上 `comments`/`authors` 不支持，`credits` 支持）；数据直接复用 `crate::update::bundled_versions(app)`。
- **备选**：自定义「关于」窗口（本地页面 `ui/about.html`），能放更多信息（版本列表 + GitHub/日志/配置文件链接），但要新增页面与窗口。
- **验收**：关于面板能看到四个版本号。

#### F16. 菜单栏图标 + 相关菜单（2026-11-03 记录）

- **现状**：只有 Dock 图标，没有菜单栏（状态栏）图标；关窗即退出。
- **目标**：菜单栏常驻小图标，点击弹出菜单：显示/隐藏主窗口 · 在浏览器中打开 · 检查更新… · 更新 pi-web… · 打开日志目录 · 退出；图标用模板图（`icon_as_template(true)`）以适配深浅色菜单栏。
- **落点**：`src-tauri/Cargo.toml` 给 tauri 加 `tray-icon` feature；`main.rs` 用 `TrayIconBuilder` + 复用 `menu.rs` 里已有的动作函数；图标复用 `icons/32x32.png`（或做一张 18×18 模板图）。
- **需要先定**：关窗行为是否改成「隐藏到菜单栏、不退出」（现在 `RunEvent::ExitRequested` 直接停侧车退出，pi-web 有后台任务时可能要保留）。
- **验收**：菜单栏出现图标；点击能显示/隐藏窗口；菜单项与主菜单行为一致；退出时侧车被清理。

#### F17. 菜单文案区分「应用更新」与「pi-web 更新」（2026-11-03 记录）

- **现状**：菜单里有「检查更新…」和「更新 pi-web…」，用户容易混淆（"检查更新"其实会先查 App、跳过 App 更新后再查 pi-web）。
- **目标**：改成「检查应用更新…」+「更新 pi-web…」，或在「检查更新…」的弹窗里先说明"将检查 App 与 pi-web 两项"。
- **落点**：`src-tauri/src/menu.rs`（文案）与 `update.rs`（弹窗首句）。

### 缺陷修复记录

#### B1. 点击「完整历史」导致 App 崩溃 —— ✅ 0.2.8 修复

- **现象**：点 pi-web 侧栏的「完整历史」，App 直接退出，macOS 弹「意外退出」。
- **崩溃报告**：`~/Library/Logs/DiagnosticReports/pi-web-desktop-2026-11-03-030447.ips`
  - `EXC_CRASH / SIGABRT`，崩溃线程是主线程
  - 关键栈：`WebKit::UIDelegate::UIClient::createNewPage` → `+[NSException raise:format:]` → `abort`
- **根因**：pi-web 的「完整历史」用 `window.open` 开新窗口，走到我们的 `on_new_window`；我们创建新窗口时**没有沿用发起方 webview 的 configuration**。Tauri 文档明确：macOS 上 `NewWindowResponse::Create` 的 webview 必须与调用方使用同一 configuration（`WindowFeatures.webview_configuration`），否则 WebKit 直接抛 NSException，进程 abort。
- **修复**（0.2.8）：`new_window_for()` 增加 `window_features(features)`，`on_new_window` 把 WebKit 传来的 `features` 透传。同一路径也覆盖「会话 ⋯ → 新标签页打开」。
- **待用户复验**：0.2.8 上点「完整历史」与「新标签页打开会话」都应正常开新窗口。

#### B2. 导出会话 HTML 报错 `Failed to patch exported HTML: sortChildren expected 1 match, found 0` —— 🟡 已定位，暂不修（2026-11-03 记录）

- **现象**：会话「导出」打开的窗口里显示 JSON 错误（`?inline=1` 返回错误 JSON，不是 HTML）。
- **注**：新窗口本身正常打开了 —— 说明 0.2.8 的崩溃修复生效（B1），这是服务端返回的内容问题。
- **根因（上游版本组合不兼容，与本壳无关）**：
  1. pi-web 0.14.2 的导出路由（`.next/server/app/api/sessions/[id]/export/route.js`）在生成 HTML 后做**字符串替换补丁**：把 `sortChildren` / `mapNodes` / `markActive` 三个递归函数替换成迭代版本（防深会话爆栈）。每个补丁要求**恰好 1 处匹配**，0 处就 `throw`。
  2. 内置 **pi SDK 1.5.0** 的 `dist/core/export-html/template.js` 已经没有这三个函数（改成迭代遍历，`stack` 出现 19 次）。
  3. 于是补丁匹配 0 处 → 抛出该错误。对照：用户 npx 那套 **pi-web 0.14.1 + SDK 1.4.0** 的 template.js 里三个函数都在（各 3 处），所以导出正常。
- **影响**：仅「导出 HTML」；其它功能正常。
- **候选方案（待定，按用户要求暂不发版）**：
  - **A. 内置降级到 pi-web 0.14.1 / pi SDK 1.4.0**（导出恢复，最稳；丢 0.14.2 的更新）
  - **B. 在 `fetch-sidecar` 阶段给 export 路由打补丁**：把"必须 1 处匹配"改成"0 处跳过、>1 才报错"（SDK 1.5.0 模板已是迭代实现，跳过即正确）。同类做法已有先例（`patch-pi-web-collapse.mjs`）
  - **C. 等上游修**（pi-web 0.14.3 / SDK 1.5.1 大概率跟进），期间把内置钉在可用组合
  - **D. 临时绕过**：命令行 `pi --export <session.jsonl> out.html`（不经过 pi-web 的补丁）
- **跟进**：smoke 目前只查 `/` 与 `/api/sessions`；建议加一条「导出接口返回 HTML」的断言，这类上游不兼容就能在发版前发现（记为 F18）。

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
- **F12. L3 下载也要有进度** —— ✅ 已在 0.2.7 完成：抽出 `src-tauri/src/progress.rs`（L2/L3 共用 `ui/update.html`），L3 下载轮询文件大小播报 MB/百分比，安装/回滚都有状态。
- **F13. 「更新 pi-web…」独立入口** —— ✅ 已在 0.2.7 完成（`Pi Web → 更新 pi-web…`）；「检查更新…」在应用更新被跳过时会继续检查 pi-web。

- **F6. 下载可取消**：进度窗口加「取消」按钮；updater 下载中断要能清临时文件
- **F7. 检查中反馈**：点「检查更新…」到弹窗之间有 1–2 秒无反馈，加一个「正在检查…」的状态
- **F8. 更新失败给建议**：现在只说"失败"，加「检查网络/代理」「打开日志」按钮
- **F9. 缩放档位可调**：现在固定 12 档（50%→300%），可按用户偏好改成 5% 步进或自定义
- **F10. 托盘常驻 / 关窗不退出**：pi-web 有后台任务时可能想要
- **F11. Developer ID + 公证**：消掉首次"右键打开"这一步（需要 $99/年证书）

## 版本历史

| 版本 | 内容 |
|---|---|
| 0.2.8 | 修复点击「完整历史」崩溃（新窗口必须沿用发起方 webview configuration） |
| 0.2.6 | **已撤回**（把 pi-web 0.14.2 打进包，消耗掉当时的 L3 验证场景；内容并入 0.2.7） |
| 0.2.7 | L3 热更新进度窗（与 L2 一致）；「更新 pi-web…」独立入口；每次启动都检查更新（同版本只提醒一次）；「关于」下方加检查更新；内置 pi-web 0.14.2 / pi SDK 1.5.0 |
| 0.2.5 | 菜单全中文；新增「视图」菜单（放大 ⌘= / 缩小 ⌘- / 实际大小 ⌘0 / 全屏）；缩放记忆；复制服务地址进剪贴板；帮助菜单加 GitHub |
| 0.2.4 | 验证用版本（与 0.2.3 同功能），用于真机验证更新流程 |
| 0.2.3 | 更新体验重做：下载进度窗（实时 MB/%）+ 安装后询问重启；修进度累计 bug |
| 0.2.2 | 修「检查更新…」菜单只查 pi-web 的 bug（改走完整编排） |
| 0.2.1 | 修整包签名（下载后不再报「已损坏」）；L2 链路验证版本 |
| 0.2.0 | 首个开源版：L1/L2/L3、开源发布流水线、内置 pi-web 0.14.1 |
