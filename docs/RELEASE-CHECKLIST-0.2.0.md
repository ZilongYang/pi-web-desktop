# 发版清单 —— 0.2.0 / 0.2.1（2026-11-02）

> 按「每次发版/执行另存打勾清单」的规矩存档。0.2.0 是首个开源版；0.2.1 是签名修复 + L2 链路验证版。

## 0.2.0（首个开源版本）

- [x] 仓库开源：`ZilongYang/pi-web-desktop`（public，master）
- [x] 签名方式仿 zWGestures：发布产物 ad-hoc；本机日常构建用固定自签证书（`scripts/create-signing-cert.sh`）
- [x] `scripts/make-dist.sh`：ad-hoc 断言、`diskutil`/`hdiutil` 自动回退出 dmg、安全断言、L3 侧车包与清单、SHA256SUMS
- [x] L1 检测（npm registry，24h 静默 + 菜单手动）
- [x] L2 应用自更新（tauri-plugin-updater，minisign 公钥进 conf，私钥进 GitHub Secrets）
- [x] L3 pi-web 热更新（GitHub Release 清单 → SHA256 → 解压 → 原子切换 → 双条件探活 → 失败回滚）
- [x] 内置 pi-web 0.13.0 → **0.14.1**；pi SDK 1.4.0；Node 22.23.4
- [x] Release 资产：dmg / zip（版本化 + 固定名）/ updater 包 + sig / latest.json / 侧车包 + 清单 / SHA256SUMS
- [x] `.github/workflows/ci.yml`（master：sidecar + smoke + cargo test）—— 首次跑**绿**
- [x] `.github/workflows/release.yml`（tag `v*`：构建 + 发布）
- [ ] release.yml 首次全绿（首跑挂在 dmg 的 `diskutil --volumeName`，已改成自动回退 hdiutil，待 v0.2.1 那轮确认）
- [ ] Developer ID + 公证（未做；用户选择 ad-hoc，首次打开需右键放行）

## 0.2.1（签名修复 + L2 验证）

- [x] 修「下载后已损坏，无法打开」：Tauri 在无签名身份时只做 linker 签名，资源封印缺失；改为构建时 `APPLE_SIGNING_IDENTITY="-"` 整包 ad-hoc 签名
- [x] `make-dist.sh` 增加 `codesign --verify --deep --strict` 硬断言（防止再发出坏包）
- [x] 重新构建并覆盖 v0.2.0 资产；线上 dmg sha256 与本地一致；挂载验收 `valid on disk` + `satisfies its Designated Requirement`
- [x] **L2 端到端实测**：已安装的 0.2.0 检测到 0.2.1 → 下载 → 安装 → 重启，`/Applications` 版本变为 0.2.1，重启后 `应用已是最新`
- [x] 远端 tag `v0.2.1` 与 Release 对齐，触发 release workflow 复验
- [ ] Windows 侧（NSIS + `taskkill /T /F` 路径）未验证 —— 见 README「平台」

## 0.2.3（更新体验重做）

用户实测反馈："点下载并安装没有进度、App 突然关闭再启动，整个流程很差。"

- [x] 新增进度窗口 `ui/update.html`：标题栏「Pi Web Desktop 更新」+「正在下载 Pi Web Desktop vX」+ 进度条 + 已下载/总大小 + 百分比
- [x] 修进度 bug：updater 回调第一个参数是**本次 chunk 长度**而不是累计值（`on_chunk(chunk.len(), content_length)`），之前进度一直 0MB；改为累加
- [x] 安装完成不再直接 `app.restart()`：弹「新版本已就绪，重启后生效」[立即重启 / 稍后]；选稍后则下次启动生效
- [x] 进度窗 `always_on_top`，capabilities 增加 `update-progress` 窗口
- [x] 实拍验证：本地假 Release（限速 12MB/s）→ 进度窗实时显示 40/60/80/100MB 与百分比（截图 docs/screenshot-update-progress.png）
- 注意：0.2.2 → 0.2.3 这次升级仍走旧体验（进度窗代码在 0.2.3 里），从 0.2.3 起生效

## 0.2.4（验证更新流程，已通过）

