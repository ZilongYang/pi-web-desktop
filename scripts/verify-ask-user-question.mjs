#!/usr/bin/env node
/**
 * 离线验证 @juicesharp/rpiv-ask-user-question 在 **Pi Web Desktop 宿主契约** 下
 * 走的是完整 TUI 问卷，而不是退化的一题一个原生对话框。
 *
 * 为什么可以离线验证：Pi Web Desktop（@agegr/pi-web）的 extension UI 契约是
 * 确定的（见 .next/server/chunks/6429.js）：
 *   bindExtensions({ uiContext, mode: "rpc" })
 *   ui.custom(factory, options) -> Promise<result>
 *     component = factory(tui, theme, keybindings, done)
 *     width = options.overlayOptions?.width ?? 92
 *     输入：component.handleInput(data) 后重新 component.render(width)
 *     tui = { terminal: { columns: 92, rows: 40, ... }, requestRender() }
 *     theme = 纯文本 Theme 子类；按键 = ANSI（Enter=\r Tab=\t Shift+Tab=\x1b[Z Esc=\x1b）
 * 本脚本照这个契约复刻一个最小宿主，直接跑插件的 tool.execute()，因此可以在
 * 不启动 App 的情况下断言：标签栏、Shift+Tab 回退、改答案覆盖、Review 页、
 * 多选复选框、Esc 取消，以及「真 RPC 宿主仍走原生对话框」的回归。
 *
 * 依赖：本机已安装 pi（用它的 jiti + pi-tui/pi-coding-agent 做模块别名，和
 * pi 自己的扩展加载方式一致），以及已安装该插件（npm:@juicesharp/rpiv-ask-user-question）。
 *
 * 用法:
 *   node scripts/verify-ask-user-question.mjs            # 验证「补丁后」
 *   node scripts/verify-ask-user-question.mjs --before   # 用 npm 原版复现「补丁前」的 bug
 *   PI_AGENT_DIR=/path/to/agent node scripts/verify-ask-user-question.mjs
 *
 * 退出码：0 = 期望全部成立；1 = 有失败。
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const AGENT_DIR = process.env.PI_AGENT_DIR ?? path.join(os.homedir(), ".pi", "agent");
const NPM_DIR = path.join(AGENT_DIR, "npm");
const PLUGIN_DIR = path.join(NPM_DIR, "node_modules", "@juicesharp", "rpiv-ask-user-question");

function die(message) {
	console.error(`[verify-aq] ${message}`);
	process.exit(1);
}
if (!fs.existsSync(PLUGIN_DIR)) die(`找不到插件目录：${PLUGIN_DIR}（先在 pi 里安装 npm:@juicesharp/rpiv-ask-user-question）`);

const currentVersionFile = path.join(AGENT_DIR, "install", "current-version");
if (!fs.existsSync(currentVersionFile)) die(`找不到 pi 安装目录：${currentVersionFile}`);
const PI_VERSION = fs.readFileSync(currentVersionFile, "utf8").trim();
const PI_RELEASE = path.join(AGENT_DIR, "install", "releases", PI_VERSION);
const PI_NM = path.join(PI_RELEASE, "node_modules");
if (!fs.existsSync(path.join(PI_NM, "@earendil-works", "pi-coding-agent"))) {
	die(`pi ${PI_VERSION} 的 SDK 不在 ${PI_NM}，无法做模块别名。`);
}

const BEFORE = process.argv.includes("--before");
const ORIGINAL_SNAPSHOT = path.join(PLUGIN_DIR, "ask-user-question.ts.orig-2.15.0");
let workPluginDir = PLUGIN_DIR;
let cleanupBeforeDir;
if (BEFORE) {
	if (!fs.existsSync(ORIGINAL_SNAPSHOT)) {
		die(`--before 需要原版快照：${ORIGINAL_SNAPSHOT}\n（先执行过 patch 脚本后才会留下它；或用 --revert 还原后自行备份）`);
	}
	// 复制到 npm 目录内：借 ~/.pi/agent/npm/node_modules 解析 @juicesharp/rpiv-config 等裸导入。
	workPluginDir = fs.mkdtempSync(path.join(NPM_DIR, ".verify-aq-orig-"));
	fs.cpSync(PLUGIN_DIR, workPluginDir, {
		recursive: true,
		filter: (src) => !/\.(orig|bak)-/.test(path.basename(src)),
	});
	fs.copyFileSync(ORIGINAL_SNAPSHOT, path.join(workPluginDir, "ask-user-question.ts"));
	cleanupBeforeDir = () => fs.rmSync(workPluginDir, { recursive: true, force: true });
	process.on("exit", cleanupBeforeDir);
}

const pluginVersion = (() => {
	try {
		return JSON.parse(fs.readFileSync(path.join(PLUGIN_DIR, "package.json"), "utf8")).version ?? "unknown";
	} catch {
		return "unknown";
	}
})();
const pluginSource = fs.readFileSync(path.join(workPluginDir, "ask-user-question.ts"), "utf8");
const patched = pluginSource.includes("LOCAL PATCH (2026-11-08");

const piRequire = createRequire(path.join(PI_NM, "@earendil-works/pi-coding-agent/package.json"));
const { createJiti } = await import(pathToFileURL(piRequire.resolve("jiti")).href);

/** 与 pi 的 core/extensions/loader.js#getAliases() 同款：扩展里的 SDK 裸导入指向 pi 自带的那份。 */
function buildAlias() {
	const entry = (pkg, file) => path.join(PI_NM, pkg, file);
	const alias = {
		"@earendil-works/pi-coding-agent": entry("@earendil-works/pi-coding-agent", "dist/index.js"),
		"@earendil-works/pi-tui": entry("@earendil-works/pi-tui", "dist/index.js"),
		"@earendil-works/pi-agent-core": entry("@earendil-works/pi-agent-core", "dist/index.js"),
		"@earendil-works/pi-ai": entry("@earendil-works/pi-coding-agent", "dist/core/extensions/pi-ai-extension-view.js"),
		"@earendil-works/pi-ai/compat": entry("@earendil-works/pi-coding-agent", "dist/core/extensions/pi-ai-extension-view.js"),
		"@earendil-works/pi-ai/models": entry("@earendil-works/pi-ai", "dist/models.js"),
		"@earendil-works/pi-ai/oauth": entry("@earendil-works/pi-ai", "dist/oauth.js"),
		"@earendil-works/pi-ai/providers/all": entry("@earendil-works/pi-ai", "dist/providers/all.js"),
	};
	const tb = piRequire.resolve("typebox");
	alias["typebox"] = tb;
	alias["typebox/compile"] = piRequire.resolve("typebox/compile");
	alias["typebox/value"] = piRequire.resolve("typebox/value");
	alias["@sinclair/typebox"] = tb;
	alias["@sinclair/typebox/compile"] = alias["typebox/compile"];
	alias["@sinclair/typebox/value"] = alias["typebox/value"];
	return alias;
}

