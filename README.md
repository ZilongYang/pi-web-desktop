# Pi Web Desktop

给 [pi-web](https://github.com/agegr/pi-web) 套的 Tauri 2 桌面壳：**自带 Node 运行时 + 内置 pi-web 服务端**，双击即用，不装 Node、不开终端、不碰浏览器。

和命令行 `pi`、浏览器版 pi-web 共享同一份 `~/.pi/agent`（会话、模型、skills、信任设置）。

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
| 0.1.0 | 0.13.0 | 22.22.3 |

版本只在 `sidecar.json` 里改一处：改完 `npm run sidecar` 再重建即可。升级 pi-web 后**必须**跑一次 `npm run smoke`。

## 配置

`~/Library/Application Support/com.zilong.piweb.desktop/config.json`：

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
- **自己跑的命令环境不同**：从 Dock/Finder 启动的 app 没有终端的 `PATH` 与 locale。壳会把默认 PATH 补全（homebrew、`~/.local/bin`、`~/.bun/bin`、`~/.cargo/bin`）并设 `LANG=en_US.UTF-8`，如果你的项目依赖别的 shell 配置（nvm/asdf 等），在 `extra_env` 里补 `PATH`。
- **TCC 授权**：项目放在 `~/Documents`、`~/Desktop`、`~/Downloads` 时，首次访问会弹系统授权，允许后才会出现在文件树里。
- **`pi-web stop` 会停掉桌面版的服务**：因为共用同一套 run record；健康监控会弹窗，点「重启服务」即可。
- **首次启动会重写 `Resources/pi-web/app/node_modules/@agegr/pi-web/.next/prerender-manifest.json`**（pi-web launcher 轮换 preview 密钥的安全设计）。之后 `codesign --deep --strict` 会因这一个数据文件报签名不符；签名校验请在本机首次运行**之前**做，或改用 Developer ID + 重新签名的发布流程。
- **附着模式**：连的是别人启动的 server，退出时不会杀它；桌面窗口与浏览器窗口的 localStorage 互不共享（WKWebView 的数据目录按 app 隔离）。

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
