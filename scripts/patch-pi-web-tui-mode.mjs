#!/usr/bin/env node
/**
 * 【实验开关】让 Pi Web Desktop 的宿主 pi-web 把扩展绑定模式从 "rpc" 改成 "tui"。
 *
 * 背景：pi-core 的扩展契约只有 tui/rpc 两档（rpc 本应 custom() 返回 undefined），
 * 而 pi-web 声明 rpc 却实现了 custom()。多数插件按契约在 rpc 下走降级分支
 * （见 scripts/check-extension-host-compat.mjs 的扫描结果）。
 * 这个脚本把 pi-web 的 `mode:"rpc"`（以及旧版兼容分支 setUIContext 的 "rpc"）
 * 改成 "tui"，用来实测「pi-web 能不能当 TUI 宿主」：
 *   - 好处：所有按 mode 闸门的插件都会走完整的 TUI/custom 路径
 *   - 风险：pi-web 的 TUI 是受限子集（onTerminalInput 空实现、无鼠标、无 overlay handle、
 *     固定 92×40），有些插件可能「看起来能开但交互缺失」，甚至等不到输入而卡住。
 *     所以它是**实验**，默认不用于构建；验证完请 --revert。
 *
 * 目标目录：
 *   默认 = 本仓库 sidecar/app（用于跑本地 pi-web 做实验，不影响 /Applications 里的 App）
 *   --app-dir <path> = 指定别的 pi-web 安装，例如
 *       --app-dir "/Applications/Pi Web Desktop.app/Contents/Resources/pi-web/app"
 *       （注意：改 /Applications 里的 App 会让 ad-hoc 签名失效、且 App 升级会覆盖；
 *        非必要不建议；真机实验优先用 sidecar + 浏览器）
 *
 * 用法：
 *   node scripts/patch-pi-web-tui-mode.mjs            # 打实验补丁
 *   node scripts/patch-pi-web-tui-mode.mjs --check    # 只看状态（已打=0，未打=2）
 *   node scripts/patch-pi-web-tui-mode.mjs --revert   # 还原
 *   node scripts/patch-pi-web-tui-mode.mjs --app-dir <pi-web app 目录>
 *
 * 打完重启 pi-web / App 才生效。实验步骤与验收清单见
 * docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md §13。
 */
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const BACKUP_SUFFIX = ".bak-tuimode";

const args = process.argv.slice(2);
const argValue = (flag) => {
	const i = args.indexOf(flag);
	return i >= 0 ? args[i + 1] : undefined;
};
const MODE = args.includes("--revert") ? "revert" : args.includes("--check") ? "check" : "patch";
const APP_DIR = path.resolve(argValue("--app-dir") ?? path.join(ROOT, "sidecar", "app"));
const PIWEB_DIR = path.join(APP_DIR, "node_modules", "@agegr", "pi-web");
const CHUNKS_DIR = path.join(PIWEB_DIR, ".next", "server", "chunks");

function fail(msg) {
	console.error(`[tui-mode] ${msg}`);
	process.exit(1);
}
if (!fs.existsSync(PIWEB_DIR)) fail(`找不到 pi-web：${PIWEB_DIR}（先 npm run sidecar）`);
const version = (() => {
	try {
		return JSON.parse(fs.readFileSync(path.join(PIWEB_DIR, "package.json"), "utf8")).version ?? "?";
	} catch {
		return "?";
	}
})();

/** 找出所有含有扩展绑定 mode 的文件，并定位需要改的字面量。 */
function scanFiles() {
	const files = fs.readdirSync(CHUNKS_DIR).filter((n) => n.endsWith(".js"));
	const found = [];
	for (const name of files) {
		const file = path.join(CHUNKS_DIR, name);
		let source;
		try {
			source = fs.readFileSync(file, "utf8");
		} catch {
			continue;
		}
		// 同时认「未打」与「已打」两种形态，否则打完补丁后 --check 会误报找不到锚点
		const modeRpc = (source.match(/mode:"rpc"/g) ?? []).length;
		const modeTui = (source.match(/mode:"tui"/g) ?? []).length;
		let legacy = 0;
		let legacyTui = 0;
		let cursor = 0;
		while ((cursor = source.indexOf("setUIContext?.(", cursor)) >= 0) {
			const window = source.slice(cursor, cursor + 300);
			if (/,\s*"rpc"\)/.test(window)) legacy++;
			else if (/,\s*"tui"\)/.test(window)) legacyTui++;
			cursor += 1;
		}
		if (modeRpc + modeTui + legacy + legacyTui === 0) continue;
		found.push({ file, name, modeRpc, modeTui, legacy, legacyTui });
	}
	return found;
}

