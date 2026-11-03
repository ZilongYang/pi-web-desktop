#!/usr/bin/env node
/**
 * fetch-sidecar.mjs -- 把「运行时 + pi-web 包」装到 sidecar/ 下。
 *
 *   sidecar/
 *     runtime/node(.exe)      官方 Node 二进制（只取 node 本体）
 *     app/                    npm install @agegr/pi-web@<pin> 的结果
 *     manifest.json           版本/平台/体积/校验摘要
 *
 * 用法:
 *   node scripts/fetch-sidecar.mjs                 # 装/重装
 *   node scripts/fetch-sidecar.mjs --if-missing    # 已有且版本匹配就跳过（dev/build 用）
 *   node scripts/fetch-sidecar.mjs --force         # 强制重装
 *   node scripts/fetch-sidecar.mjs --target darwin-arm64
 *   node scripts/fetch-sidecar.mjs --no-prune      # 不裁剪（排查用）
 *   node scripts/fetch-sidecar.mjs --no-patch      # 不打「默认折叠」「导出 HTML」两个补丁
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { pruneSidecar } from "./prune-sidecar.mjs";
import { patchPiWebCollapse } from "./patch-pi-web-collapse.mjs";
import { patchPiWebExport } from "./patch-pi-web-export.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SIDECAR = path.join(ROOT, "sidecar");
const APP_DIR = path.join(SIDECAR, "app");
const RUNTIME_DIR = path.join(SIDECAR, "runtime");
const MANIFEST = path.join(SIDECAR, "manifest.json");

const args = process.argv.slice(2);
const has = (flag) => args.includes(flag);
const argValue = (flag) => {
  const i = args.indexOf(flag);
  return i >= 0 ? args[i + 1] : undefined;
};

// sidecar.json 是唯一的版本源
const pins = JSON.parse(fs.readFileSync(path.join(ROOT, "sidecar.json"), "utf8"));

// 目标平台 -> Node 发行包命名
const TARGETS = {
  "darwin-arm64": { node: "darwin-arm64", ext: "tar.gz" },
  "darwin-x64": { node: "darwin-x64", ext: "tar.gz" },
  "linux-x64": { node: "linux-x64", ext: "tar.gz" },
  "linux-arm64": { node: "linux-arm64", ext: "tar.gz" },
  "win32-x64": { node: "win-x64", ext: "zip" },
  "win32-arm64": { node: "win-arm64", ext: "zip" },
};

const target = argValue("--target") ?? `${process.platform}-${process.arch}`;
const targetInfo = TARGETS[target];
if (!targetInfo) {
  console.error(`不支持的 target: ${target}（可选: ${Object.keys(TARGETS).join(", ")}）`);
  process.exit(1);
}

function log(msg) {
  console.log(`[sidecar] ${msg}`);
}

function human(bytes) {
  if (bytes > 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)}GB`;
  if (bytes > 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(0)}MB`;
  return `${(bytes / 1024).toFixed(0)}KB`;
}

function dirSize(dir) {
  let total = 0;
  let files = 0;
  const walk = (d) => {
    let entries;
    try {
      entries = fs.readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) {
        files += 1;
        try {
          total += fs.statSync(full).size;
        } catch {
          /* ignore */
        }
      }
    }
  };
  walk(dir);
  return { total, files };
}

function readManifest() {
  try {
    return JSON.parse(fs.readFileSync(MANIFEST, "utf8"));
  } catch {
    return null;
  }
}

// ---- 跳过判断 -------------------------------------------------------------
const existing = readManifest();
const uptodate =
  existing &&
  existing.piWebVersion === pins.piWebVersion &&
  existing.nodeVersion === pins.nodeVersion &&
  existing.target === target &&
  fs.existsSync(path.join(APP_DIR, "node_modules", "@agegr", "pi-web", "package.json")) &&
  fs.existsSync(path.join(RUNTIME_DIR, process.platform === "win32" ? "node.exe" : "node"));

if (has("--if-missing") && uptodate && !has("--force")) {
  // 已装好也可能缺补丁（例如补丁脚本是后加的），先兜底对齐。
  if (fs.existsSync(path.join(APP_DIR, "node_modules", "@agegr", "pi-web", ".next"))) {
    applyPatches("跳过前校验");
  }
  log(`已就绪，跳过：pi-web ${pins.piWebVersion} / node ${pins.nodeVersion} / ${target}`);
  process.exit(0);
}

