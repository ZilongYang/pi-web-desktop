#!/usr/bin/env node
/**
 * 让 pi-web 的「自定义消息」（extension 注入的 custom message，例如 mnemon 的
 * recall/remember 指令）默认折叠，只保留一行标题，点箭头才展开。
 *
 * 为什么需要：pi-web 前端渲染 custom message 的组件里写死了
 *   [expanded, setExpanded] = useState(!0)
 * 即默认展开，且折叠状态不持久化。pi-web 不提供相关设置项。
 * mnemon extension 每条用户消息都会注入一大段内容，展开会刷屏。
 *
 * 做法（幂等）：
 *   1. 在 static/chunks/app/page-*.js 里用 "markdown-custom-message" 定位自定义消息组件
 *   2. 把该组件里第一个 useState(!0) 改成 useState(!1)（展开状态初值）
 *   3. 内容变了 -> 用内容 sha256 前 16 位生成新文件名，并把 .next 下所有引用
 *      同长度替换（Next 的静态资源是 immutable 缓存一年，换 URL 才能失效）
 *
 * 用法：
 *   node scripts/patch-pi-web-collapse.mjs                 # patch sidecar/app
 *   node scripts/patch-pi-web-collapse.mjs --app-dir X     # patch 指定 app 目录
 *   node scripts/patch-pi-web-collapse.mjs --check         # 只检查，不写入
 */

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_APP_DIR = path.join(HERE, "..", "sidecar", "app");

const CUSTOM_RENDERER_ANCHOR = "markdown-custom-message";
const ON = "useState)(!0)";
const OFF = "useState)(!1)";

function log(message) {
  console.log(`[patch-collapse] ${message}`);
}

function piWebRoot(appDir) {
  return path.join(appDir, "node_modules", "@agegr", "pi-web");
}

/** 在 static/chunks/app 下找自定义消息组件所在的 page chunk。 */
function findPageChunks(piWebDir) {
  const dir = path.join(piWebDir, ".next", "static", "chunks", "app");
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir)
    .filter((name) => /^page-[0-9a-f]{16}\.js$/.test(name))
    .map((name) => path.join(dir, name));
}

/**
 * 从 anchor 往前倒序枚举 `function ` 边界，取「区间内恰好 1 个 ON、0 个 OFF」
 * 的那一段 —— 就是自定义消息组件本身。
 * 返回 { fnStart, hit } 或 null。
 */
function locateExpandedState(source) {
  const anchor = source.indexOf(CUSTOM_RENDERER_ANCHOR);
  if (anchor === -1) return null;

  let cursor = anchor;
  while (cursor > 0) {
    const fnStart = source.lastIndexOf("function ", cursor - 1);
    if (fnStart === -1) break;
    const segment = source.slice(fnStart, anchor);
    const onCount = segment.split(ON).length - 1;
    // 组件内只有「展开状态」初值为 !0，其余 state 本来就是 !1。
    if (onCount === 1) {
      return { fnStart, hit: fnStart + segment.indexOf(ON), segmentLength: segment.length };
    }
    // 已 patch 过的文件：这段里只剩 OFF，说明改过了。
    if (onCount === 0 && isPatchedComponent(source, fnStart, anchor)) {
      return { fnStart, hit: -1, alreadyPatched: true, segmentLength: segment.length };
    }
    cursor = fnStart;
  }
  return null;
}

function isPatchedComponent(source, fnStart, anchor) {
  const segment = source.slice(fnStart, anchor);
  const off = segment.indexOf(OFF);
  if (off === -1) return false;
  // 该 OFF 之前不应再有 useState 调用，才算我们改的那个（组件首个 state）。
  return !segment.slice(0, off).includes("useState)");
}

/** 递归收集 .next 下需要改引用的文件（跳过编译缓存）。 */
function collectRefFiles(root) {
  const skipDirs = new Set(["cache", "dev", "trace", "diagnostics", "types"]);
  const files = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (skipDirs.has(entry.name)) continue;
        walk(full);
      } else if (entry.isFile()) {
        files.push(full);
      }
    }
  };
  walk(root);
  return files;
}

/** 收集 .next 下除 app chunk 目录外的所有文本（用于判定哪些 chunk 还被引用）。 */
function readRefHaystack(nextDir) {
  const appChunkDir = path.join(nextDir, "static", "chunks", "app");
  const parts = [];
  for (const file of collectRefFiles(nextDir)) {
    if (file.startsWith(`${appChunkDir}${path.sep}`)) continue;
    try {
      parts.push(fs.readFileSync(file, "latin1"));
    } catch {
      /* 读不到就跳过 */
    }
  }
  return parts.join("\n");
}

