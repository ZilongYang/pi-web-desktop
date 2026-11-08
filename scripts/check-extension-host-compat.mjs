#!/usr/bin/env node
/**
 * 只读诊断：本机装了哪些 pi 扩展，它们是否在用 `ctx.mode` 推断宿主能力。
 *
 * 背景（见 docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md）：
 * pi-core 的扩展契约只有 `tui` / `rpc` / `json` / `print` 四档，
 * 且文档规定 rpc 模式 `custom()` 返回 undefined —— 插件据此在 `mode === "rpc"`
 * 时放弃自定义 UI 是"守约"。
 * 但 Pi Web Desktop 的 pi-web 是"超集宿主"：声明 rpc，却实现了 custom()。
 * 于是按契约写法的插件会在本 App 里静默降级（ask_user_question 就是如此）。
 *
 * 本脚本只读扫描 `~/.pi/agent/settings.json` 里启用的扩展，找出：
 *   1. `mode === "rpc"` / `mode !== "tui"` / `mode === "tui"` 这类能力闸门
 *   2. `ui.custom(...)` 调用点
 *   3. `onTerminalInput` 依赖（pi-web 里是空实现）
 *   4. 自带的能力覆盖开关（如 PI_SUBAGENTS_COMPONENT_UI=1）
 * 输出一张"候选风险清单"供人工判定，不做任何写操作。
 *
 * 用法：
 *   node scripts/check-extension-host-compat.mjs             # 人读表格
 *   node scripts/check-extension-host-compat.mjs --json      # 机器可读
 *   PI_AGENT_DIR=/path/to/agent node scripts/check-extension-host-compat.mjs
 *
 * 退出码：0（诊断而已，发现风险也返回 0）。
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const AGENT_DIR = process.env.PI_AGENT_DIR ?? path.join(os.homedir(), ".pi", "agent");
const NPM_DIR = path.join(AGENT_DIR, "npm");
const SETTINGS = path.join(AGENT_DIR, "settings.json");
const JSON_OUT = process.argv.includes("--json");

const SKIP_DIRS = new Set([
	"node_modules",
	".git",
	".next",
	"docs",
	"locales",
	"coverage",
	"target",
	"generated",
	"images",
	"schemas",
]);
const MAX_FILE_BYTES = 2_500_000;

const PATTERNS = [
	{ key: "modeRpc", label: 'mode === "rpc"', re: /(?:ctx\.mode|\bmode)\s*(?:===|!==|==|!=)\s*["']rpc["']/ },
	{ key: "modeTui", label: 'mode === "tui"', re: /(?:ctx\.mode|\bmode)\s*(?:===|!==|==|!=)\s*["']tui["']/ },
	{ key: "customCall", label: "ui.custom(...)", re: /\.custom\s*[<(]/ },
	{ key: "terminalInput", label: "onTerminalInput", re: /onTerminalInput/ },
	{ key: "capabilityEnv", label: "能力覆盖环境变量", re: /process\.env\.[A-Z0-9_]+/ }, // 下面再按 _UI/_COMPONENT/_CUSTOM 边界过滤，避免 GUIDE 之类误报
];

function readSettings() {
	if (!fs.existsSync(SETTINGS)) {
		console.error(`[compat] 找不到 ${SETTINGS}`);
		process.exit(1);
	}
	return JSON.parse(fs.readFileSync(SETTINGS, "utf8"));
}

/** 把 settings.packages 的条目解析成（类型, 路径）。只处理会真正被加载的条目。 */
function resolvePackages(settings) {
	const out = [];
	for (const entry of settings.packages ?? []) {
		const source = typeof entry === "string" ? entry : entry?.source;
		if (typeof source !== "string" || source.length === 0) continue;
		if (source.startsWith("npm:")) {
			const spec = source.slice(4);
			// npm:pkg / npm:pkg@1.2.3 → node_modules/pkg
			const dir = spec.startsWith("@")
				? spec.split("/").slice(0, 2).join("/").replace(/@[^/]*$/, "")
				: spec.replace(/@.*$/, "");
			out.push({ kind: "npm", source, dir: path.join(NPM_DIR, "node_modules", dir) });
		} else {
			out.push({ kind: "local", source, dir: path.resolve(AGENT_DIR, source) });
		}
	}
	return out;
}

function listSourceFiles(pkgDir) {
	const files = [];
	const walk = (dir, depth) => {
		if (depth > 6) return;
		let entries;
		try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
		for (const e of entries) {
			const p = path.join(dir, e.name);
			if (e.isDirectory()) {
				if (!SKIP_DIRS.has(e.name)) walk(p, depth + 1);
				continue;
			}
			if (!/\.(ts|tsx|js|mjs|cjs)$/.test(e.name)) continue;
			let st; try { st = fs.statSync(p); } catch { continue; }
			if (st.size > MAX_FILE_BYTES) continue;
			files.push(p);
		}
	};
	walk(pkgDir, 0);
	return files;
}