- [x] 发布 0.2.4（与 0.2.3 功能相同，只改版本号），线上 `latest.json` = 0.2.4
- [x] **用户在真机 0.2.3 上完整验证通过**：菜单「检查更新…」→ 提示 v0.2.4 →「下载并安装」→ 进度窗实时 20/40/60/80/100/120MB → 「安装完成，等待用户确认重启」→ 点「立即重启」→ 启动 0.2.4
- [x] 日志证据：`开始下载应用更新 v0.2.4` / `下载中 …/120MB` / `安装完成，等待用户确认重启` / `用户选择立即重启` / `启动 Pi Web Desktop 0.2.4`
- [x] 升级后无残留：无 overlay（仍用包内 pi-web 0.14.1）、无 staging、无孤儿 node 进程

结论：L2（应用自更新）在真机上闭环验证完成。

## 0.2.5（菜单中文化 + 视图/缩放）

- [x] 自建完整菜单替代 `Menu::default()`：全中文（Pi Web Desktop / 文件 / 编辑 / **视图** / 窗口 / Pi Web / 帮助）
- [x] 新增「视图」：放大 ⌘=、缩小 ⌘-、实际大小 ⌘0、进入全屏；档位 50%–300%，写入 `config.zoom`，重启保持
- [x] 实拍验证：菜单栏中文化（`docs/screenshot-menu-chinese.png`）；`zoom=1.0` 与 `zoom=2.0` 同区域截图对比，页面确实放大
- [x] 顺带修：「复制服务地址」从"只写日志"改成真正 pbcopy；帮助菜单加 GitHub 仓库入口
- [x] 发布 v0.2.5（线上 latest.json 已更新）

## 0.2.6（F1 + F2，内置 pi-web 0.14.2）

- [x] **F1**：`silent_app_check` / `silent_check` 去掉 24h 节流 → 每次启动都检查；新增「同一版本只提醒一次」（`update-state.json` 记 `notified_*_version`）
- [x] **F2**：App 菜单「关于 Pi Web Desktop」下面新增「检查更新…」，与 Pi Web 菜单入口共用 `manual_check_all`
- [x] pi-web 有新版时的文案改为指向「菜单 → 检查更新…」热更新
- [x] 内置 pi-web 0.14.1 → **0.14.2**（pi SDK 1.5.0）；`npm run smoke` 通过
- [x] 实测：连续两次冷启动，两次都执行检查（日志有 `应用已是最新` 与 pi-web 检查结果）
- [x] 发布 v0.2.6（含 L3 侧车包 0.14.2 + `piweb-sidecar.json`）

## 0.2.6（已撤回）

- 内容：F1 + F2 + 内置 pi-web 升到 0.14.2
- **撤回原因**：把 0.14.2 打进包，等于把用户想验证的 L3 场景（0.14.1 → 0.14.2 热更新）消耗掉。Release 与 tag 已删除。
- 处置：`releases/latest` 回到 v0.2.5；把 **0.14.2 的侧车包与清单挂到 v0.2.5 release**，这样 0.2.5 用户点「检查更新…」时：应用已是最新 → 继续检查 pi-web → 可热更新到 0.14.2（真机验证 L3）。
- 代码状态：F1/F2 的改动保留在 master，随 0.2.7 发布。

## 0.2.7（L3 进度窗 + 独立入口 + F1/F2）

- [x] 抽出 `src-tauri/src/progress.rs`：L2/L3 共用进度窗口（`ui/update.html`）
- [x] **L3 下载有进度**：curl 子进程 + 轮询文件大小，实时 MB/百分比；安装/成功/失败/回滚都有状态文案（实拍：`docs/screenshot-l3-progress.png`）
- [x] 新增菜单「Pi Web → 更新 pi-web…」直达热更新；「检查更新…」在跳过应用更新后继续检查 pi-web
- [x] F1：每次启动都检查（同版本只提醒一次）；F2：App 菜单「关于」下方也有「检查更新…」
- [x] 内置 pi-web 0.14.2 / pi SDK 1.5.0
- [x] 发布 v0.2.7

## 0.2.8（修复「完整历史」崩溃）