/** 删除已无任何引用的 page chunk（改名后的旧文件）。 */
function cleanupStaleChunks(nextDir, keepPath) {
  const dir = path.join(nextDir, "static", "chunks", "app");
  const haystack = readRefHaystack(nextDir);
  const removed = [];
  for (const name of fs.readdirSync(dir)) {
    if (!/^page-[0-9a-f]{16}\.js$/.test(name)) continue;
    const full = path.join(dir, name);
    if (full === keepPath) continue;
    const hash = name.slice("page-".length, -".js".length);
    if (!haystack.includes(hash)) {
      fs.rmSync(full, { force: true });
      removed.push(name);
    }
  }
  return removed;
}

export function patchPiWebCollapse({ appDir = DEFAULT_APP_DIR, check = false } = {}) {
  const piWebDir = piWebRoot(appDir);
  const nextDir = path.join(piWebDir, ".next");
  if (!fs.existsSync(nextDir)) {
    throw new Error(`找不到 pi-web 构建产物：${nextDir}（先跑 npm run sidecar）`);
  }

  const chunks = findPageChunks(piWebDir);
  if (chunks.length === 0) throw new Error(`在 ${nextDir}/static/chunks/app 下没找到 page-*.js`);

  // 先整体探测：只要已有一个已打补丁的 chunk，就认为搞定（改名后旧文件可能还在）。
  for (const file of chunks) {
    const text = fs.readFileSync(file, "utf8");
    const found = locateExpandedState(text);
    if (found?.alreadyPatched) {
      const stale = check ? [] : cleanupStaleChunks(nextDir, file);
      log(`已 patch，跳过：${path.basename(file)}`);
      if (stale.length > 0) log(`清理了无引用的旧 chunk：${stale.join(", ")}`);
      return { patched: false, alreadyPatched: true, chunk: path.basename(file), staleRemoved: stale };
    }
  }

  let target = null;
  let source = null;
  let located = null;
  for (const file of chunks) {
    const text = fs.readFileSync(file, "utf8");
    const found = locateExpandedState(text);
    if (found) {
      target = file;
      source = text;
      located = found;
      break;
    }
  }
  if (!target) {
    throw new Error(
      `没能在 page chunk 里定位自定义消息组件（anchor "${CUSTOM_RENDERER_ANCHOR}"）。` +
        ` pi-web 结构可能已变，请更新本脚本。`,
    );
  }

  const oldName = path.basename(target);
  const oldHash = oldName.slice("page-".length, -".js".length);

  const patchedSource = source.slice(0, located.hit) + OFF + source.slice(located.hit + ON.length);
  const newHash = createHash("sha256").update(patchedSource).digest("hex").slice(0, 16);
  const newName = `page-${newHash}.js`;
  const newPath = path.join(path.dirname(target), newName);

  if (check) {
    log(`[check] 可 patch：${oldName} -> ${newName}（展开初值 true -> false）`);
    return { patched: false, dryRun: true, chunk: oldName, nextChunk: newName };
  }

  fs.writeFileSync(newPath, patchedSource);
  // 旧文件保留：它不是入口（引用已全部改指新 hash），留着可避免漏改导致 404。
  log(`写新 chunk：${newName}（旧文件保留）`);

  let touched = 0;
  for (const file of collectRefFiles(nextDir)) {
    if (file === newPath) continue;
    let text;
    try {
      text = fs.readFileSync(file, "latin1"); // 二进制安全：逐字节往返
    } catch {
      continue;
    }
    if (!text.includes(oldHash)) continue;
    const updated = text.split(oldHash).join(newHash);
    fs.writeFileSync(file, Buffer.from(updated, "latin1"));
    touched += 1;
  }
  log(`更新 ${touched} 个引用文件：${oldHash} -> ${newHash}`);

  // 校验旧 hash 是否还有残留引用；没残留就删旧 chunk，否则保留（宁可多一个文件，不要 404）。
  const leftovers = collectRefFiles(nextDir).filter((file) => {
    if (file === newPath || file === target) return false;
    try {
      return fs.readFileSync(file, "latin1").includes(oldHash);
    } catch {
      return false;
    }
  });
  if (leftovers.length === 0) {
    fs.rmSync(target, { force: true });
    log(`旧 chunk 已清理：${oldName}`);
  } else {
    log(`注意：${leftovers.length} 个文件仍引用旧 hash，保留 ${oldName} 作为兜底`);
  }
  return { patched: true, chunk: newName, previousChunk: oldName, refsUpdated: touched };
}

// ---- CLI ------------------------------------------------------------------
function main() {
  const argv = process.argv.slice(2);
  const getArg = (name) => {
    const i = argv.indexOf(name);
    return i === -1 ? null : argv[i + 1];
  };
  const appDir = getArg("--app-dir")
    ? path.resolve(getArg("--app-dir"))
    : DEFAULT_APP_DIR;

  const result = patchPiWebCollapse({ appDir, check: argv.includes("--check") });
  log(`${appDir} -> ${JSON.stringify(result)}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`[patch-collapse] 失败：${error.message}`);
    process.exit(1);
  }
}