const jiti = createJiti(pathToFileURL(path.join(PI_NM, "@earendil-works/pi-coding-agent/dist/core/extensions/loader.js")).href, {
	moduleCache: false,
	alias: buildAlias(),
});
const mod = await jiti.import(path.join(workPluginDir, "ask-user-question.ts"));
const tui = await jiti.import("@earendil-works/pi-tui");
const sdk = await jiti.import("@earendil-works/pi-coding-agent");

// ---- pi-web 的 extension UI theme 复刻（chunk 6429 模块 47588）-------------
class PlainTheme extends sdk.Theme {
	constructor() {
		super({ muted: "", text: "", thinkingXhigh: "", searchMatchText: "" }, { selectedBg: "" }, "truecolor");
	}
	fg(...[, text]) { return text; }
	bg(...[, text]) { return text; }
	bold(text) { return text; }
	italic(text) { return text; }
	underline(text) { return text; }
	inverse(text) { return text; }
	strikethrough(text) { return text; }
	getFgAnsi() { return ""; }
	getBgAnsi() { return ""; }
	getThinkingBorderColor() { return (t) => t; }
	getBashModeBorderColor() { return (t) => t; }
}
const theme = new PlainTheme();
const keybindings = new tui.KeybindingsManager(tui.TUI_KEYBINDINGS);
const WIDTH = 92; // pi-web getCustomUiWidth() 默认值
const ROWS = 40; // pi-web TUI shim 默认值

const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));
// 未打补丁时 TUI 路径不会挂载：把等待缩短，快速给出结论而不是干等。
const MOUNT_TIMEOUT = patched ? 20000 : 3000;
async function until(pred, label, timeoutMs = MOUNT_TIMEOUT) {
	const start = Date.now();
	while (!pred()) {
		if (Date.now() - start > timeoutMs) throw new Error(`timeout waiting for ${label}`);
		await tick(20);
	}
}