- [x] 崩溃报告定位：`WebKit::UIDelegate::UIClient::createNewPage` → NSException → SIGABRT（主线程）
- [x] 根因：新窗口没沿用发起方 webview 的 configuration（Tauri 文档对 macOS 的硬性要求）
- [x] 修复：`new_window_for()` 加 `window_features(features)`，`on_new_window` 透传
- [x] 发布 v0.2.8；待用户复验「完整历史」与「新标签页打开会话」

## 0.2.9（B2 导出修复 + F18 + F15 + F17）—— ⚠️ 已撤回（启动崩溃）

- [x] **B2 走方案 B**：新增 `scripts/patch-pi-web-export.mjs`，把导出路由里的 `if(1!==g)throw …` 改成 `if(g>1)throw …`（0 匹配即跳过 —— SDK 1.5.0 模板已是迭代实现）；幂等 + `--check` + 锚点对不上就报错退出
- [x] 接进 `scripts/fetch-sidecar.mjs`（安装后 + 跳过前校验两处，受 `--no-patch` 控制）→ CI 的 `npm run sidecar` 也会带上
- [x] **F18**：`scripts/smoke.mjs` 写最小 fixture 会话，断言 `/api/sessions` 能看到它 + `export?inline=1` 返回 200 `text/html`
- [x] **F15**：`menu.rs` 的 AboutMetadata 加 credits →「内置 pi-web 0.14.2 · pi SDK 1.5.0 · Node 22.23.4」（3 个单元测试）
- [x] **F17**：菜单「检查更新…」→「检查应用更新…」；应用更新弹窗说明"这一步只处理应用本身"；pi-web 结果弹窗标题改「检查 pi-web 更新」
- [x] 本地验证：patch 前 smoke 红（`HTTP 500 … sortChildren expected 1 match, found 0`）→ patch 后绿（导出 278.9 KB）；`cargo test` 12 项通过；`tauri build --bundles app` 成功
- [x] 打包产物复验：在 `.app/Contents/Resources/pi-web` 上直接起侧车，导出接口 200 + `text/html`（285.5 KB）
- [x] `make-dist.sh` 校验链全绿：签名 adhoc / 资源封印 `--verify --deep --strict` / arm64 / 版本 0.2.9 / 无调试 entitlement / 无构建者路径泄漏 / 侧车可执行位；zip 生成成功
- [x] 产物二进制里核对 F15/F17 文案已编入（`内置 pi-web`、`检查应用更新` 等 UTF-8 字节）
- [ ] 本地 dmg 未生成：受限沙箱里 `hdiutil` 挂载失败（OSStatus error 1），dmg 由 CI（macos-14）生成；非沙箱终端可跑 `npm run dist` 复验
- [x] 发布 v0.2.9：`git tag v0.2.9` + push → ci 绿（含新的导出断言）、release 绿（构建 + dmg + updater 包 + Release 资产齐全）
- [x] 发布侧核验：`latest.json` 版本 0.2.9 + minisign 签名；`SHA256SUMS` 四件套齐全；`piweb-sidecar.json` 指向 0.14.2 侧车包（同版本，L3 不会误弹）
- [ ] 用户真机复验：关于面板能看到三个内置版本号；菜单文案为「检查应用更新…」；pi-web 里点「导出」得到 HTML 而不是 JSON 错误
- [x] **撤回原因**：用户升级后**双击启动即崩**（无窗口）。终端复现：
  `panicked at tauri-2.12.5/src/lib.rs:761: state() called before manage() for tauri_utils::Env`
  —— F15 在 `.menu(|app| ...)` 回调里读内置版本 → `app.path()`，而该回调早于 setup，path state 未初始化。见 ROADMAP B3。
- [x] 处置：v0.2.9 的 Release/tag 已按 0.2.6 先例撤回（`gh release delete --cleanup-tag`，本地/远端 tag 都已删）；修复随 0.2.10 发布

## 0.2.10（修 0.2.9 启动崩溃 + F19 窗口记忆 + 启动冒烟）