/**
 * 安装 pi-web 后要打的补丁，统一在这里调用（--no-patch 全部跳过）。
 *
 *  1. 自定义消息默认折叠（patch-pi-web-collapse.mjs）—— 纯 UI 偏好。
 *  2. 导出 HTML 容忍 0 匹配（patch-pi-web-export.mjs）—— 修 B2：pi-web 0.14.2 的
 *     导出路由要求模板里三个递归函数各命中 1 处，而 SDK 1.5.0 已改成迭代实现，
 *     导致导出接口 500。
 */
function applyPatches(phase) {
  applyCollapsePatch(phase);
  applyExportPatch(phase);
}

/**
 * pi-web 渲染 extension 注入的 custom message 时默认展开，且无设置项。
 * mnemon 每轮都会注入一大段指令，展开会刷屏 —— 这里把它改成默认折叠。
 * 见 scripts/patch-pi-web-collapse.mjs。
 */
function applyCollapsePatch(phase) {
  if (has("--no-patch")) {
    log(`${phase}：--no-patch，跳过「自定义消息默认折叠」补丁`);
    return;
  }
  try {
    const result = patchPiWebCollapse({ appDir: APP_DIR });
    if (result.patched) {
      log(`${phase}：折叠补丁已应用 -> ${result.chunk}（${result.refsUpdated} 个引用）`);
    } else {
      log(`${phase}：折叠补丁已在位（${result.chunk}）`);
    }
  } catch (error) {
    console.error(`[patch-collapse] ${phase}失败：${error.message}`);
    console.error("  修复 patch-pi-web-collapse.mjs 后重跑；或用 --no-patch 跳过（跳过时 UI 会保持默认展开）。");
    process.exit(1);
  }
}

/**
 * 修 B2：pi-web 0.14.2 的导出路由用字符串替换把 SDK 模板里的
 * sortChildren / mapNodes / markActive 换成迭代实现，且要求恰好 1 处匹配；
 * 而 SDK 1.5.0 的模板已经是迭代实现（0 处匹配）-> 导出接口 500。
 * 见 scripts/patch-pi-web-export.mjs。
 */
function applyExportPatch(phase) {
  if (has("--no-patch")) {
    log(`${phase}：--no-patch，跳过「导出 HTML 容忍 0 匹配」补丁`);
    return;
  }
  try {
    const result = patchPiWebExport({ appDir: APP_DIR });
    if (result.patched) {
      log(`${phase}：导出补丁已应用（${result.route}）`);
    } else if (result.skipped) {
      log(`${phase}：导出补丁无需处理（${result.route}）`);
    } else {
      log(`${phase}：导出补丁已在位`);
    }
  } catch (error) {
    console.error(`[patch-export] ${phase}失败：${error.message}`);
    console.error("  修复 patch-pi-web-export.mjs 后重跑；或用 --no-patch 跳过（跳过时导出接口仍会 500）。");
    process.exit(1);
  }
}

// ---- 1. npm 装 pi-web ------------------------------------------------------
log(`安装 @agegr/pi-web@${pins.piWebVersion} -> sidecar/app …`);
fs.mkdirSync(APP_DIR, { recursive: true });
const npmCmd = process.platform === "win32" ? "npm.cmd" : "npm";
execFileSync(
  npmCmd,
  [
    "install",
    `@agegr/pi-web@${pins.piWebVersion}`,
    "--omit=dev",
    "--no-audit",
    "--no-fund",
    "--no-progress",
    "--prefix",
    APP_DIR,
  ],
  { stdio: "inherit", cwd: APP_DIR, env: process.env },
);

// ---- 2. 下载官方 Node，只取 node 本体 --------------------------------------
const nodeAsset = `node-v${pins.nodeVersion}-${targetInfo.node}.${targetInfo.ext}`;
const base = `${pins.nodeDownloadBase}/v${pins.nodeVersion}`;
const nodeBinName = process.platform === "win32" ? "node.exe" : "node";

