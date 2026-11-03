#!/usr/bin/env node
/**
 * smoke.mjs -- 侧车冒烟测试（每个平台 CI 都跑，裁剪后必须复跑）。
 *
 *   1. 用 sidecar/runtime/node 起 sidecar/app/bin/pi-web.js（临时 agent 目录，不碰 ~/.pi）
 *   2. 等 GET / 与 /api/sessions 正常；再测导出接口（?inline=1）返回 HTML（F18）
 *   3. SIGTERM 收尾，断言：端口释放 / 进程树全灭 / run record 已删
 *
 * 退出码 0 = 通过。
 */
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SIDECAR = path.join(ROOT, "sidecar");
const NODE_BIN = path.join(SIDECAR, "runtime", process.platform === "win32" ? "node.exe" : "node");
const APP_DIR = path.join(SIDECAR, "app");
// npm --prefix 安装后包体在 app/node_modules/@agegr/pi-web；launcher 用 __dirname/.. 当 pkgDir，
// 所以 cwd（以及 .next、next.config.ts 的位置）必须是这个包目录。
const PKG_DIR = path.join(APP_DIR, "node_modules", "@agegr", "pi-web");
const ENTRY = path.join(PKG_DIR, "bin", "pi-web.js");

const READY_TIMEOUT_MS = 90_000;
const STOP_TIMEOUT_MS = 10_000;

const failures = [];
const ok = (msg) => console.log(`  ✓ ${msg}`);
const bad = (msg) => {
  console.log(`  ✗ ${msg}`);
  failures.push(msg);
};

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

function portOpen(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ port, host: "127.0.0.1" });
    socket.setTimeout(500);
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