- [x] **B3 修复**：删掉 `Builder::menu(|app| …)`，改在 `setup()` 里 `app.set_menu(menu::build(&handle)?)`（并注释写明原因）
- [x] **F19**：`config.json` 新增 `window {x,y,width,height}`（物理像素）；`window_state.rs` 在移动/缩放（800ms 节流）与关窗/退出时保存，启动时 show 前恢复；拔屏/换屏后不在任何显示器内 → 居中；最大化/全屏不覆盖普通态
- [x] **新增 App 启动冒烟** `scripts/smoke-app.mjs`（`npm run smoke:app`）：临时 `PIWEB_CONFIG_DIR` 隔离 + `PIWEB_WINDOW_PROBE=1` 探针，断言「退出码 0 / 无 panic / 有启动行 / 几何能落盘 / 预置几何能恢复」；**已接进 `release.yml`**（构建后、打包前）
- [x] 本地真机验证：`npm run smoke:app` 通过（几何落盘 2560×1720 @ (1024,466) → 物理像素；预置 1000×680 @ (120,140) 恢复成功）
- [x] `cargo test` 12 项通过；`tauri build --bundles app`（`APPLE_SIGNING_IDENTITY=-`）成功
- [x] 发布 v0.2.10：release 流水线绿 —— 其中**「启动冒烟（App 真的能起来）」步骤真跑并通过**（新增的防线生效）
- [x] 撤回 v0.2.9（Release + tag 全删）；`latest.json` 已指向 0.2.10（minisign 签名正常）
- [x] 安装恢复：0.2.10 覆盖安装到 `/Applications`，`codesign --verify --deep --strict` 通过（`valid on disk` / `satisfies its Designated Requirement`），启动正常（`启动 Pi Web Desktop 0.2.10` + `服务就绪用时 1168ms`）
- [ ] 用户真机复验：启动正常；拖动/缩放窗口后重启回到原位；关于面板三个内置版本号；菜单「检查应用更新…」；pi-web「导出」得到 HTML

## 0.2.11（修 F20 输入框横向滚动条）

- [x] 真机定位：注入诊断脚本，确认只有输入框 `<textarea>` 横向溢出（`sw=817 cw=545`），并用对照组排除窗口尺寸因素（100% 正常 / 150% 复现）
- [x] 修复：`main.rs` 增加 `PIWEB_CSS_FIX` 初始化脚本（`.chat-input-textarea{overflow-x:hidden!important}`），主窗口与新开窗口都挂，随每次导航生效
- [x] 真机验证：缩放 110% 与 150% 截图对照，滚动条消失、占位文字完整（`docs/screenshot-composer-scrollbar-{before,after}.png`）
- [x] `cargo test` 12 项通过；`npm run smoke:app`、`npm run smoke` 均通过；安装包签名封印有效（`valid on disk` / `satisfies its Designated Requirement`）
- [x] 0.2.11 已安装到 `/Applications`，缩放已还原为你原来的 110%
- [ ] 发布 v0.2.11（等确认）

## 0.3.0-beta.1（更新通道：正式版 / 测试版）

- [x] 方案存档：`docs/PLAN-2026-11-03-channels.md`（四项决策：只有 stable+beta；允许降级重装；pi-web 不分通道；先发 0.2.11）
- [x] 配置：`config.json` 新增 `update_channel`（默认 `stable`）
- [x] 菜单：`Pi Web → 更新通道` 两个勾选项（正式版 / 测试版 beta）
- [x] 运行时按通道选 endpoint（`UpdaterBuilder::endpoints`）；降级用 `version_comparator` 绕过「只升不降」
- [x] `compare_versions` 改用 semver（预发布顺序：0.3.0-beta.2 < 0.3.0 < 0.3.1）
- [x] CI：版本号带 `-` → 发成 pre-release + 更新固定 `channel-beta` Release 的 `latest.json`
- [x] 单测：通道归一化 / beta endpoint / 降级识别（15 项全过）
- [x] 通道探针 `PIWEB_CHANNEL_PROBE=beta|stable`（只查不装，写日志后退出）用于真机验证端点选择
- [x] 发布 v0.3.0-beta.1（pre-release）+ 固定 `channel-beta` 清单（CI 自动）
- [x] 真机验证：beta 清单可读（探针 `已是最新`）；正式版清单未被 beta 抢走（仍是 0.2.11）；跨通道降级识别正确（探针 `发现 v0.2.11（当前 v0.3.0-beta.1）-> 降级`）
- [x] 真机验证升级链路：v0.3.0-beta.2（pre-release）→ `channel-beta` 清单更新为 beta.2 → 通道=beta 时检出 `v0.3.0-beta.2（当前 beta.1）-> 升级`
- [ ] 用户在界面上点一次「切换到测试版」验证弹窗/下载/安装/重启（当前装的是 0.3.0-beta.1，切过去会提示 beta.2）
- [ ] beta 稳定几天后发 v0.3.0 正式版

