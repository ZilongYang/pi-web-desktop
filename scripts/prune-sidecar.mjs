#!/usr/bin/env node
/**
 * prune-sidecar.mjs -- 侧车裁剪：删掉运行时用不到的东西，只留当前平台的二进制。
 *
 * 原则：只删「确定不影响 next start 运行」的内容；每步之后都要跑 smoke。
 *   - 所有 *.map、node_modules 里的文档
 *   - sharp / @img（pi-web 的 next.config 里 images.unoptimized = true，不走优化器）
 *   - 非当前平台的 @next/swc-*
 *   - node-pty 里非当前平台的 prebuilds
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const SWC_BY_TARGET = {
  "darwin-arm64": "darwin-arm64",
  "darwin-x64": "darwin-x64",
  "linux-x64": "linux-x64-gnu",
  "linux-arm64": "linux-arm64-gnu",
  "win32-x64": "win32-x64-msvc",
  "win32-arm64": "win32-arm64-msvc",
};

const PTY_BY_TARGET = {
  "darwin-arm64": "darwin-arm64",
  "darwin-x64": "darwin-x64",
  "linux-x64": "linux-x64",
  "linux-arm64": "linux-arm64",
  "win32-x64": "win32-x64",
  "win32-arm64": "win32-arm64",
};

function sizeOf(dir) {
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
      if (entry.isDirectory() && !entry.isSymbolicLink()) walk(full);
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

function rm(target, removed, label) {
  let stat;
  try {
    stat = fs.lstatSync(target);
  } catch {
    return 0;
  }
  const bytes = stat.isDirectory() ? sizeOf(target).total : stat.size;
  fs.rmSync(target, { recursive: true, force: true });
  removed.push(`${label} (${(bytes / 1024 ** 2).toFixed(0)}MB)`);
  return bytes;
}

function isDocMarkdown(name) {
  if (!name.toLowerCase().endsWith(".md")) return false;
  if (/^(license|licence|copying|notice|authors|patents)/i.test(name)) return false;
  return true;
}

export function pruneSidecar({ appDir, target }) {
  const removed = [];
  const nm = path.join(appDir, "node_modules");

  // 1. source map
  const walkFiles = (dir, visit) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory() && !entry.isSymbolicLink()) walkFiles(full, visit);
      else visit(full, entry);
    }
  };
  let mapBytes = 0;
  let mapCount = 0;
  walkFiles(nm, (full, entry) => {
    if (entry.isFile() && entry.name.endsWith(".map")) {
      try {
        mapBytes += fs.statSync(full).size;
        fs.rmSync(full, { force: true });
        mapCount += 1;
      } catch {
        /* ignore */
      }
    }
  });
  if (mapCount) removed.push(`source maps ×${mapCount} (${(mapBytes / 1024 ** 2).toFixed(0)}MB)`);

  // 2. 包内文档
  let docBytes = 0;
  let docCount = 0;
  walkFiles(nm, (full, entry) => {
    if (entry.isFile() && isDocMarkdown(entry.name)) {
      try {
        docBytes += fs.statSync(full).size;
        fs.rmSync(full, { force: true });
        docCount += 1;
      } catch {
        /* ignore */
      }
    }
  });
  if (docCount) removed.push(`文档 ×${docCount} (${(docBytes / 1024 ** 2).toFixed(0)}MB)`);

  // 3. sharp（next/image 优化器不用；pi-web 显式 unoptimized）
  for (const name of ["sharp", "@img"]) rm(path.join(nm, name), removed, `node_modules/${name}`);

  // 4. 非本平台 @next/swc-*
  const keepSwc = SWC_BY_TARGET[target];
  const nextScope = path.join(nm, "@next");
  if (keepSwc && fs.existsSync(nextScope)) {
    for (const name of fs.readdirSync(nextScope)) {
      if (!name.startsWith("swc-")) continue;
      if (name === `swc-${keepSwc}`) continue;
      rm(path.join(nextScope, name), removed, `@next/${name}（非本平台）`);
    }
  }

  // 5. node-pty 非本平台 prebuilds
  const keepPty = PTY_BY_TARGET[target];
  const prebuilds = path.join(nm, "node-pty", "prebuilds");
  if (keepPty && fs.existsSync(prebuilds)) {
    for (const name of fs.readdirSync(prebuilds)) {
      if (name === keepPty) continue;
      rm(path.join(prebuilds, name), removed, `node-pty/prebuilds/${name}`);
    }
  }

  return { removed, after: sizeOf(appDir) };
}

// 直接执行时对 sidecar/app 裁剪
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const target = process.argv.includes("--target")
    ? process.argv[process.argv.indexOf("--target") + 1]
    : `${process.platform}-${process.arch}`;
  const appDir = path.join(ROOT, "sidecar", "app");
  if (!fs.existsSync(appDir)) {
    console.error(`没有找到 ${appDir}，先跑 npm run sidecar`);
    process.exit(1);
  }
  const before = sizeOf(appDir);
  const result = pruneSidecar({ appDir, target });
  console.log(`裁剪 ${before.total} -> ${result.after.total} 字节`);
  for (const item of result.removed) console.log(`  - ${item}`);
}