async function httpStatus(port, urlPath = "/") {
  try {
    const res = await fetch(`http://127.0.0.1:${port}${urlPath}`, {
      redirect: "manual",
      signal: AbortSignal.timeout(3000),
    });
    return res.status;
  } catch {
    return 0;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  console.log("[smoke] 侧车冒烟测试");
  for (const file of [NODE_BIN, ENTRY]) {
    if (!fs.existsSync(file)) {
      console.error(`缺少 ${file}，先跑 npm run sidecar`);
      process.exit(1);
    }
  }

  const port = await freePort();
  const agentDir = fs.mkdtempSync(path.join(os.tmpdir(), "piweb-smoke-agent-"));
  const runDir = path.join(agentDir, "pi-web-run");
  // 造一个最小会话：导出接口（F18）要有 id 才能测。布局见 pi CLI：
  //   <agent>/sessions/<project>/<timestamp>_<sessionId>.jsonl，首行是 session 头。
  const FIXTURE_ID = "smoke-fixture-session";
  const fixtureDir = path.join(agentDir, "sessions", "--smoke-fixture--");
  fs.mkdirSync(fixtureDir, { recursive: true });
  fs.writeFileSync(
    path.join(fixtureDir, `2026-01-01T00-00-00-000Z_${FIXTURE_ID}.jsonl`),
    [
      {
        type: "session",
        version: 3,
        id: FIXTURE_ID,
        timestamp: "2026-01-01T00:00:00.000Z",
        cwd: agentDir,
      },
      {
        type: "message",
        id: "smoke-user-1",
        parentId: null,
        timestamp: "2026-01-01T00:00:01.000Z",
        message: { role: "user", content: "smoke fixture: hello" },
      },
      {
        type: "message",
        id: "smoke-assistant-1",
        parentId: "smoke-user-1",
        timestamp: "2026-01-01T00:00:02.000Z",
        message: {
          role: "assistant",
          content: [{ type: "text", text: "smoke fixture: hi" }],
          stopReason: "stop",
        },
      },
    ]
      .map((entry) => JSON.stringify(entry))
      .join("\n") + "\n",
  );
  console.log(`  端口 ${port}，临时 agent 目录 ${agentDir}`);

  const child = spawn(NODE_BIN, [ENTRY, "--no-open", "-p", String(port), "-H", "127.0.0.1"], {
    cwd: PKG_DIR,
    env: {
      ...process.env,
      PI_CODING_AGENT_DIR: agentDir,
      PI_WEB_NO_OPEN: "1",
      PI_WEB_SKIP_VERSION_CHECK: "1",
      PI_WEB_HOSTNAME: "127.0.0.1",
      LANG: "en_US.UTF-8",
      LC_ALL: "en_US.UTF-8",
      TERM: "xterm-256color",
    },
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
  });

  let output = "";
  child.stdout.on("data", (b) => (output += b.toString()));
  child.stderr.on("data", (b) => (output += b.toString()));
  let exited = false;
  child.on("exit", () => (exited = true));

  // 1. 等就绪
  const startedAt = Date.now();
  let status = 0;
  while (Date.now() - startedAt < READY_TIMEOUT_MS) {
    if (exited) break;
    status = await httpStatus(port, "/");
    if (status > 0) break;
    await sleep(150);
  }
  const elapsed = Date.now() - startedAt;

  if (status === 0 || exited) {
    bad(`服务未就绪（${elapsed}ms，status=${status}）`);
    console.log(output.split("\n").slice(-25).join("\n"));
  } else {
    ok(`GET / -> HTTP ${status}（${elapsed}ms）`);

    const sessionsStatus = await httpStatus(port, "/api/sessions");
    if (sessionsStatus === 200) {
      const body = await (await fetch(`http://127.0.0.1:${port}/api/sessions`)).json();
      if (Array.isArray(body.sessions)) {
        ok(`GET /api/sessions 正常（${body.sessions.length} 个会话）`);
        if (!body.sessions.some((session) => session.id === FIXTURE_ID)) {
          bad(`会话列表里没有 fixture 会话 ${FIXTURE_ID}（导出断言无法进行）`);
        }
      } else bad("/api/sessions 返回结构不对");
    } else {
      bad(`GET /api/sessions -> HTTP ${sessionsStatus}`);
    }

    // 导出接口（F18）：pi-web 生成 HTML 后会用字符串替换去 patch SDK 模板里的
    // sortChildren / mapNodes / markActive 三个递归函数，要求「恰好 1 处匹配」。
    // 一旦内置的 pi-web 与 pi SDK 版本组合对不上，首页正常但这个接口 500（B2）。
    try {
      const res = await fetch(
        `http://127.0.0.1:${port}/api/sessions/${FIXTURE_ID}/export?inline=1`,
        { signal: AbortSignal.timeout(60_000) },
      );
      const text = await res.text();
      const type = res.headers.get("content-type") ?? "";
      if (res.status === 200 && type.includes("text/html") && /<!doctype html/i.test(text)) {
        ok(`GET /api/sessions/<id>/export -> HTML（${(text.length / 1024).toFixed(1)} KB）`);
      } else {
        bad(`导出接口异常：HTTP ${res.status} ${type} ${text.slice(0, 200)}`);
      }
    } catch (error) {
      bad(`导出接口请求失败：${error.message}`);
    }

    // 2. run record 已写
    const records = fs.existsSync(runDir) ? fs.readdirSync(runDir).filter((n) => n.endsWith(".json")) : [];
    if (records.length === 1) ok(`run record 已写：${records[0]}`);
    else bad(`run record 数量异常：${records.length}`);
  }

  // 3. 优雅停止
  if (!exited) {
    if (process.platform === "win32") {
      try {
        execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
      } catch {
        /* ignore */
      }
    } else {
      try {
        process.kill(-child.pid, "SIGTERM");
      } catch {
        child.kill("SIGTERM");
      }
    }
  }

  const stopStart = Date.now();
  while (!exited && Date.now() - stopStart < STOP_TIMEOUT_MS) await sleep(100);
  if (exited) ok(`launcher 退出（${Date.now() - stopStart}ms）`);
  else {
    bad("SIGTERM 后 10s 未退出");
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      /* ignore */
    }
  }

  // 4. 收尾断言
  await sleep(500);
  if (await portOpen(port)) bad(`端口 ${port} 仍在监听（孤儿进程）`);
  else ok("端口已释放");

  const leftover = fs.existsSync(runDir) ? fs.readdirSync(runDir).filter((n) => n.endsWith(".json")) : [];
  if (leftover.length === 0) ok("run record 已清理");
  else bad(`run record 残留：${leftover.join(", ")}`);

  fs.rmSync(agentDir, { recursive: true, force: true });

  console.log(failures.length === 0 ? "[smoke] 通过" : `[smoke] 失败 ${failures.length} 项`);
  process.exit(failures.length === 0 ? 0 : 1);
}

main().catch((error) => {
  console.error("[smoke] 异常", error);
  process.exit(1);
});