## 0.3.0（更新通道转正）

- [x] 由 `0.3.0-beta.2` 转正：版本号去掉 `-beta.2`（四处：package.json / package-lock.json / Cargo.toml / tauri.conf.json）
- [x] 发版前三件套：`cargo test` 15 项通过、`npm run smoke` 通过、`npm run smoke:app` 通过（启动行 0.3.0 / 窗口几何存取）
- [x] 构建 + ad-hoc 整包签名校验：`valid on disk` / `satisfies its Designated Requirement`
- [x] 发布 v0.3.0：CI 绿（ci 1m12s / release 8m59s）；Release 为 **Latest**（非预发布）
- [x] 发布后核验：`releases/latest` = **v0.3.0**；`latest.json` version=0.3.0；`channel-beta` 仍指向 0.3.0-beta.2（符合设计）；本机安装 0.3.0、通道=正式版，启动日志 `应用已是最新`

## 0.3.1-beta.1（F16 托盘 + F14 侧车清单独立）

- [x] 方案存档：`docs/PLAN-2026-11-03-tray.md`、`docs/PLAN-2026-11-03-sidecar-manifest.md`
- [x] **F16**：tauri 特性加 `tray-icon`/`image-png`；新增 `src-tauri/src/tray.rs`（π 模板图标、托盘菜单复用主菜单动作）；主窗口 `CloseRequested` → `prevent_close()` + `hide()`；`RunEvent::Reopen` → 唤回窗口；`PIWEB_TRAY_PROBE` 探针
- [x] **F14**：`hotupdate.rs` 默认清单地址改为 `releases/download/piweb-sidecar/piweb-sidecar.json`；`make-dist.sh` 包 URL 同步；`release.yml` 新增「发布侧车清单到固定 Release `piweb-sidecar`（pre-release）」；应用 Release 仍保留一份清单兼容 ≤0.3.0
- [x] F14 端到端演练（本地假 0.14.3 侧车包 + 本地 HTTP 清单 + `PIWEB_SIDECAR_MANIFEST_URL`）：清单读取 → 下载 79MB → SHA256 → 切换（备份 .prev）→ 侧车重启 → 删备份 → 提示已更新；演练后删 overlay 恢复内置 0.14.2
- [x] F16 关窗探针：`[window] 关闭按钮：已隐藏到菜单栏` + `[tray-probe] close() 之后 visible=false`，进程仍存活
- [x] 菜单栏图标真机截图：`docs/screenshot-tray-icon.png`
- [x] 发版前三件套：`cargo test` / `npm run smoke` / `npm run smoke:app`
- [x] 发布 v0.3.1-beta.1（Pre-release，ci 2m32s / release 8m49s 双绿）
- [ ] 真机手动确认：点关闭 → 窗口隐藏 App 不退出；托盘「显示/隐藏主窗口」「退出」；Dock 图标唤回
- [x] 发布后确认：固定 Release `piweb-sidecar` 存在（Pre-release），含侧车包 + 清单，清单 URL 为 `releases/download/piweb-sidecar/...`；默认地址真机验证通过

## 0.3.1-beta.2（修托盘菜单双触发）

- [x] 现象：0.3.1-beta.1 上点「检查应用更新…」/「更新 pi-web…」会弹两个窗（日志同毫秒两条）
- [x] 根因：托盘菜单也注册了 `TrayIconBuilder::on_menu_event`，而托盘菜单点击同时会走到 App 的全局菜单监听（`Builder::on_menu_event`）→ 同一个动作执行两遍
- [x] 修复：`tray.rs` 不再注册托盘级监听；托盘专有 id（`tray_toggle` / `tray_quit`）统一在 `menu::on_event` 处理
- [x] 真机复验：点击后日志只剩一条「手动检查更新」
- [x] **F22 退出确认**（同一版）：任何退出（⌘Q / 菜单 / 托盘）都先弹确认框；有会话在跑时提示「会中断 N 个会话」；
      取消则继续运行。`cargo test` 覆盖 `runningSessionIds` 解析；更新后自动重启（RESTART_EXIT_CODE）直接放行
