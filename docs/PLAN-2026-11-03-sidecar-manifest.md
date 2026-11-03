# PLAN-2026-11-03 侧车清单独立（F14）

> 存档规则：批准时原文存档、不修改；实施偏差只在文末「事后补记」追加。
> 决策：2026-11-03 22:05，用户确认「按 A 做」。

## 一、问题（现状）

L3（pi-web 热更新）的清单与包，现在和**应用 Release**绑在一起：

- App 读 `https://github.com/ZilongYang/pi-web-desktop/releases/latest/download/piweb-sidecar.json`
- 清单里的包 URL 也是 `releases/latest/download/piweb-app-<ver>-<target>.tar.gz`
- 发布时 `make-dist.sh` 生成清单与侧车包，`release.yml` 把它们和应用包一起挂到同一个应用 Release

两个坑：

1. **CI `--clobber` 覆盖**：想「只换 pi-web、不动壳」时只能把新清单/包传到已有应用 Release；
   下次发壳版本，CI 重新上传资产会把它们冲掉（0.2.5 发生过）。
2. **CDN 缓存**：`releases/latest/download/...` 有约 5 分钟缓存，刚替换完资产仍可能拿到旧清单。

附带问题：每个应用 Release 都重复挂一份 80MB 侧车包；清单 URL 强依赖 `releases/latest`。

## 二、批准方案（A：固定 Release 存侧车）

1. **App**：`hotupdate.rs` 的 `DEFAULT_MANIFEST_URL` 改为
   `…/releases/download/piweb-sidecar/piweb-sidecar.json`
   （保留 `PIWEB_SIDECAR_MANIFEST_URL` 钩子用于本地演练）
2. **清单生成**：`make-dist.sh` 里包的 `url` 改为
   `…/releases/download/piweb-sidecar/piweb-app-<ver>-<target>.tar.gz`
3. **CI**：`release.yml` 新增一步 —— 确保固定 Release `piweb-sidecar` 存在（**必须是 pre-release**，
   否则会抢走 `releases/latest`、把 L2 的 `latest.json` 指错），并把「侧车包 + 清单」`--clobber` 上传。
4. **过渡期兼容（保守双写）**：应用 Release 仍然保留 `piweb-sidecar.json` 与侧车包，
   让 ≤0.3.0 的老版本继续能读；新版本 App 读固定 Release。
5. **独立发布侧车**：固定 Release 建好后，可以不改壳版本、只替换它：
   ```bash
   npm run sidecar && bash scripts/make-dist.sh   # 产出 dist/piweb-app-*.tar.gz + dist/piweb-sidecar.json
   gh release upload piweb-sidecar dist/piweb-app-*.tar.gz dist/piweb-sidecar.json --clobber
   ```
   （无壳版本变化时不必动 `latest.json`）

## 三、验证

1. **本地端到端演练**（不真发版）：
   - 造一个「假的新版本」侧车包（内容就是当前侧车，只把 `manifest.json` 的 `piWebVersion` 抬高，如 0.14.3）
   - 本地起静态服务，清单指向本地 tar.gz
   - 启动 App 时设 `PIWEB_SIDECAR_MANIFEST_URL=http://127.0.0.1:<port>/piweb-sidecar.json`
     + `PIWEB_UPDATE_HEADLESS=1`（自动确认下载）
   - 断言：日志出现「清单里最新 pi-web 0.14.3」→ 下载 → SHA256 → 切换 → 侧车重启成功；
     以及 `overlay` 目录结构正确
   - 演练后删掉 overlay，恢复内置侧车
2. **真机验证**：`0.3.1-beta.1` 发布后，应用内点「更新 pi-web…」应读到固定 Release 的清单
   （当前同版本 → 「已是最新」），并且应用 Release 里那份兼容清单也仍存在。

## 四、风险

- 老版本（≤0.3.0）只认 `releases/latest`，靠「保守双写」继续可用；等用户升级到 0.3.1+ 后自然走新地址。
- `piweb-sidecar` 必须是 pre-release，否则会污染 `releases/latest`（CI 里已写死 `--prerelease`）。
- 固定 Release 的资产替换仍受 GitHub CDN 缓存影响（约 5 分钟），但影响面从「应用 Release」缩到「侧车清单」，
  且用户手动点「更新 pi-web…」时更容易用重试绕过。
