# 文档索引

> 这个项目按「计划存档、验收留痕、发版打勾、BUG 留档」的规矩维护文档。四类文档（计划 / 验收 / 发版清单 / BUG 记录）的更新规则不同，先看下表再动手。
>
> **中断后恢复上下文**：先读 [../ROADMAP.md](../ROADMAP.md) 的「下次继续」+「已修复记录」，再 `git log --oneline -5`。
>
> 当前版本以 ROADMAP 顶部为准（本文档不写版本号，避免过期）。

| 文档 | 是什么 | 更新规则 |
|---|---|---|
| [../ROADMAP.md](../ROADMAP.md) | 现在做到哪、接下来做什么（含 **Backlog**） | **持续更新**：每完成一项就改状态；新需求先记这里再开工 |
| [RELEASE-PROCESS.md](RELEASE-PROCESS.md) | **发版流程与版本号规则**（默认只递增 patch；beta → 验证 → 正式版） | 持续更新：规则变了先改这里 |
| [PLAN-2026-11-01.md](PLAN-2026-11-01.md) | 初版壳的已批准方案（Tauri 壳、侧车、进程管理、签名、平台矩阵） | **不修改**：批准的原文存档；实施偏差只在文末「事后补记」追加 |
| [PLAN-2026-11-02-L2L3.md](PLAN-2026-11-02-L2L3.md) | L2（应用自更新）/ L3（pi-web 热更新）的已批准方案（**L3 已于 2026-11-04 取消**，历史方案） | 同上（含实施记录与踩坑） |
| [PLAN-2026-11-03-channels.md](PLAN-2026-11-03-channels.md) | 更新通道（正式版 / 测试版 beta）的已批准方案 | 同上 |
| [PLAN-2026-11-03-sidecar-manifest.md](PLAN-2026-11-03-sidecar-manifest.md) | L3 侧车清单独立（F14）的已批准方案（随 L3 一起取消，历史方案） | 同上 |
| [PLAN-2026-11-03-tray.md](PLAN-2026-11-03-tray.md) | 菜单栏托盘 + 关窗隐藏（F16）的已批准方案 | 同上 |
| [PLAN-2026-11-04-drop-l3.md](PLAN-2026-11-04-drop-l3.md) | **取消 L3**（pi-web 随壳固定）+ 运行期定期检查的已批准方案 | 同上 |
| [PLAN-2026-11-05-path-selfheal.md](PLAN-2026-11-05-path-selfheal.md) | 已批准并实施：PATH 缓存自愈（P0a/P0b/P1/P3+菜单重新载入登录环境） | 同上 |
| [QA-2026-11-01.md](QA-2026-11-01.md) | 验收清单 + 每轮真机验证记录 + 回归修复记录 | **持续追加**：每次修 bug / 发版验证都在对应小节记结果 |
| [BUG-2026-11-05-login-path-cache.md](BUG-2026-11-05-login-path-cache.md) | 登录 shell PATH 缓存陈旧导致 App 内全局命令消失：根因、证据、临时修复与根因修复方案（P0–P3） | **不修改**：留档；已追加 §11 实施记录（0.3.3-beta.1） |
| [RELEASE-CHECKLIST-0.2.0.md](RELEASE-CHECKLIST-0.2.0.md) | 每个版本的发版打勾清单（0.2.0 至最新） | **持续追加**：发一版加一节，不改历史 |

## 截图

| 文件 | 内容 |
|---|---|
| [screenshot-attach-mode.png](screenshot-attach-mode.png) | 附着到本机已在运行的 pi-web（30141） |
| [screenshot-menu-chinese.png](screenshot-menu-chinese.png) | 全中文菜单栏（0.2.5） |
| [screenshot-update-progress.png](screenshot-update-progress.png) | 应用更新（L2）下载进度窗（0.2.3 起） |
| [screenshot-l3-progress.png](screenshot-l3-progress.png) | pi-web 热更新（L3）安装阶段进度窗（0.2.7 起；L3 已取消，属历史截图） |
| [screenshot-tray-icon.png](screenshot-tray-icon.png) | 菜单栏托盘图标（π 模板图，0.3.1 起） |
| [screenshot-composer-scrollbar-before.png](screenshot-composer-scrollbar-before.png) | F20 修复前：缩放 150% 时输入框横向滚动条压住文字（0.2.10） |
| [screenshot-composer-scrollbar-after.png](screenshot-composer-scrollbar-after.png) | F20 修复后：同一缩放档位下显示正常（0.2.11） |
| [screenshot-error-state.png](screenshot-error-state.png) | 启动失败态（原生对话框 + 日志尾部 + 重试） |

## 写文档的约定

1. **方案**：批准时原文存档，文件名 `PLAN-<日期>.md`；实施后不改正文，只在文末「事后补记」写偏差与原因。
2. **验收**：能复现的结论才写「已验证」，并附命令/截图/日志证据；没跑过的写「未验证」。
3. **发版**：每版在 `RELEASE-CHECKLIST` 加一节打勾清单，包含"怎么验证的"。
4. **Backlog**：新需求先写进 ROADMAP（现状/目标/落点/验收），再决定何时做——避免"聊过就忘"。
