# PLAN-2026-11-03 更新通道（正式版 / 测试版 beta）

> 存档规则：批准时原文存档、不修改；实施后如有偏差，只在文末「事后补记」追加。
> 决策时间：2026-11-03 17:05（用户逐项拍板）。

## 一、背景

现在的更新只有一条线：
- **应用更新（L2）**：Tauri updater，端点写死在 `tauri.conf.json` 的
  `https://github.com/ZilongYang/pi-web-desktop/releases/latest/download/latest.json`；
  GitHub 的 `releases/latest` **自动忽略 pre-release**。
- **pi-web 热更新（L3）**：读 Release 资产里的 `piweb-sidecar.json` 清单。
- 发布：tag `v*` → CI 构建 → 发 Release（非预发布）→ 自动成为 `latest`。

问题：没有地方发「测试版」。想给一版试水，只能直接发正式版，或者手动装本地构建。

## 二、批准结论（用户 2026-11-03 拍板）

| # | 问题 | 结论 |
|---|---|---|
| 1 | 通道模型 | **正式版 + 测试版 beta**（不做 nightly） |
| 2 | 从 beta 切回正式版、但当前版本更新时 | **允许降级重装**（下载稳定的安装包覆盖安装，约 130MB，需确认） |
| 3 | pi-web 热更新是否分通道 | **不分**：beta 通道只针对壳，pi-web 仍跟正式版（少维护一套侧车包） |
| 4 | 0.2.11（F20 修复） | 先发版 |

## 三、方案

### 3.1 版本号语义

- 正式版：`vX.Y.Z`（如 `v0.3.0`）
- 测试版：`vX.Y.Z-beta.N`（如 `v0.3.0-beta.1`），**下一个正式号 + 预发布后缀**。
  这样正式版发出来（`0.3.0`）后，beta 用户切回正式版能自然升级（`0.3.0 > 0.3.0-beta.N`）。
- 签名：beta 用**同一把 updater 私钥**（否则校验失败），只是版本号不同。

### 3.2 GitHub 侧的清单布局

| 通道 | 清单地址 | 说明 |
|---|---|---|
| 正式版 | `releases/latest/download/latest.json` | GitHub 自动指向最新的**非预发布** Release，零维护 |
| 测试版 | `releases/download/channel-beta/latest.json` | 固定 tag `channel-beta` 的 Release，由 CI 在每次发 beta 时覆盖上传 |

- beta 的 tag（`v0.3.0-beta.1`）发成 **pre-release**，不会抢走 `latest`。
- 固定 `channel-beta` Release 只放清单（`latest.json`），不放安装包 —— 顺带把 F14 的
  「清单挂在应用 Release 上被 `--clobber` 冲掉 / CDN 缓存」两个坑分开处理。

### 3.3 应用侧

- `config.json` 新增 `update_channel: "stable" | "beta"`（默认 `stable`）。
- 菜单：`Pi Web Desktop → 更新通道` 子菜单，两个勾选项：**正式版（默认）** / **测试版 beta**。
- 「检查应用更新…」按当前通道取清单检查。
- **通道切换**：切换后立刻按新通道检查一次：
  - 目标通道版本 ≠ 当前版本（**允许降级**）→ 弹窗「切换到 X 通道：vA → vB，会下载并重装，完成后自动重启」→ 走现有的下载进度窗 + 安装 + 重启确认；
  - 版本相同 → 提示「已切换，当前已是最新」。
- 关于面板的 credits 里显示当前通道。
- 实现要点：用 `tauri_plugin_updater` 的 `UpdaterBuilder` 在运行时指定 endpoint；
  降级通过 `version_comparator(|current, remote| remote.version != current)` 绕过「只升不降」。
- L3（pi-web 热更新）**不受影响**，仍读正式版 Release 的侧车清单。

### 3.4 CI（release.yml）

- tag 含 `-` → 判定为 beta：
  - `gh release create ... --prerelease`
  - 额外更新固定 `channel-beta` Release：`gh release upload channel-beta dist/latest.json --clobber`
    （不存在就先 `gh release create channel-beta`）
- 正式版 tag：行为不变（非预发布 + `latest.json` 随 Release）。
- 现有的「启动冒烟」步骤保留（发布硬规则：必须真机启动验证过）。

## 四、验收

1. 单测：通道 → endpoint 选择、通道字符串归一化。
2. 真机流程（真实 GitHub）：
   - 发 `v0.3.0-beta.1`（pre-release）→ 本机装它 → 菜单切到「测试版」→ 检查更新提示已是最新；
   - 发 `v0.3.0-beta.2` → 测试版通道能检出并升级；
   - 发 `v0.3.0`（正式版）→ 切回「正式版」→ 提示降级重装 → 装成 0.3.0。
3. 发版前仍必须 `npm run smoke` + `npm run smoke:app`。

## 五、风险

- **降级重装**复用 updater 的安装逻辑（替换 .app 后重启）；如果用户此时有未保存的终端会话，重启会丢 —— 弹窗里写明。
- beta 清单走固定 Release，仍受 GitHub CDN 约 5 分钟缓存影响（F14）→ 刚发 beta 立刻切通道可能还看到旧清单，属预期。
- pi-web 不分通道：如果 beta 壳依赖上游 pi-web 的预发布特性，需要手动热更新，属已知限制。

## 事后补记（2026-11-03 晚）

1. **版本递增口径修正**（用户要求）：本方案的「beta 用下一个正式号 + 后缀」仍然成立，但**默认只递增 patch**，
   不再为功能去升 minor。例如 `0.3.0` 之后的版本是 `0.3.1-beta.1` → `0.3.1` → `0.3.2-beta.1` …。
   完整规则见 [RELEASE-PROCESS.md](RELEASE-PROCESS.md)。
2. **2026-11-03 晚：`0.3.0-beta.2` 已转正为 `v0.3.0` 正式版**（Latest），beta 通道保留 beta.2 不再更新；之后按「默认递增 patch」的规则进入 `0.3.1-beta.1`。
3. 实施中发现的探针小 bug（env 通道只用于打印、实际按 config 检查）已修，记录见 [QA-2026-11-01.md](QA-2026-11-01.md)。
