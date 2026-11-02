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

## 每次发版的固定动作

```bash
# 1. 改版本：src-tauri/tauri.conf.json、package.json、src-tauri/Cargo.toml
# 2. 升侧车（如需）：改 sidecar.json → npm run sidecar → npm run smoke
# 3. 本地打包验收：npm run dist（在普通终端跑，dmg 需要挂载）
# 4. 发布：git commit → git tag v<ver> → git push origin master v<ver>
#    （CI 构建并发布 Release；本次也验证过"本地产物 + gh release create"这条手动后路）
```
