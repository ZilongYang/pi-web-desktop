#!/usr/bin/env node
/**
 * 给 pi 扩展 @juicesharp/rpiv-ask-user-question 打「让 Pi Web Desktop 走完整 TUI
 * 问卷」补丁（对应 docs/BUG-2026-11-08-ask-user-question-rpc-oneway.md 方案 A）。
 *
 * 为什么需要：插件把 `ctx.mode === "rpc"` 当成「ui.custom() 不可用」的充分条件。
 * 但 Pi Web Desktop（@agegr/pi-web）虽然声明 mode:"rpc"（它的 select/input 是
 * 原生对话框），custom() 却是真实现 —— 它调用组件的 render(width) 把文本行推给
 * 前端，并把前端按键（Tab / Shift+Tab / 方向键 / Esc）转成 ANSI 回灌
 * handleInput。于是问卷在本机退化成一题一个原生对话框：没有标签栏、不能回退
 * 改答案，中途 Esc 还会让整份问卷作废。
 *
 * 补丁只改一道闸门：`mode === "rpc"` 且宿主**确实没有** custom() 时才走 RPC 走查；
 * 其余情况走完整 TUI 路径。宿主 custom() 解析成 undefined 时插件原有的
 * resolveUndefinedResult() 兜底仍会退回 RPC 走查，所以这条改动没有「变坏」分支。
 *
 * 用法:
 *   node scripts/patch-rpiv-ask-user-question.mjs            # 打补丁（幂等）
 *   node scripts/patch-rpiv-ask-user-question.mjs --check    # 只检查：已打=0，未打=2
 *   node scripts/patch-rpiv-ask-user-question.mjs --revert   # 还原成 npm 原版
 *   PI_AGENT_DIR=/some/agent node scripts/...                # 自定义 agent 目录
 *
 * 注意：插件装在 ~/.pi/agent/npm/node_modules 下，`pi update` / 重装扩展会覆盖
 * 补丁 —— 升级后重跑本脚本即可。补丁锚点钉在 2.15.0；版本变了会先警告，锚点
 * 找不到时直接报错退出，不会乱改文件。
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const TARGET_VERSION = "2.15.0";
const AGENT_DIR = process.env.PI_AGENT_DIR ?? path.join(os.homedir(), ".pi", "agent");
const PLUGIN_DIR = path.join(AGENT_DIR, "npm", "node_modules", "@juicesharp", "rpiv-ask-user-question");
const TARGET = path.join(PLUGIN_DIR, "ask-user-question.ts");
const MARKER = "LOCAL PATCH (2026-11-08, BUG-2026-11-08-ask-user-question-rpc-oneway)";

/** npm 2.15.0 原文（闸门那一段）。 */
const ORIGINAL_BLOCK = `			// RPC hosts (VSCode pendant, ACP clients like Zed/Paseo — issue #78):
			// ui.custom() cannot render there, but the select/input dialog
			// sub-protocol works. Hosts that advertise ctx.mode (pi ≥0.79) route to
			// the sequential dialog walker up front, skipping the TUI render-graph
			// import entirely; RPC builds that predate ctx.mode are caught by the
			// custom()-resolved-undefined backstop below. See ./rpc-fallback.ts.
			const viaRpc = (ctx as { mode?: string }).mode === "rpc" && hasDialogUI(ctx.ui);`;

/** 打完补丁后的样子。 */
const PATCHED_BLOCK = `			// RPC hosts (VSCode pendant, ACP clients like Zed/Paseo — issue #78):
			// ui.custom() cannot render there, but the select/input dialog
			// sub-protocol works. Hosts that advertise ctx.mode (pi ≥0.79) route to
			// the sequential dialog walker up front, skipping the TUI render-graph
			// import entirely; RPC builds that predate ctx.mode are caught by the
			// custom()-resolved-undefined backstop below. See ./rpc-fallback.ts.
			//
			// LOCAL PATCH (2026-11-08, BUG-2026-11-08-ask-user-question-rpc-oneway):
			// \`mode === "rpc"\` alone is not proof that ui.custom() is unusable. Pi Web
			// Desktop (@agegr/pi-web) reports rpc mode because its select/input dialogs
			// are native, yet it implements custom() for real: it calls the component's
			// render(width) and forwards keys as ANSI. Treating mode as capability
			// degraded every multi-question questionnaire to the one-way dialog walker
			// (no tabs, no going back). Require the host to lack custom() as well; a
			// host whose custom() resolves undefined still lands in
			// resolveUndefinedResult(), which runs the dialog walker unchanged.
			const viaRpc =
				(ctx as { mode?: string }).mode === "rpc" &&
				hasDialogUI(ctx.ui) &&
				typeof (ctx.ui as { custom?: unknown }).custom !== "function";`;

function fail(message) {
	console.error(`[patch-rpiv-aq] ${message}`);
	process.exit(1);
}

function readState() {
	if (!fs.existsSync(TARGET)) fail(`找不到插件源码: ${TARGET}\n（先在 pi 里安装 npm:@juicesharp/rpiv-ask-user-question）`);
	const versionFile = path.join(PLUGIN_DIR, "package.json");
	let version = "unknown";
	try {
		version = JSON.parse(fs.readFileSync(versionFile, "utf8")).version ?? "unknown";
	} catch {
		// 版本读不到不影响补丁本身，下面按锚点判断。
	}
	const source = fs.readFileSync(TARGET, "utf8");
	const patched = source.includes(MARKER) && source.includes("typeof (ctx.ui as { custom?: unknown }).custom !== \"function\"");
	const original = source.includes(ORIGINAL_BLOCK);
	if (!patched && !original) {
		fail(
			`补丁锚点在 ask-user-question.ts 里找不到（插件版本 ${version}，脚本针对 ${TARGET_VERSION}）。\n` +
				`上游很可能改了这段代码，请人工核对后再更新本脚本，不要盲打。`,
		);
	}
	return { source, version, patched, original };
}

function writeSource(next) {
	const tmp = `${TARGET}.patch-tmp`;
	fs.writeFileSync(tmp, next);
	fs.renameSync(tmp, TARGET);
}

function main() {
	const args = process.argv.slice(2);
	const mode = args.includes("--revert") ? "revert" : args.includes("--check") ? "check" : "patch";
	const { source, version, patched } = readState();

	if (version !== TARGET_VERSION) {
		console.warn(`[patch-rpiv-aq] 注意：插件版本是 ${version}，本脚本按 ${TARGET_VERSION} 的锚点编写。`);
	}

	if (mode === "check") {
		console.log(patched ? `[patch-rpiv-aq] 已打补丁（插件 ${version}）` : `[patch-rpiv-aq] 未打补丁（插件 ${version}）`);
		process.exit(patched ? 0 : 2);
	}

	if (mode === "revert") {
		if (!patched) {
			console.log("[patch-rpiv-aq] 本来就没有补丁，无需还原。");
			return;
		}
		writeSource(source.replace(PATCHED_BLOCK, ORIGINAL_BLOCK));
		console.log("[patch-rpiv-aq] 已还原成 npm 原版（重启 Pi 后生效）。");
		return;
	}

	if (patched) {
		console.log("[patch-rpiv-aq] 已打过补丁，跳过。");
		return;
	}
	writeSource(source.replace(ORIGINAL_BLOCK, PATCHED_BLOCK));
	console.log("[patch-rpiv-aq] 补丁已写入（重启 Pi / Pi Web Desktop 后生效）。");
}

main();
