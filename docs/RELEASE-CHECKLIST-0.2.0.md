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

## 每次发版的固定动作

```bash
# 1. 改版本：src-tauri/tauri.conf.json、package.json、src-tauri/Cargo.toml
# 2. 升侧车（如需）：改 sidecar.json → npm run sidecar → npm run smoke
# 3. 本地打包验收：npm run dist（在普通终端跑，dmg 需要挂载）
# 4. 发布：git commit → git tag v<ver> → git push origin master v<ver>
#    （CI 构建并发布 Release；本次也验证过"本地产物 + gh release create"这条手动后路）
```
