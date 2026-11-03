#!/usr/bin/env node
/**
 * smoke-app.mjs -- 启动**构建产物**，验证「App 真能起来」。
 *
 * 为什么需要（0.2.9 的教训）：`npm run smoke` 只测侧车（pi-web 服务），App 自己的
 * 启动路径（菜单 / 窗口 / 配置）完全没覆盖。0.2.9 因为在菜单构建期调 `app.path()`
 * 直接 panic —— CI 全绿、二进制里字符串也在，用户双击就崩。这个脚本就是补上这一课：
 * **发版前必须真的把 App 启动一次**。
 *
 * 做法（不碰用户真实配置，也不需要 GUI 自动化权限）：
 *   0. 先把 .app **复制一份**再启动：pi-web 运行时会改写自己 `.next` 下的文件
 *      （如 prerender-manifest.json），原地启动会把签名封印弄坏，让 CI 的打包校验
 *      （codesign --verify --deep --strict）失败。
 *      ⚠️ 副本必须放在**非临时目录**：macOS 不允许从 /var/folders、/tmp 启动的 .app 访问
 *      自身资源（`sandbox_extension_issue_file_to_process` 失败 → 侧车找不到 → 启动失败）。
 *      所以复制到 `~/Applications/`，测试结束删掉。
 *   1. 用 `PIWEB_CONFIG_DIR` 指到临时目录隔离配置；
 *   2. `PIWEB_WINDOW_PROBE=1` 让 App 启动 3 秒后打印窗口几何、写盘、退出；
 *   3. 断言：退出码 0 / stderr 里没有 `panicked at` / 有「启动 Pi Web Desktop」行 /
 *      config.json 里写进了 window 几何；
 *   4. 第二轮：预先写一个几何进 config，断言启动日志出现「恢复窗口 W×H @ (x, y)」。
 *
 * 环境不支持 GUI（无 WindowServer 等）时打印 SKIP 并返回 0；**panic 永远算失败**。
 *
 * 用法：
 *   node scripts/smoke-app.mjs                 # 默认找 src-tauri/target/release/bundle/macos
 *   node scripts/smoke-app.mjs --app <path>    # 指定 .app
 *   node scripts/smoke-app.mjs --keep          # 保留临时目录（排查用）
 */
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_APP = path.join(ROOT, "src-tauri", "target", "release", "bundle", "macos", "Pi Web Desktop.app");
const EXIT_TIMEOUT_MS = 60_000;

const argv = process.argv.slice(2);
const argValue = (flag) => {
  const index = argv.indexOf(flag);
  return index >= 0 ? argv[index + 1] : undefined;
};
const APP = path.resolve(argValue("--app") ?? DEFAULT_APP);
const KEEP = argv.includes("--keep");
const VERSION = JSON.parse(fs.readFileSync(path.join(ROOT, "package.json"), "utf8")).version;

const failures = [];
const ok = (message) => console.log(`  ✓ ${message}`);
const bad = (message) => {
  console.log(`  ✗ ${message}`);
  failures.push(message);
};

/** 环境不支持 GUI 的特征（CI/无头容器）；命中就跳过，不算失败。 */
const ENV_SKIP_MARKERS = [
  "WindowServer",
  "window server",
  "NSApplication",
  "unable to open display",
  "cannot open display",
  "The application cannot be opened",
];

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

function writeConfig(configDir, config) {
  fs.mkdirSync(configDir, { recursive: true });
  fs.writeFileSync(path.join(configDir, "config.json"), `${JSON.stringify(config, null, 2)}\n`);
}

function readConfig(configDir) {
  try {
    return JSON.parse(fs.readFileSync(path.join(configDir, "config.json"), "utf8"));
  } catch {
    return null;
  }
}