- [x] 发布 v0.3.1-beta.2（Pre-release，ci 绿 / release 绿；`channel-beta` 清单 → 0.3.1-beta.2；正式版通道仍 0.3.0）
- [x] 期间修了一个测试本身的坑：smoke-app 的副本 App 放在 temp 里会被 macOS 拒绝访问自身资源
      （`sandbox_extension_issue_file_to_process`），导致「假通过」且失败弹窗跳到屏幕上。
      已改为复制到 `~/Applications/Pi-Web-Desktop-Smoke.app`，并**新增断言：侧车真的在监听端口（HTTP 可连）**
- [x] 用户本机验收通过（托盘/关窗隐藏/⌘Q 确认/退出确认/检查更新单弹窗）

## 0.3.1（正式版：托盘 / 侧车清单独立 / 退出确认）

- [x] 由 `0.3.1-beta.2` 转正（代码不变，仅去掉预发布后缀；四处版本号）
- [x] 发版前三件套：`cargo test` 17 项、`npm run smoke`、`npm run smoke:app`（这次冒烟真验证了侧车端口可连）
- [x] 构建 + ad-hoc 整包签名校验通过；本机安装 0.3.1 并运行（`GET / → 200`）
- [x] 发布 v0.3.1：ci + release 双绿；Release = **Latest**；`latest.json` → 0.3.1；`channel-beta` 仍 0.3.1-beta.2
- [x] 发布后核验：线上 `releases/latest/download/latest.json` = 0.3.1

## 0.3.2-beta.1（取消 L3 + pi-web 0.15.0 + 定期检查）

- [x] 方案存档：`docs/PLAN-2026-11-04-drop-l3.md`（决策：pi-web 随壳固定，取消 L3）
- [x] 删除 `hotupdate.rs`；`sidecar::resolve` 去掉 overlay；启动时清理历史 overlay（`sidecar::cleanup_overlay`）
- [x] 删除 L1 npm 上游检查、菜单/托盘「更新 pi-web…」、CI 侧车清单发布步骤、make-dist 侧车包产物
- [x] pi-web 升到 **0.15.0**（SDK 仍 1.5.0 / Node 22.23.4）：折叠补丁已应用；**导出补丁判定"上游已不再使用字符串补丁"，自动跳过**（等于上游修好了 B2）
- [x] `npm run smoke` 通过（导出 HTML 278.9 KB）
- [x] 新增运行期定期检查：启动查一次 + 每 6 小时查（`PIWEB_PERIODIC_CHECK_SECS` 可覆盖）
- [x] `cargo test` / `npm run smoke:app`；本机验收通过（导出、折叠、托盘、⌘Q 确认、定期检查日志、手动检查反馈）
- [x] 发布 v0.3.2-beta.1（Pre-release；`channel-beta` → 0.3.2-beta.1；正式版通道仍 0.3.1）

## 0.3.2（正式版：取消 L3 / pi-web 0.15.0 / 定期检查）

- [x] 由 `0.3.2-beta.1` 转正（仅版本号变化）；`cargo test` 14 项、`smoke`、`smoke:app` 全过
- [x] 本机安装 0.3.2 并运行（`GET / → 200`），签名封印有效
- [x] 发布 v0.3.2：ci + release 双绿；Release = **Latest**；`latest.json` → 0.3.2；beta 通道仍 0.3.2-beta.1
- [x] 发布资产核对：dmg / zip / updater 包 + sig / latest.json / SHA256SUMS（**不再有侧车包**）

## 每次发版的固定动作

```bash
# 1. 改版本：src-tauri/tauri.conf.json、package.json、src-tauri/Cargo.toml
# 2. 升侧车（如需）：改 sidecar.json → npm run sidecar → npm run smoke
# 3. 本地打包验收：npm run dist（在普通终端跑，dmg 需要挂载）
# 4. 发布：git commit → git tag v<ver> → git push origin master v<ver>
#    （CI 构建并发布 Release；本次也验证过"本地产物 + gh release create"这条手动后路）
```