function scanPackage(pkg) {
	const pkgJsonPath = path.join(pkg.dir, "package.json");
	if (!fs.existsSync(pkgJsonPath)) return { ...pkg, missing: true, hits: [] };
	let meta = {};
	try { meta = JSON.parse(fs.readFileSync(pkgJsonPath, "utf8")); } catch {}
	const files = listSourceFiles(pkg.dir);
	const hits = [];
	for (const file of files) {
		let text;
		try { text = fs.readFileSync(file, "utf8"); } catch { continue; }
		const lines = text.split("\n");
		for (const pat of PATTERNS) {
			for (let i = 0; i < lines.length; i++) {
				const line = lines[i];
				if (!pat.re.test(line)) continue;
				if (pat.key === "capabilityEnv") {
					const m = line.match(/process\.env\.([A-Z0-9_]+)/);
					const token = m?.[1] ?? "";
					if (!/(^|_)(UI|COMPONENT|CUSTOM)(_|$)/.test(token)) continue;
				}
				const trimmed = line.trim().slice(0, 160);
				// 纯注释行也保留（作者注释常常是最有价值的意图证据），单独标记
				const comment = /^\s*(\/\/|\*|\/\*)/.test(line);
				hits.push({
					pattern: pat.key,
					label: pat.label,
					file: path.relative(pkg.dir, file),
					line: i + 1,
					comment,
					text: trimmed,
				});
			}
		}
	}
	// dist/ 与 src/ 常常是同一份代码的两份拷贝：按 pattern + 行文本去重，避免重复计数
	const seen = new Set();
	const uniqueHits = [];
	for (const h of hits) {
		const key = `${h.pattern}|${h.text}`;
		if (seen.has(key)) continue;
		seen.add(key);
		uniqueHits.push(h);
	}
	const count = (key) => uniqueHits.filter((h) => h.pattern === key && !h.comment).length;
	const summary = {
		modeRpc: count("modeRpc"),
		modeTui: count("modeTui"),
		customCall: count("customCall"),
		terminalInput: count("terminalInput"),
		capabilityEnv: count("capabilityEnv"),
	};
	const flags = [];
	if (summary.modeRpc > 0) flags.push("按契约在 rpc 下放弃 custom（Pi Web 里会降级）");
	if (summary.modeTui > 0) flags.push("非 tui 时走降级/拒绝分支（Pi Web 里走降级侧）");
	if (summary.capabilityEnv > 0) flags.push("自带能力覆盖环境变量（生态已有的临时解法）");
	if (summary.customCall > 0 && summary.modeRpc === 0 && summary.modeTui === 0) flags.push("直接调用 custom，未按 mode 设闸（Pi Web 里可能已能用）");
	if (summary.terminalInput > 0) flags.push("依赖 onTerminalInput（Pi Web 里是空实现）");
	return {
		...pkg,
		version: meta.version ?? "?",
		missing: false,
		fileCount: files.length,
		summary,
		flags,
		hits: uniqueHits,
	};
}

function main() {
	const settings = readSettings();
	const packages = resolvePackages(settings);
	const results = packages.map(scanPackage);
	const risky = results.filter((r) => r.flags.length > 0);

	if (JSON_OUT) {
		console.log(JSON.stringify({ agentDir: AGENT_DIR, packages: results }, null, 2));
		return;
	}

	console.log(`[compat] agent 目录：${AGENT_DIR}`);
	console.log(`[compat] settings.json 里启用 ${packages.length} 个扩展；其中 ${risky.length} 个存在宿主能力相关用法。\n`);
	console.log("包".padEnd(42) + "版本".padEnd(12) + "rpc/tui闸门".padEnd(12) + "custom  onTermInput  判定");
	console.log("-".repeat(120));
	for (const r of results) {
		const gate = `${r.summary.modeRpc}/${r.summary.modeTui}`;
		console.log(
			(r.source ?? r.dir).padEnd(42) +
				String(r.version).padEnd(12) +
				gate.padEnd(12) +
				String(r.summary.customCall).padEnd(7) +
				String(r.summary.terminalInput).padEnd(13) +
				(r.flags.length ? r.flags[0] : "—"),
		);
	}
	if (risky.length > 0) {
		console.log("\n证据行（每包最多 4 条，注释行带 *）：");
		for (const r of risky) {
			console.log(`\n# ${r.source ?? r.dir}`);
			const shown = r.hits.filter((h) => h.pattern !== "customCall").slice(0, 3);
			const customHits = r.hits.filter((h) => h.pattern === "customCall").slice(0, 1);
			for (const h of [...shown, ...customHits]) {
				console.log(`  ${h.comment ? "*" : " "} ${h.file}:${h.line}  ${h.text}`);
			}
		}
	}
	console.log("\n提醒：这只是候选清单，是否真的降级/有损要结合该插件是否有 fallback 人工判定；");
	console.log("      参考 docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md 的 §12 影响面审计。");
}

main();