/** 启动一次 App，等它自己退出（探针会 exit(0)），返回输出。 */
function httpOk(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ port, host: "127.0.0.1" });
    socket.setTimeout(700);
    socket.on("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.on("error", () => resolve(false));
    socket.on("timeout", () => {
      socket.destroy();
      resolve(false);
    });
  });
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function runOnce({ binary, configDir, port, label }) {
  return new Promise((resolve) => {
    const child = spawn(binary, [], {
      cwd: path.dirname(binary),
      env: {
        ...process.env,
        PIWEB_CONFIG_DIR: configDir,
        PIWEB_WINDOW_PROBE: "1",
        PIWEB_WINDOW_PROBE_DELAY_SECS: "10",
        PIWEB_UPDATE_HEADLESS: "1",
        PIWEB_SKIP_VERSION_CHECK: "1",
      },
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    });

    let output = "";
    const collect = (chunk) => {
      output += chunk.toString();
      if (output.length > 400_000) output = output.slice(-200_000);
    };
    child.stdout.on("data", collect);
    child.stderr.on("data", collect);

    let settled = false;
    const finish = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(result);
    };

    const killTree = () => {
      try {
        process.kill(-child.pid, "SIGKILL");
      } catch {
        try {
          child.kill("SIGKILL");
        } catch {
          /* ignore */
        }
      }
    };

    const timer = setTimeout(() => {
      killTree();
      finish({ timedOut: true, code: null, output, label });
    }, EXIT_TIMEOUT_MS);

    // 启动后尽快确认侧车真的在服务（否则「窗口能开但服务没起来」会被误判为通过）
    let served = false;
    const pollStarted = Date.now();
    const poll = setInterval(async () => {
      if (served || settled) return;
      if (await httpOk(port)) {
        served = true;
        clearInterval(poll);
      } else if (Date.now() - pollStarted > 12_000) {
        clearInterval(poll);
      }
    }, 300);

    child.on("exit", (code) => {
      killTree(); // 侧车/子进程一并收掉，避免残留
      finish({ timedOut: false, code, output, served, label });
    });
    child.on("error", (error) => finish({ timedOut: false, code: null, output: `${output}\n${error.message}`, label }));
  });
}

/** 判断一次运行是否「环境不支持 GUI」，是则整体 SKIP。 */
function looksLikeEnvironmentSkip(result) {
  if (result.timedOut) return false;
  if (result.code === 0) return false;
  return ENV_SKIP_MARKERS.some((marker) => result.output.includes(marker));
}

function printOutput(result) {
  const lines = result.output.split("\n").filter(Boolean).slice(-25);
  console.log("  ── App 输出（末 25 行）──");
  for (const line of lines) console.log(`  ${line}`);
}

