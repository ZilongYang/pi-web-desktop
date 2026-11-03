# Pi Web Desktop

给 [pi-web](https://github.com/agegr/pi-web) 套的 Tauri 2 桌面壳：**自带 Node 运行时 + 内置 pi-web 服务端**，双击即用，不装 Node、不开终端、不碰浏览器。

和命令行 `pi`、浏览器版 pi-web 共享同一份 `~/.pi/agent`（会话、模型、skills、信任设置）。

> 当前进度与待办见 [ROADMAP.md](ROADMAP.md)；文档入口见 [docs/README.md](docs/README.md)。

## 下载 / 安装（macOS）

从 [Releases](https://github.com/ZilongYang/pi-web-desktop/releases/latest) 下载 `Pi-Web-Desktop-aarch64.dmg`（或 `.zip`），拖进「应用程序」。

**首次打开需要放行**：本项目用 **ad-hoc 签名**（和 zWGestures 的做法一致），没有 Apple Developer ID 证书、也没做公证，所以 Gatekeeper 会拦一次。两种放行方式任选：

- 右键点 App → **打开** → 再点「打开」；
- 或终端里执行：`xattr -dr com.apple.quarantine "/Applications/Pi Web Desktop.app"`

为什么不用 Developer ID：申请、年费、公证流程都只为消掉这一次点击；ad-hoc 不需要任何证书，任何人 clone 下来都能复现出同样的包。取舍写在 `scripts/make-dist.sh` 头部。

## 它是怎么跑的

```
Pi Web Desktop.app
├─ Rust (Tauri 2)        原生窗口 / 菜单 / 生命周期 / 侧车守护
│    └─ 窗口直接导航 http://127.0.0.1:<port>
└─ Resources/pi-web/     侧车（npm 包 + 官方 Node 二进制）
     └─ node bin/pi-web.js --no-open -p <port> -H 127.0.0.1
          └─ next start（launcher 的子进程）
```

启动流程：splash 窗口 → 清理上次崩溃残留的侧车 → 若本机已有**同版本** pi-web 在跑就附着 → 否则选稳定端口起侧车 → `GET /` 探活 → 导航过去 → 每 10s 健康监控 → 退出时按进程组收尾（不留孤儿 `next-server`）。

为什么不 iframe、不自己起 `next start`、不 fork pi-web：见 `docs/PLAN-2026-11-01.md`。

## 开发

```bash
npm install                 # 只装 @tauri-apps/cli
npm run sidecar             # 装侧车：npm 包 + 官方 Node + 裁剪（约 400MB，首次要下载）
npm run smoke               # 侧车冒烟测试：起服务 → 探活 → 收尾 → 断言无残留
npm run smoke:app           # App 启动冒烟：真启动构建产物，断言不崩 + 窗口几何存取（发版前必跑）
npm run dev                 # tauri dev（侧车缺失时会先自动下载）
```

要求：macOS 12+ / Xcode Command Line Tools / Rust stable / Node 22（只为跑脚本，App 本身不依赖系统 Node）。

Rust 侧：

```bash
cd src-tauri
cargo test                  # 单元测试（探活、run record 解析、端口选择）
cargo build                 # 调试构建
../node_modules/.bin/tauri build   # 出 .app / .dmg
```

## 内置版本

| 壳版本 | 内置 pi-web | 内置 Node |
|---|---|---|
| 0.3.1-beta.1 | 0.14.2 | 22.23.4 |

发版流程与版本号规则见 [docs/RELEASE-PROCESS.md](docs/RELEASE-PROCESS.md)（默认只递增 patch：`0.3.1-beta.1` → `0.3.1` → `0.3.2-beta.1`）。

版本只在 `sidecar.json` 里改一处：改完 `npm run sidecar` 再重建即可。升级 pi-web 后**必须**跑一次 `npm run smoke`。

**Node 要跟安全版本走**：内置 Node 是 pi-web 服务端的运行时，它负责所有出站 HTTPS（模型 API、OAuth 登录、更新检查）。`pi.json` 里 pin 的版本低于上游安全修复时，pi 的 Plannotator 扩展会在界面顶部提示（例如 2026-10 的 eMudhra CA 事件要求 Node ≥ 22.23.4 / 24.22.1 / 26.12.1）。升级方式同样是改 `sidecar.json` 的 `nodeVersion` → `npm run sidecar` → `tauri build`。

## 配置

`~/Library/Application Support/com.zilong.piweb.desktop/config.json`：

> `update_channel`：`stable`（默认，正式版）或 `beta`（测试版）。菜单 `Pi Web → 更新通道` 可直接切换；切回正式版时如果当前版本更高，会提示降级重装。

```json
{
  "port": 30241,
  "attach_policy": "same-version",
  "proxy": null,
  "extra_env": {},
  "pi_coding_agent_dir": null
}
```

- `port`：默认 30241（刻意与 pi-web CLI 的 30141 错开，避免撞端口）。**要稳定**。WebView 的 localStorage 按 origin（含端口）隔离，换端口等于侧栏偏好/主题/语言/草稿全部重置。端口被占时会顺延 10 个并写回这里。
- `attach_policy`：`same-version`（默认，同版本就附着本机已有实例）/ `never`（永远起自己的）。
- `proxy`：GUI 启动拿不到你终端里的 `HTTPS_PROXY`。需要让 pi-web 的服务端请求走 Clash，就填 `"http://127.0.0.1:7890"`（会同时注入 `NO_PROXY=localhost,127.0.0.1`）。
- `pi_coding_agent_dir`：默认 `null`，即共享 `~/.pi/agent`。

日志：`~/Library/Logs/com.zilong.piweb.desktop/sidecar.log`（菜单「打开日志目录」）。

## 已知差异 / 注意事项

- **没有 Service Worker**：macOS 的 WKWebView 不支持 SW（Tauri 无法开 App-Bound Domains），pi-web 的 `sw.js` 注册会失败，页面功能不受影响，只是没有离线缓存。
- **`PATH` 怎么来的**：从 Dock/Finder 启动的 app 只有 `/usr/bin:/bin:/usr/sbin:/sbin`，终端里的 `PATH`（nvm、pnpm、deno、go、x-cmd…）全是 `.zshrc` 自己加的。所以侧车启动时的 `PATH` 按这个顺序拼：
  1. 登录 shell 的 `PATH` 缓存（`~/Library/Application Support/com.zilong.piweb.desktop/login-path.txt`，7 天有效）；
  2. 没有缓存时用「常见目录兜底」（nvm 各版本按 `~/.nvm/alias/default` 优先、volta/asdf/mise/fnm、pnpm、deno、go、conda、orbstack、x-cmd…），同时**后台**跑一次 `zsh -lic 'printf %s "$PATH"'` 写缓存给下次启动用（GUI 里这条命令实测会卡 1s 上下，所以不阻塞启动）；
  3. 进程继承来的 `PATH`；
  4. 最后才是包内 `runtime`（保证你机器上完全没有 node 时也能跑；正常情况用你自己的 node，和终端一致）。

  实测结果：侧车里 `node`/`pnpm`/`mnemon`/`git`/`python3`/`go`/`deno` 的解析结果与终端**完全一致**。想换规则：`config.json` 里 `inherit_login_shell_path: false` 就只用兜底目录；删掉上面的缓存文件可强制重新探测。
- **TCC 授权**：项目放在 `~/Documents`、`~/Desktop`、`~/Downloads` 时，首次访问会弹系统授权，允许后才会出现在文件树里。
- **`pi-web stop` 会停掉桌面版的服务**：因为共用同一套 run record；健康监控会弹窗，点「重启服务」即可。
- **导出会话 HTML 暂时不可用（pi-web 0.14.2 + pi SDK 1.5.0 的上游不兼容）**：点「导出」会报 `Failed to patch exported HTML: sortChildren expected 1 match, found 0`。这是服务端的补丁逻辑与新版 SDK 模板对不上（细节见 [ROADMAP · B2](ROADMAP.md)），与本壳无关；临时可用命令行 `pi --export <session.jsonl> out.html` 导出，或等方案定稿后修复。
- **首次启动会重写 `Resources/pi-web/app/node_modules/@agegr/pi-web/.next/prerender-manifest.json`**（pi-web launcher 轮换 preview 密钥的安全设计）。之后 `codesign --deep --strict` 会因这一个数据文件报签名不符；签名校验请在本机首次运行**之前**做，或改用 Developer ID + 重新签名的发布流程。
- **附着模式**：连的是别人启动的 server，退出时不会杀它；桌面窗口与浏览器窗口的 localStorage 互不共享（WKWebView 的数据目录按 app 隔离）。

## 菜单

全中文，跟系统语言一致（Tauri 自带的 `Menu::default()` 只有英文、而且**没有 View 菜单**，所以这里自己拼）：

![菜单](docs/screenshot-menu-chinese.png)

| 菜单 | 内容 |
|---|---|
| Pi Web Desktop | 关于 / 服务 / 隐藏 / 退出（⌘Q） |
| 文件 | 关闭窗口（⌘W） |
| 编辑 | 撤销 ⌘Z / 重做 ⇧⌘Z / 剪切 ⌘X / 拷贝 ⌘C / 粘贴 ⌘V / 全选 ⌘A |
| **视图** | **放大 ⌘= / 缩小 ⌘- / 实际大小 ⌘0 / 进入全屏**；档位 50%–300%，会记住（写进 `config.json` 的 `zoom`） |
| 窗口 | 最小化 / 缩放窗口 |
| Pi Web | 重新加载页面 ⌘R / 重启服务 ⇧⌘R / **检查更新…** / **更新 pi-web…** / 在浏览器中打开 / 复制服务地址 / 打开日志目录 / 开发者工具 ⌥⌘I |
| 帮助 | GitHub 仓库 |

页面缩放只影响显示，不影响终端面板里执行的命令。

## 更新

三层机制，都是「菜单 → 检查更新…」的一部分：

| 层 | 更新什么 | 怎么做 | 失败时 |
|---|---|---|---|
| **L1 检测** | 版本认知 | 每次启动查 npm registry，落后就提示（同一版本只提醒一次） | 静默 |
| **L2 应用自更新** | 整个 `.app`（含内置 pi-web / pi SDK / Node） | Tauri updater：下载 `.app.tar.gz` → 校验 minisign 签名 → 替换 → 重启 | 报错并保持旧版本 |
| **L3 pi-web 热更新** | **只换 pi-web**，Node 不动 | 从 GitHub Release 下载侧车包（≈80MB）→ **进度窗** → 校验 SHA256 → 解压 → 原子切换 → 重启侧车 | 自动回滚到上一版本（也有提示） |

细节：

- 菜单「检查更新…」先查应用本身（L2），没有应用更新（或你选择跳过）再查 pi-web（L3）；「Pi Web → 更新 pi-web…」可直达 L3。
- **每次启动都会静默检查**（应用更新 20s 后、pi-web 10s 后）；同一个版本只提醒一次，不会每次开机都弹窗。
- L3 装到 `~/Library/Application Support/com.zilong.piweb.desktop/sidecar/`，**不改签名过的 `.app`**；包内版本更高时会自动忽略旧的热更新目录（升级 app 后不会"降级运行"）。
- pi 的扩展/插件/skills 不需要壳参与：pi-web 的插件面板自己就能装/升级。
- 开发/演练用环境变量：`PIWEB_UPDATE_HEADLESS=1`（不弹窗、自动确认）、`PIWEB_SIDECAR_MANIFEST_URL`（指向本地假 Release）、`PIWEB_UPDATE_CHECK_ON_STARTUP=1`（启动 3s 后自动跑一次检查）。

## 签名与发布

| 场景 | 签名 | 说明 |
|---|---|---|
| 日常构建（自己用） | 本机自签名证书 | `npm run signing-cert` 建一把固定证书（独立钥匙串），然后 `APPLE_SIGNING_IDENTITY="Pi Web Desktop Local Signing" npm run build`。好处是 App 身份稳定，TCC/授权在重建之间不会被重置 |
| 发布产物（`dist/`） | **ad-hoc**（`codesign -s -`） | 对下载者来说两者等价（都过不了 Gatekeeper 的 Developer ID 要求），ad-hoc 不依赖任何证书，构建可复现 |
| 自动更新包 | Tauri updater 密钥（minisign） | 与 Apple 签名无关；私钥只存在 GitHub Secrets，公钥在 `tauri.conf.json` |

updater 密钥（minisign）与 Apple 签名无关：

```bash
# 一次性生成（私钥不进 git）
./node_modules/.bin/tauri signer generate -w .secrets/updater.key -p "<password>"
# 公钥写进 src-tauri/tauri.conf.json → plugins.updater.pubkey
# 私钥与密码加到 GitHub Secrets：TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

发布：改 `src-tauri/tauri.conf.json` 与 `package.json`/`Cargo.toml` 的版本 → 提交 → `git tag v<ver> && git push origin v<ver>`，CI（`.github/workflows/release.yml`）会构建并发布 Release（dmg / zip / updater 包 + `latest.json` / L3 侧车包 + `piweb-sidecar.json` / SHA256SUMS）。

本地想先验证打包：`npm run dist`（在普通终端里跑，`diskutil` 需要挂载卷）。

## 构建环境的坑（已内置规避）

- **macOS 26/27 + Rust < 1.99**：cargo 的 release 默认 strip 会作用到 proc-macro，strip 后的 dylib 被新 dyld 拒绝（`mis-aligned LINKEDIT string pool`，表现为 `can't find crate for xxx_derive`）。`src-tauri/Cargo.toml` 已加 `[profile.release.build-override] strip = false` 规避；升到 Rust ≥ 1.99 后可删。
- 侧车 400MB，`tauri build` 打包时会把 `sidecar/` 复制进 `Contents/Resources/pi-web/`，`.app` 体积在 400MB 量级，dmg 压缩后更小。

## 平台

| | macOS | Windows | Linux |
|---|---|---|---|
| 状态 | v1 主线，已跑通 | 代码路径已就绪，需在 Windows 上跑一次 `npm run sidecar && npm run build` | 代码路径已就绪，未验证 |
| 侧车 | `darwin-arm64/x64` | `win32-x64/arm64`（`taskkill /T /F` 收尾、`CREATE_NO_WINDOW`） | `linux-x64/arm64` |
| 产物 | `.app` + `.dmg` | NSIS | deb/AppImage（待配） |

## 目录

```
sidecar.json           # 唯一版本源（pi-web / node）
scripts/               # fetch-sidecar（装+裁剪+manifest）、prune、smoke
ui/index.html          # 本地 splash 页（唯一能用 Tauri IPC 的页面）
src-tauri/src/         # main（状态机/窗口/菜单/命令）、sidecar、probe、discovery、logging
docs/PLAN-2026-11-01.md# 方案原文（批准版）+ 事后补记
```

## 文档

总入口：[docs/README.md](docs/README.md)（每份文档的作用与更新规则）

- **路线图 / Backlog**：[ROADMAP.md](ROADMAP.md) —— 现在做到哪、接下来做什么（新需求先记这里）
- 方案与实施记录：[PLAN-2026-11-01.md](docs/PLAN-2026-11-01.md)（初版壳）、[PLAN-2026-11-02-L2L3.md](docs/PLAN-2026-11-02-L2L3.md)（更新机制）
- 验收与回归：[QA-2026-11-01.md](docs/QA-2026-11-01.md)
- 发版清单：[RELEASE-CHECKLIST-0.2.0.md](docs/RELEASE-CHECKLIST-0.2.0.md)

## License

MIT，见 [LICENSE](LICENSE)。