function sha256(file) {
  return createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

async function download(url, dest) {
  log(`下载 ${url}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`下载失败 ${url}: HTTP ${res.status}`);
  const buf = Buffer.from(await res.arrayBuffer());
  fs.writeFileSync(dest, buf);
  return buf;
}

const wantShasums = fs.existsSync(path.join(RUNTIME_DIR, nodeBinName)) === false;
if (wantShasums) fs.mkdirSync(RUNTIME_DIR, { recursive: true });

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "piweb-node-"));
try {
  const shasumsPath = path.join(tmp, "SHASUMS256.txt");
  await download(`${base}/SHASUMS256.txt`, shasumsPath);
  const expected = fs
    .readFileSync(shasumsPath, "utf8")
    .split("\n")
    .map((line) => line.trim().split(/\s+/))
    .find(([, name]) => name === nodeAsset)?.[0];
  if (!expected) throw new Error(`SHASUMS256.txt 里找不到 ${nodeAsset}`);

  const archivePath = path.join(tmp, nodeAsset);
  await download(`${base}/${nodeAsset}`, archivePath);
  const actual = sha256(archivePath);
  if (actual !== expected) throw new Error(`校验失败 ${nodeAsset}: ${actual} != ${expected}`);
  log(`校验通过 ${nodeAsset} (${human(fs.statSync(archivePath).size)})`);

  // 解压
  if (targetInfo.ext === "tar.gz") {
    execFileSync("tar", ["-xzf", archivePath, "-C", tmp], { stdio: "inherit" });
    const extracted = fs.readdirSync(tmp).find((n) => n.startsWith(`node-v${pins.nodeVersion}-`));
    const src = path.join(tmp, extracted, "bin", "node");
    fs.copyFileSync(src, path.join(RUNTIME_DIR, "node"));
    fs.chmodSync(path.join(RUNTIME_DIR, "node"), 0o755);
  } else {
    execFileSync(
      "powershell",
      ["-NoProfile", "-Command", `Expand-Archive -Path '${archivePath}' -DestinationPath '${tmp}' -Force`],
      { stdio: "inherit" },
    );
    const extracted = fs.readdirSync(tmp).find((n) => n.startsWith(`node-v${pins.nodeVersion}-`));
    const src = path.join(tmp, extracted, "node.exe");
    fs.copyFileSync(src, path.join(RUNTIME_DIR, "node.exe"));
  }
  log(`运行时就绪 sidecar/runtime/${nodeBinName}`);
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}

// ---- 3. 裁剪 ---------------------------------------------------------------
let before;
if (!has("--no-prune")) {
  before = dirSize(SIDECAR);
  log(`裁剪前 ${human(before.total)} / ${before.files} 文件`);
  const result = pruneSidecar({ appDir: APP_DIR, target });
  log(`裁剪后 ${human(result.after.total)} / ${result.after.files} 文件（省 ${human(before.total - result.after.total)}）`);
  for (const item of result.removed) log(`  - ${item}`);
} else {
  before = dirSize(SIDECAR);
}

// ---- 3.5 打 pi-web 补丁（默认折叠 + 导出容忍 0 匹配） ----------------------
applyPatches("安装后");

// ---- 4. manifest ----------------------------------------------------------
const nodeBin = path.join(RUNTIME_DIR, nodeBinName);
const after = dirSize(SIDECAR);
let piSdkVersion = "";
try {
  piSdkVersion = JSON.parse(
    fs.readFileSync(
      path.join(APP_DIR, "node_modules", "@earendil-works", "pi-coding-agent", "package.json"),
      "utf8",
    ),
  ).version;
} catch {
  /* 读不到就留空，启动时再从包内读 */
}

const manifest = {
  piWebVersion: pins.piWebVersion,
  piSdkVersion,
  nodeVersion: pins.nodeVersion,
  nodeVersionActual: execFileSync(nodeBin, ["--version"], { encoding: "utf8" }).trim(),
  target,
  createdAt: new Date().toISOString(),
  bytes: after.total,
  files: after.files,
  nodeSha256: sha256(nodeBin),
};
fs.writeFileSync(MANIFEST, `${JSON.stringify(manifest, null, 2)}\n`);

log("完成");
log(`  pi-web ${manifest.piWebVersion} · pi SDK ${manifest.piSdkVersion || "?"} · node ${manifest.nodeVersionActual}`);
log(`  侧车总体积 ${human(after.total)} / ${after.files} 文件`);
log(`  manifest: sidecar/manifest.json`);
assertNodeVersionOk(manifest.nodeVersionActual);

function assertNodeVersionOk(versionString) {
  const [major, minor] = versionString.replace(/^v/, "").split(".").map(Number);
  if (major > 22 || (major === 22 && minor >= 19)) return;
  throw new Error(`Node ${versionString} 低于 pi-web 要求的 22.19.0`);
}