async function main() {
  console.log(`[smoke-app] 启动冒烟测试（期望版本 ${VERSION}）`);

  if (process.platform !== "darwin") {
    console.log("[smoke-app] SKIP：目前只验证 macOS 构建产物");
    return;
  }
  if (!fs.existsSync(APP)) {
    console.error(`[smoke-app] 找不到构建产物：${APP}\n  先跑：APPLE_SIGNING_IDENTITY="-" ./node_modules/.bin/tauri build --bundles app`);
    process.exit(1);
  }
  const binary = path.join(APP, "Contents", "MacOS", "pi-web-desktop");
  if (!fs.existsSync(binary)) {
    console.error(`[smoke-app] 产物里没有可执行文件：${binary}`);
    process.exit(1);
  }

  // 已有实例在跑时，单实例插件会让新进程直接退出，表现成"启动即结束"——先说清楚。
  const running = await new Promise((resolve) => {
    const socket = net.connect({ port: 30241, host: "127.0.0.1" });
    socket.setTimeout(800);
    socket.on("connect", () => { socket.destroy(); resolve(true); });
    socket.on("error", () => resolve(false));
    socket.on("timeout", () => { socket.destroy(); resolve(false); });
  });
  if (running) {
    console.error("[smoke-app] 检测到已有实例在运行（端口 30241）：先退出 Pi Web Desktop 再跑本测试");
    console.error("  说明：单实例插件会让新进程立刻退出，看起来像“启动就崩”，会误报。");
    process.exit(1);
  }

  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "piweb-app-smoke-"));
  let appCopyForCleanup = path.join(os.homedir(), "Applications", "Pi-Web-Desktop-Smoke.app");
  const cleanup = () => {
    if (!KEEP) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      fs.rmSync(appCopyForCleanup, { recursive: true, force: true });
    } else {
      console.log(`  临时目录保留：${tmpRoot}`);
    }
  };

  try {
    // 复制一份再跑：pi-web 会往自己 `.next/` 里写文件，原地跑会破坏已签名的产物。
    // 注意放在 ~/Applications（不能放 temp，见文件头注释）。
    const copyRoot = path.join(os.homedir(), "Applications");
    fs.mkdirSync(copyRoot, { recursive: true });
    const appCopy = path.join(copyRoot, "Pi-Web-Desktop-Smoke.app");
    fs.rmSync(appCopy, { recursive: true, force: true });
    execFileSync("ditto", [APP, appCopy], { stdio: "inherit" });
    const runBinary = path.join(appCopy, "Contents", "MacOS", "pi-web-desktop");
    if (!fs.existsSync(runBinary)) {
      console.error(`[smoke-app] 复制后的产物里没有可执行文件：${runBinary}`);
      process.exit(1);
    }

    // ---- 第一轮：全新配置，验证能启动 + 几何能落盘 -------------------------
    const firstConfigDir = path.join(tmpRoot, "config-1");
    const firstPort = await freePort();
    writeConfig(firstConfigDir, { port: firstPort, zoom: 1 });

    const first = await runOnce({ binary: runBinary, configDir: firstConfigDir, port: firstPort, label: "first" });
    // DSH/沙箱父进程里启动副本 App 会被 macOS 拒绝 sandbox extension -> 资源找不到。
    // 这不是产物问题（CI/普通终端正常），明确 SKIP，别误报也别弹窗。
    if (first.output.includes("sandbox_extension_issue_file_to_process")) {
      console.log("[smoke-app] SKIP：当前 shell 的沙箱阻止启动副本 App（请在普通终端或 CI 里跑）");
      printOutput(first);
      return;
    }
    if (looksLikeEnvironmentSkip(first)) {
      console.log("[smoke-app] SKIP：当前环境起不了 GUI（无 WindowServer？）");
      printOutput(first);
      return;
    }

    if (first.timedOut) {
      bad(`App 启动后 ${EXIT_TIMEOUT_MS / 1000}s 内没有退出（探针未生效 / 卡死）`);
      printOutput(first);
    } else if (/panicked at/.test(first.output)) {
      bad("启动时 panic（就是 0.2.9 那类崩溃）");
      printOutput(first);
    } else if (first.code !== 0) {
      bad(`App 退出码 ${first.code}（预期 0）`);
      printOutput(first);
    } else {
      ok(`App 启动并正常退出（退出码 0）`);
      if (first.output.includes(`启动 Pi Web Desktop ${VERSION}`)) ok(`日志有启动行（当前版本 ${VERSION}）`);
      else bad(`日志里没有「启动 Pi Web Desktop ${VERSION}」`);

      const saved = readConfig(firstConfigDir);
      const geometry = saved?.window;
      if (geometry && geometry.width >= 200 && geometry.height >= 150) {
        ok(`窗口几何已落盘：${geometry.width}×${geometry.height} @ (${geometry.x}, ${geometry.y})`);
      } else {
        bad(`config.json 里没有有效的 window 几何：${JSON.stringify(geometry)}`);
      }
      if (first.served) {
        ok(`侧车真的在服务（http://127.0.0.1:${firstPort}/ 可连）`);
      } else {
        bad("侧车没有在监听端口（窗口起来了但服务没起）—— 检查启动失败日志");
      }
      if (first.output.includes("[failed]")) {
        bad("启动失败：日志里出现 [failed]");
      }
      if (first.output.includes("window-probe")) ok("探针输出正常（window-probe）");
    }

    // ---- 第二轮：预置几何，验证启动时恢复 --------------------------------
    const secondConfigDir = path.join(tmpRoot, "config-2");
    const secondPort = await freePort();
    const wanted = { x: 120, y: 140, width: 1000, height: 680 };
    writeConfig(secondConfigDir, { port: secondPort, zoom: 1, window: wanted });

    const second = await runOnce({ binary: runBinary, configDir: secondConfigDir, port: secondPort, label: "second" });
    const restoreLine = `恢复窗口 ${wanted.width}×${wanted.height} @ (${wanted.x}, ${wanted.y})`;
    if (second.timedOut) {
      bad("第二轮启动超时");
      printOutput(second);
    } else if (/panicked at/.test(second.output)) {
      bad("第二轮启动时 panic");
      printOutput(second);
    } else if (!second.output.includes(restoreLine)) {
      bad(`启动日志里没有恢复窗口的痕迹：「${restoreLine}」`);
      printOutput(second);
    } else {
      ok(`启动时恢复窗口位置/大小：${wanted.width}×${wanted.height} @ (${wanted.x}, ${wanted.y})`);
    }
  } finally {
    cleanup();
  }

  console.log(failures.length === 0 ? "[smoke-app] 通过" : `[smoke-app] 失败 ${failures.length} 项`);
  process.exit(failures.length === 0 ? 0 : 1);
}

main().catch((error) => {
  console.error("[smoke-app] 异常", error);
  process.exit(1);
});
