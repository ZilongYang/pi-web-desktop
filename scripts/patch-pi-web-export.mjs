#!/usr/bin/env node
/**
 * 让 pi-web 的「导出会话 HTML」在 SDK 模板已是迭代实现时也能工作。
 *
 * 为什么需要（ROADMAP B2）：pi-web 0.14.2 的导出路由
 *   .next/server/app/api/sessions/[id]/export/route.js
 * 在生成 HTML 后，用字符串替换把 SDK 模板里的三个递归函数
 *   sortChildren / mapNodes / markActive
 * 换成迭代版本（防深会话爆栈）。替换辅助函数要求**恰好 1 处匹配**，
 * 0 处就 throw：
 *   Failed to patch exported HTML: sortChildren expected 1 match, found 0
 *
 * 而内置 pi SDK 1.5.0 的 dist/core/export-html/template.js 里这三个函数已经
 * 不存在了（上游改成了迭代实现，`stack` 出现 19 次），于是三个补丁全部 0 匹配，
 * 导出接口直接 500。0.14.1 + SDK 1.4.0 的组合才匹配 3 处。
 *
 * 做法（幂等，只改一处条件）：
 *   `if(1!==g)throw Error(...)`  ->  `if(g>1)throw Error(...)`
 *   · g === 0：模板已是迭代实现，`a.replace(e,f)` 天然是 no-op，正确；
 *   · g === 1：正常替换；
 *   · g  > 1：锚点不再唯一，仍然报错（真正的异常）。
 *
 * 上游之后如果重构掉这套字符串补丁，本脚本会判定「无需补丁」并跳过；
 * 如果锚点形状变了但对不上，则**报错退出**，避免静默打出坏包。
 *
 * 用法：
 *   node scripts/patch-pi-web-export.mjs                 # patch sidecar/app
 *   node scripts/patch-pi-web-export.mjs --app-dir X     # patch 指定 app 目录
 *   node scripts/patch-pi-web-export.mjs --check         # 只检查，不写入
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_APP_DIR = path.join(HERE, "..", "sidecar", "app");

/** 导出路由相对 pi-web 包根的位置。 */
const ROUTE_REL = [".next", "server", "app", "api", "sessions", "[id]", "export", "route.js"];

/** 需要放宽的断言：恰好 1 处匹配。 */
const ASSERTION = /if\(1!==(\w+)\)(throw Error\(`Failed to patch exported HTML:)/;
/** 已打过本补丁的痕迹。 */
const PATCHED = /if\((\w+)>1\)throw Error\(`Failed to patch exported HTML:/;
/** 整个字符串补丁机制的痕迹（上游若删除它，本脚本无事可做）。 */
const PATCH_MECHANISM = "Failed to patch exported HTML";

function log(message) {
  console.log(`[patch-export] ${message}`);
}

function routePath(appDir) {
  return path.join(appDir, "node_modules", "@agegr", "pi-web", ...ROUTE_REL);
}

/**
 * @returns {{patched:boolean, alreadyPatched?:boolean, skipped?:boolean, matches?:number, route?:string}}
 */
export function patchPiWebExport({ appDir = DEFAULT_APP_DIR, check = false } = {}) {
  const route = routePath(appDir);
  if (!fs.existsSync(route)) {
    // 上游可能把路由挪走/删了：没有就没什么可补的，但要留痕。
    log(`没找到导出路由（${route}），跳过`);
    return { patched: false, skipped: true, route };
  }

  const source = fs.readFileSync(route, "utf8");

  const patchedMarkers = source.match(new RegExp(PATCHED.source, "g")) ?? [];
  if (patchedMarkers.length > 0) {
    if (source.includes("if(1!==")) {
      throw new Error(
        `导出路由里同时存在已打标记与原始的 "if(1!==" —— 状态不明确，请人工检查：${route}`,
      );
    }
    log(`已 patch，跳过（${route}）`);
    return { patched: false, alreadyPatched: true, matches: patchedMarkers.length, route };
  }

  const matches = source.match(new RegExp(ASSERTION.source, "g")) ?? [];
  if (matches.length === 0) {
    if (!source.includes(PATCH_MECHANISM)) {
      // 上游已整体移除这套字符串补丁 -> 不再需要我们。
      log(`上游已不再使用字符串补丁，无需处理（${route}）`);
      return { patched: false, skipped: true, route };
    }
    throw new Error(
      `找到了字符串补丁机制，但对不上 "if(1!==…)throw Error(...)" 锚点；` +
        `上游可能改了写法，请更新 scripts/patch-pi-web-export.mjs（${route}）`,
    );
  }
  if (matches.length > 1) {
    throw new Error(`锚点出现 ${matches.length} 次，预期 1 次；请人工检查：${route}`);
  }

  if (check) {
    log(`[check] 可 patch：1!==g -> g>1（${route}）`);
    return { patched: false, dryRun: true, matches: 1, route };
  }

  const next = source.replace(ASSERTION, (_all, count, throwPart) => `if(${count}>1)${throwPart}`);
  if (next === source) throw new Error(`替换未生效（${route}）`);
  if (next.includes("if(1!==")) throw new Error(`替换后仍残留 "if(1!=="（${route}）`);
  fs.writeFileSync(route, next);
  log(`已 patch：容忍 0 匹配（${route}）`);
  return { patched: true, matches: 1, route };
}

// ---- CLI ------------------------------------------------------------------
function main() {
  const argv = process.argv.slice(2);
  const getArg = (name) => {
    const i = argv.indexOf(name);
    return i === -1 ? null : argv[i + 1];
  };
  const appDir = getArg("--app-dir") ? path.resolve(getArg("--app-dir")) : DEFAULT_APP_DIR;

  const result = patchPiWebExport({ appDir, check: argv.includes("--check") });
  log(`${appDir} -> ${JSON.stringify(result)}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`[patch-export] 失败：${error.message}`);
    process.exit(1);
  }
}