function patchSource(source) {
	let next = source.replaceAll('mode:"rpc"', 'mode:"tui"');
	// 只改 setUIContext?.(..., "rpc") 里的那个字面量，不动文件里其它 "rpc"。
	// 注意游标必须单调前进：替换后窗口里已经看不到 "rpc"，从 0 重新搜会死循环。
	let searchFrom = 0;
	for (;;) {
		const cursor = next.indexOf("setUIContext?.(", searchFrom);
		if (cursor < 0) break;
		const window = next.slice(cursor, cursor + 300);
		const m = /,\s*"rpc"\)/.exec(window);
		if (!m) {
			searchFrom = cursor + 1;
			continue;
		}
		const at = cursor + m.index + m[0].indexOf('"rpc"');
		next = next.slice(0, at) + '"tui"' + next.slice(at + 5);
		searchFrom = at + 5;
	}
	return next;
}

function isPatched(file) {
	const source = fs.readFileSync(file, "utf8");
	if (/mode:"rpc"/.test(source)) return false;
	const legacyRemains = source
		.split("setUIContext?.(")
		.slice(1)
		.some((window) => /,\s*"rpc"\)/.test(window.slice(0, 300)));
	return !legacyRemains;
}

function jsSyntaxOk(file) {
	try {
		execFileSync(process.execPath, ["--check", file], { stdio: "pipe" });
		return true;
	} catch {
		return false;
	}
}

function main() {
	const targets = scanFiles();
	console.log(`[tui-mode] pi-web ${version} @ ${PIWEB_DIR}`);
	if (targets.length === 0) {
		console.log("[tui-mode] 没找到扩展绑定 mode 的锚点（可能已达标或 pi-web 结构变了），什么都不做。");
		process.exit(MODE === "check" ? 2 : 0);
	}
	const patchedCount = targets.filter((t) => isPatched(t.file)).length;
	console.log(
		`[tui-mode] 锚点文件 ${targets.length} 个：` +
			targets
				.map(
					(t) =>
						`${t.name}(mode rpc/tui=${t.modeRpc}/${t.modeTui}${t.legacy || t.legacyTui ? `, legacy rpc/tui=${t.legacy}/${t.legacyTui}` : ""})`,
				)
				.join(", "),
	);
	console.log(`[tui-mode] 当前状态：${patchedCount}/${targets.length} 已改成 "tui"`);

	if (MODE === "check") {
		process.exit(patchedCount === targets.length ? 0 : 2);
	}

	if (MODE === "revert") {
		if (patchedCount === 0) {
			console.log("[tui-mode] 没打过实验补丁，无需还原。");
			return;
		}
		for (const t of targets) {
			const backup = t.file + BACKUP_SUFFIX;
			if (fs.existsSync(backup)) {
				fs.copyFileSync(backup, t.file);
				fs.rmSync(backup);
			} else {
				// 没有备份就反向替换（只在本脚本打过的前提下安全）
				const source = fs.readFileSync(t.file, "utf8");
				let restored = source.replaceAll('mode:"tui"', 'mode:"rpc"');
				let searchFrom = 0;
				for (;;) {
					const cursor = restored.indexOf("setUIContext?.(", searchFrom);
					if (cursor < 0) break;
					const window = restored.slice(cursor, cursor + 300);
					const m = /,\s*"tui"\)/.exec(window);
					if (!m) {
						searchFrom = cursor + 1;
						continue;
					}
					const at = cursor + m.index + m[0].indexOf('"tui"');
					restored = restored.slice(0, at) + '"rpc"' + restored.slice(at + 5);
					searchFrom = at + 5;
				}
				fs.writeFileSync(t.file, restored);
			}
			console.log(`[tui-mode] 已还原 ${t.name}`);
		}
		return;
	}

	if (patchedCount === targets.length) {
		console.log("[tui-mode] 已经是实验模式（mode=tui），跳过。");
		return;
	}
	for (const t of targets) {
		if (!isPatched(t.file)) {
			const backup = t.file + BACKUP_SUFFIX;
			if (!fs.existsSync(backup)) fs.copyFileSync(t.file, backup);
			fs.writeFileSync(t.file, patchSource(fs.readFileSync(t.file, "utf8")));
		}
		if (!jsSyntaxOk(t.file)) {
			const backup = t.file + BACKUP_SUFFIX;
			if (fs.existsSync(backup)) fs.copyFileSync(backup, t.file);
			fail(`打完补丁后 ${t.name} 语法检查失败，已自动回滚，请人工核对。`);
		}
		console.log(`[tui-mode] 已改 ${t.name}（备份 ${path.basename(t.file)}${BACKUP_SUFFIX}）`);
	}
	console.log("\n[tui-mode] 实验模式已启用：所有扩展会认为宿主是 TUI。");
	console.log("[tui-mode] 重启 pi-web / Pi Web Desktop 生效；验证完请 --revert，App 升级也会覆盖。");
	console.log("[tui-mode] 验收清单：docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md §13。");
}

main();