let passes = 0;
let failures = 0;
function check(cond, label, extra) {
	if (cond) {
		passes++;
		console.log(`  ✓ ${label}`);
	} else {
		failures++;
		console.log(`  ✗ ${label}${extra !== undefined ? `\n      got: ${JSON.stringify(extra)}` : ""}`);
	}
}

/** 复刻 pi-web 的 createExtensionUiContext() + requestExtensionCustomUi()。 */
class PiWebHost {
	constructor({ withCustom = true, customImpl, dialogAnswers = [] } = {}) {
		this.withCustom = withCustom;
		this.customImpl = customImpl;
		this.dialogAnswers = [...dialogAnswers];
		this.selects = [];
		this.inputs = [];
		this.renders = [];
		this.terminalInputRegistrations = 0;
		this.component = undefined;
		this.options = undefined;
		this.ui = {
			notify: () => {},
			onTerminalInput: () => {
				this.terminalInputRegistrations++;
				return () => {};
			},
			select: async (title, options) => {
				this.selects.push({ title, options });
				return this.dialogAnswers.shift();
			},
			input: async (title, placeholder) => {
				this.inputs.push({ title, placeholder });
				return this.dialogAnswers.shift();
			},
		};
		if (withCustom) this.ui.custom = (factory, options) => this.mount(factory, options);
	}
	get ctx() {
		return { hasUI: true, mode: "rpc", cwd: process.cwd(), isProjectTrusted: () => true, ui: this.ui };
	}
	get width() {
		const opts = this.options?.overlayOptions;
		const w = typeof opts === "function" ? opts() : opts;
		const n = w?.width;
		return typeof n === "number" && Number.isFinite(n) ? Math.max(40, Math.min(140, Math.round(n))) : WIDTH;
	}
	get tuiShim() {
		return {
			terminal: Object.freeze({ columns: WIDTH, rows: ROWS, kittyProtocolActive: false }),
			requestRender: () => this.render(),
		};
	}
	mount(factory, options) {
		this.options = options;
		return new Promise((resolve) => {
			if (this.customImpl) return resolve(this.customImpl(factory, options));
			const comp = factory(this.tuiShim, theme, keybindings, (result) => resolve(result));
			Promise.resolve(comp).then((c) => {
				this.component = c;
				this.render();
			});
		});
	}
	render() {
		if (!this.component) return [];
		const lines = this.component.render(this.width);
		this.renders.push(lines);
		return lines;
	}
	get screen() {
		return (this.renders.at(-1) ?? []).join("\n").replace(/\u001b\[[0-9;]*m/g, "");
	}
	async feed(data) {
		if (!this.component) throw new Error("no mounted component to feed");
		this.component.handleInput(data);
		this.render(); // pi-web: handleExtensionUiInput() 之后重新 emitCustomUiRender()
		await tick();
	}
}

// ---- 假 ExtensionAPI + 场景数据 -----------------------------------------
const tools = new Map();
mod.registerAskUserQuestionTool({ registerTool: (def) => tools.set(def.name, def), events: { on() {}, off() {}, emit() {} } });
const tool = tools.get("ask_user_question");
if (!tool) die("ask_user_question 注册失败");

const KEYS = { enter: "\r", tab: "\t", shiftTab: "\u001b[Z", right: "\u001b[C", left: "\u001b[D", up: "\u001b[A", down: "\u001b[B", esc: "\u001b", space: " " };

const q1 = () => ({
	header: "排序",
	question: "第一题：用哪个数据库？",
	options: [
		{ label: "Postgres", description: "关系型" },
		{ label: "SQLite", description: "嵌入式" },
	],
});
const q2 = () => ({
	header: "缓存",
	question: "第二题：缓存放哪？",
	options: [
		{ label: "Redis", description: "外部服务" },
		{ label: "进程内", description: "本地" },
	],
});
const q3 = () => ({
	header: "部署",
	question: "第三题：怎么部署？",
	options: [
		{ label: "Docker", description: "容器" },
		{ label: "裸机", description: "直接跑" },
	],
});
const three = { questions: [q1(), q2(), q3()] };
const newHost = (opts = {}) => new PiWebHost(opts);

// ---- 场景 ---------------------------------------------------------------
async function scenarioTuiPath() {
	console.log("\n[1] Pi Web Desktop 宿主：3 题问卷 —— 标签栏 / 回退 / 改答案 / 提交");
	const host = newHost();
	const exec = tool.execute("call-1", three, undefined, undefined, host.ctx);
	await until(() => host.component, "问卷挂载");

	let screen = host.screen;
	check(screen.includes(" ← "), "渲染出标签栏（← … →）");
	check(screen.includes("□ 排序") && screen.includes("□ 缓存") && screen.includes("□ 部署"), "3 个未答标签（□）都在");
	check(screen.includes("✓ Submit"), "Submit 标签存在");
	check(screen.includes("第一题：用哪个数据库？") && screen.includes("Postgres"), "当前题与选项可见");
	check(screen.includes("Type something."), "自动追加的「Type something.」行存在");
	check(!screen.includes("Enter the numbers"), "没有落到 RPC 的「手打 1,3」多选文案");
	check(host.selects.length === 0, "没有调用宿主原生 select");
	check(host.renders.every((lines) => Array.isArray(lines)), "render(width) 返回字符串数组（宿主契约）");
	const widest = Math.max(...host.renders.at(-1).map((line) => tui.visibleWidth(line)));
	check(widest <= WIDTH, `渲染宽度不超过宿主给的 ${WIDTH} 列（实际 ${widest}）`);
	check(host.terminalInputRegistrations === 0, "内联挂载不注册 onTerminalInput（pi-web 里它是空实现，插件也只在 overlay 模式才注册）");

	await host.feed(KEYS.enter); // Q1 -> Postgres，自动进 Q2
	check(host.screen.includes("第二题：缓存放哪？"), "确认后自动进入第 2 题");
	check(host.screen.includes("■ 排序"), "第 1 题标签变已答（■）");

	await host.feed(KEYS.down);
	await host.feed(KEYS.enter); // Q2 -> 进程内
	check(host.screen.includes("第三题：怎么部署？"), "第 2 题确认后进入第 3 题");

	await host.feed(KEYS.enter); // Q3 -> Docker
	screen = host.screen;
	check(screen.includes("Review your answers"), "最后一题后出现 Review 页");
	check(screen.includes("→ Postgres") && screen.includes("→ 进程内") && screen.includes("→ Docker"), "Review 页列出 3 个答案");
	check(screen.includes("Submit answers"), "Review 页有 Submit 入口");

	// ★ 本 bug 的核心：能不能回退
	await host.feed(KEYS.shiftTab);
	check(host.screen.includes("第三题：怎么部署？"), "Review 页 Shift+Tab 回退到第 3 题");
	check(host.screen.includes("Docker ✔"), "回退后光标停在原答案（Docker ✔）");
	await host.feed(KEYS.shiftTab);
	await host.feed(KEYS.shiftTab);
	screen = host.screen;
	check(screen.includes("第一题：用哪个数据库？"), "Shift+Tab 连续回退到第 1 题");
	check(screen.includes("Postgres ✔"), "第 1 题光标停在原答案（Postgres ✔）");

	await host.feed(KEYS.down);
	await host.feed(KEYS.enter); // 改成 SQLite
	check(host.screen.includes("第二题：缓存放哪？"), "改选后前进到第 2 题");

	await host.feed(KEYS.tab); // -> Q3
	await host.feed(KEYS.tab); // -> Submit
	check(host.screen.includes("Review your answers"), "Tab 回到 Review 页");
	check(host.screen.includes("→ SQLite"), "Review 页显示改后的答案 SQLite");

	await host.feed(KEYS.enter); // Submit answers
	const d = (await exec).details ?? {};
	const answers = d.answers ?? [];
	check(d.cancelled === false, "提交后 cancelled=false", d.cancelled);
	check(answers.length === 3, "答案数 3（回改是覆盖不是追加）", answers.map((a) => a.answer));
	check(answers[0]?.answer === "SQLite" && answers[1]?.answer === "进程内" && answers[2]?.answer === "Docker", "最终答案 = SQLite / 进程内 / Docker", answers.map((a) => a.answer));
	check(answers.every((a) => a.kind === "option"), "答案 kind 都是 option", answers.map((a) => a.kind));
	check((host.renders.at(-1) ?? []).every((line) => tui.visibleWidth(line) <= WIDTH), `提交前所有渲染行 ≤ ${WIDTH} 列`);
}

async function scenarioEscape() {
	console.log("\n[2] Esc = 取消整份问卷");
	const host = newHost();
	const exec = tool.execute("call-2", three, undefined, undefined, host.ctx);
	await until(() => host.component, "问卷挂载");
	await host.feed(KEYS.enter); // 先答第 1 题
	await host.feed(KEYS.esc);
	const result = await exec;
	const d = result.details ?? {};
	check(d.cancelled === true, "Esc 后 cancelled=true", d.cancelled);
	check(/cancel|declin|取消/i.test(result.content?.[0]?.text ?? ""), "结果文本是取消语义");
}

async function scenarioSingleQuestion() {
	console.log("\n[3] 单题问卷不回归");
	const host = newHost();
	const exec = tool.execute("call-3", { questions: [q1()] }, undefined, undefined, host.ctx);
	await until(() => host.component, "问卷挂载");
	check(host.screen.includes("第一题：用哪个数据库？"), "单题正文可见");
	check(!host.screen.includes(" ← "), "单题不渲染多题标签栏");
	await host.feed(KEYS.enter);
	const d = (await exec).details ?? {};
	check(d.cancelled === false && d.answers?.length === 1, "单题直接提交", d.cancelled);
	check(d.answers?.[0]?.answer === "Postgres", "单题答案 = Postgres", d.answers?.[0]);
}

async function scenarioNativeRpcHost() {
	console.log("\n[4] 真 RPC 宿主（无 custom，只有 select/input）—— 回归：仍走原生对话框");
	const host = newHost({ withCustom: false, dialogAnswers: ["1. Postgres — 关系型", "2. 进程内 — 本地"] });
	const d = (await tool.execute("call-4", { questions: [q1(), q2()] }, undefined, undefined, host.ctx)).details ?? {};
	check(host.selects.length === 2, "原生 select 被调用 2 次", host.selects.length);
	check(d.cancelled === false && d.answers?.length === 2, "RPC 路径拿到 2 个答案", d.answers?.map((a) => a.answer));
	check(d.answers?.[0]?.answer === "Postgres", "RPC 第 1 题 = Postgres", d.answers?.[0]?.answer);
	check(host.renders.length === 0, "RPC 路径没有挂 custom UI");
}

async function scenarioUnrenderableCustom() {
	console.log("\n[5] 宿主有 custom() 但返回 undefined —— 兜底回 RPC，不报 no_custom_ui");
	const host = newHost({ customImpl: () => Promise.resolve(undefined), dialogAnswers: ["1. Postgres — 关系型"] });
	const result = await tool.execute("call-5", { questions: [q1()] }, undefined, undefined, host.ctx);
	const d = result.details ?? {};
	check(host.selects.length === 1, "custom 失效后回退到原生 select", host.selects.length);
	check(d.cancelled === false && d.answers?.[0]?.answer === "Postgres", "兜底路径照样拿到答案", d.answers?.[0]);
	check(!/no_custom_ui/.test(result.content?.[0]?.text ?? ""), "没有报 no_custom_ui");
}

async function scenarioMultiSelect() {
	console.log("\n[6] 单题多选：复选框 + Space 勾选 + Submit 行");
	const host = newHost();
	const params = {
		questions: [
			{
				header: "功能",
				question: "要启用哪些功能？",
				multiSelect: true,
				options: [
					{ label: "搜索", description: "全文搜索" },
					{ label: "导出", description: "导出 CSV" },
				],
			},
		],
	};
	const exec = tool.execute("call-6", params, undefined, undefined, host.ctx);
	await until(() => host.component, "问卷挂载");
	check(host.screen.includes("要启用哪些功能？"), "多选正文可见");
	await host.feed(KEYS.space);
	await host.feed(KEYS.down);
	await host.feed(KEYS.space);
	check(host.screen.includes("[✔] 搜索") && host.screen.includes("[✔] 导出"), "两行出现勾选标记 [✔]");
	check(host.screen.includes("Submit"), "列表底部有 Submit 行");
	await host.feed(KEYS.down);
	await host.feed(KEYS.down);
	await host.feed(KEYS.enter);
	const d = (await exec).details ?? {};
	check(d.cancelled === false, "多选提交成功", d.cancelled);
	check((d.answers?.[0]?.selected ?? []).join(",") === "搜索,导出", "选中搜索+导出", d.answers?.[0]?.selected);
}

async function scenarioPreview() {
	console.log("\n[7] 带 preview 的选项：92 列下不崩，preview 折叠/上下堆叠");
	const host = newHost();
	const params = {
		questions: [
			{
				header: "布局",
				question: "选哪个布局？",
				options: [
					{ label: "布局 A", description: "两栏", preview: "布局 A 的 ASCII 预览内容" },
					{ label: "布局 B", description: "单栏", preview: "布局 B 的 ASCII 预览内容" },
				],
			},
		],
	};
	const exec = tool.execute("call-7", params, undefined, undefined, host.ctx);
	await until(() => host.component, "问卷挂载");
	const lines = host.renders.at(-1) ?? [];
	check(lines.every((line) => tui.visibleWidth(line) <= WIDTH), `preview 渲染行都 ≤ ${WIDTH} 列`);
	const screen = host.screen;
	check(screen.includes("选哪个布局？"), "题目正文可见");
	const previewAt = screen.indexOf("布局 A 的 ASCII 预览内容");
	const listEndAt = screen.indexOf("3. Type something.");
	check(previewAt >= 0, "preview 内容渲染出来了（没被折叠丢弃）");
	check(previewAt >= 0 && listEndAt >= 0 && previewAt > listEndAt, "92 列下 preview 上下堆叠（在选项列表下方），不是并排");
	await host.feed(KEYS.enter);
	const d = (await exec).details ?? {};
	check(d.cancelled === false && d.answers?.[0]?.answer === "布局 A", "preview 场景仍可正常选择", d.answers?.[0]);
}

console.log(`[verify-aq] ${BEFORE ? "补丁前（npm 原版）复现" : "补丁后验证"}：插件 ${pluginVersion}（${patched ? "已打 TUI 路径补丁" : "未打补丁"}），pi ${PI_VERSION}，宿主 = Pi Web Desktop 契约模拟 ${WIDTH}x${ROWS}`);

/** --before：原版应表现为「一题一个原生对话框」，ui.custom() 从未挂载。 */
async function scenarioBefore() {
	console.log("\n[before] npm 原版：3 题问卷退化为原生对话框走查（复现 bug）");
	const host = newHost({ dialogAnswers: ["1. Postgres — 关系型", "2. 进程内 — 本地", "1. Docker — 容器"] });
	const d = (await tool.execute("before-1", three, undefined, undefined, host.ctx)).details ?? {};
	check(host.component === undefined, "ui.custom() 没有被调用（没有自定义面板）", host.component);
	check(host.selects.length === 3, "3 题 = 3 个独立原生 select（一问一屏）", host.selects.length);
	check(host.renders.length === 0, "没有产生任何自定义渲染行");
	check(d.cancelled === false && d.answers?.length === 3, "原版同样能拿到 3 个答案", d.answers?.map((a) => a.answer));

	console.log("\n[before] 中途关掉一个对话框 = 整份问卷作废（DECLINE）");
	const host2 = newHost({ dialogAnswers: ["1. Postgres — 关系型", undefined] });
	const result = await tool.execute("before-2", { questions: [q1(), q2()] }, undefined, undefined, host2.ctx);
	const d2 = result.details ?? {};
	check(d2.cancelled === true, "cancelled=true", d2.cancelled);
	check(d2.answers?.length === 1, "只有第 1 题落进 details", d2.answers?.map((a) => a.answer));
	check(/cancel|declin|取消/i.test(result.content?.[0]?.text ?? ""), "模型侧只收到 DECLINE 语义");
}

const scenarios = BEFORE
	? [["[before]", scenarioBefore]]
	: [
			["[1]", scenarioTuiPath],
			["[2]", scenarioEscape],
			["[3]", scenarioSingleQuestion],
			["[4]", scenarioNativeRpcHost],
			["[5]", scenarioUnrenderableCustom],
			["[6]", scenarioMultiSelect],
			["[7]", scenarioPreview],
		];

for (const [label, fn] of scenarios) {
	try {
		await fn();
	} catch (e) {
		failures++;
		console.log(`  ✗ ${label} 抛错: ${e?.stack ?? e}`);
	}
}

console.log(`\n===== ${passes} passed, ${failures} failed =====`);
process.exit(failures === 0 ? 0 : 1);
