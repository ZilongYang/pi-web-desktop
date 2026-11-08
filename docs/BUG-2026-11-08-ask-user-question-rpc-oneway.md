# BUG 记录 —— Pi Web Desktop 下 `ask_user_question` 问卷退化为单向走查（无法回退修改答案）

- **记录日期**：2026-11-08
- **发现场景**：在 Pi Web Desktop 里让 agent 连续问 3 个问题（`ask_user_question`），答完第 1 题、进入第 2 题后发现第 1 题选错，**`←` / `Shift+Tab` / `Tab` 全部无反应**，无法回退
- **状态**：**方案 A（就地补丁）已实施 → 离线宿主模拟验证 51/51 通过；待子龙在 Pi Web Desktop 真机验收**
- **本轮改动**（2026-11-08）：插件 `ask-user-question.ts` 改 1 处闸门（原版/补丁后均有备份，可回滚）；仓库新增 `scripts/patch-rpiv-ask-user-question.mjs`（幂等打补丁 / 检查 / 还原）与 `scripts/verify-ask-user-question.mjs`（离线宿主模拟验证）；实施与验证记录见 §11
- **严重度**：中（功能可用但交互降级；误操作会导致整份问卷作废）

---

## 1. 摘要（TL;DR）

`@juicesharp/rpiv-ask-user-question` 有两条渲染路径：

| 路径 | 条件 | 表现 |
|---|---|---|
| **TUI（完整）** | `ctx.ui.custom()` 可渲染 | 标签栏、`←`/`Shift+Tab` 翻题、提交前 Review 页、复选框多选、`n` 备注、数字键快选、preview 并排 |
| **RPC（降级）** ← **当前实际走这条** | `ctx.mode === "rpc"` 且宿主有 `select`/`input` | **一题一个原生对话框，严格顺序，无标签栏，不可回退** |

插件把 `mode === "rpc"` 一视同仁地当成"渲染不了自定义 UI 的宿主"（VSCode pendant / Zed / Paseo 确实是），**但 `@agegr/pi-web` 是例外** —— 它的 `ui.custom()` 是真实现，并且键盘（含 `Shift+Tab`、方向键）是完整转发的。

### 根因在哪一层（2026-11-08 复盘修订）

本文档最初写成"插件在本不该降级的地方主动降级了"。核对上游契约后要修正这个说法：

- **插件是照上游文档写的，不是写错**。pi 官方扩展文档 `docs/extensions.md` → *Mode Behavior* 明确：
  RPC 模式下 "`custom()` returns `undefined`, component factories are ignored"，并要求
  "Use `ctx.mode === "tui"` before TUI-specific features (`custom()`, component factories, terminal input)"；
  `docs/rpc-extension-ui.md` → *Limitations* 第一条也是 "`custom()` returns `undefined`"。
  所以按 `mode === "rpc"` 放弃自定义 UI 是**守约**（VSCode pendant / Zed / Paseo 正是这样）。
- **pi-core 只有四档 `mode`**（`tui | rpc | json | print`），**没有"rpc 传输 + 支持文本自定义面板"这一档**。
- **pi-web 是契约外的"超集宿主"**：声明 `mode:"rpc"`，却真实现了 `custom()`（渲染 `render(width)` 并回灌 ANSI 按键）。
  它属于"第三类宿主"，但没有名字可用 —— **根因是宿主能力声明与 pi-core 契约的错配**，不是某个插件写错。

因此 §7.1 / §11 的本机补丁改的是"错误推断的落点"：正确、必要、可立即生效，但**它不是"根除"** ——
pi-core 不提供能力协商的话，每一个按契约写法的插件都会在 pi-web 里降级。生态影响面见 §12，
"为什么不能靠改宿主一个字符串根除 + 可回滚实验"见 §13。

---

## 2. 现象（用户视角）

在 Pi Web Desktop 里：

- 问卷以**一题一个对话框**的形态依次出现（不是带标签栏的一整块面板）。
- 答完当前题按回车 → 直接进入下一题，**看不到任何"上一题/下一题"的入口**。
- `←` `→` `Tab` `Shift+Tab` 均无任何反应。
- 多选题不是复选框，而是**一个文本框，要求手打 `1,3`**（可加 `; 自定义答案`）。
- 每题的最后一行为自定义答案输入（"Type something." 语义），选中会再弹一个输入框。
- 侧边 preview 并排布局不出现（被折叠进对话框标题，截断 600 字符）。

> 对照插件上游文档 `docs/keyboard.md`：那里写的是"`Tab`/`Shift+Tab` 切换标签页，`→`/`←` 同上"—— 该文档描述的是 **TUI 路径**，与本机实际行为不符，是排查时最容易踩的坑（本轮就踩了）。

---

## 3. 环境（复现基线）

| 项 | 值 |
|---|---|
| 宿主 | Pi Web Desktop（`__CFBundleIdentifier=com.zilong.piweb.desktop`） |
| pi | 1.5.0（`~/.pi/agent/bin/pi`） |
| pi-web | `@agegr/pi-web` 0.15.0 |
| 插件 | `@juicesharp/rpiv-ask-user-question` **2.15.0**（= npm `latest`，2026-11-08 查询，上游无修复版本） |
| 插件形态 | 纯 TS 源码，由 pi 直接加载（`package.json` 的 `"./index.ts"`），无编译产物 |
| 相关同类产物 | `@narumitw/pi-tui-kit` 也装在本机，但**与本问题无关**（其 questionnaire 组件未被加载）——排查时曾误判，见 §9 |

关键路径：

```
~/.pi/agent/settings.json                                     # packages 列表
~/.pi/agent/npm/node_modules/@juicesharp/rpiv-ask-user-question/   # 插件（TS 源码）
/Applications/Pi Web Desktop.app/Contents/Resources/pi-web/app/node_modules/@agegr/pi-web/
  .next/server/chunks/6429.js                                # 宿主：扩展绑定 + 自定义 UI 机制
  .next/static/chunks/app/page-b18f4d3b4d3efd1b.js           # 宿主：前端键盘 → ANSI 映射
```

---

## 4. 机制（代码地图 + 证据）

### 4.1 分叉点：插件里唯一的那道闸门

`@juicesharp/rpiv-ask-user-question@2.15.0` → `ask-user-question.ts`：

```ts
// RPC hosts (VSCode pendant, ACP clients like Zed/Paseo — issue #78):
// ui.custom() cannot render there, but the select/input dialog
// sub-protocol works. Hosts that advertise ctx.mode (pi ≥0.79) route to
// the sequential dialog walker up front, skipping the TUI render-graph
// import entirely; RPC builds that predate ctx.mode are caught by the
// custom()-resolved-undefined backstop below. See ./rpc-fallback.ts.
const viaRpc = (ctx as { mode?: string }).mode === "rpc" && hasDialogUI(ctx.ui);

const open = answers.open(toolCallId, typed.questions, viaRpc ? "rpc" : "tui");
if (viaRpc) return await runRpcPath(pi, ctx.ui, typed, toolCallId);
return await runTuiPath(pi, ctx, typed, open);
```

`hasDialogUI`（`rpc-fallback.ts`）：

```ts
export function hasDialogUI(ui: unknown): ui is DialogUI {
	const u = ui as Partial<Record<"select" | "input", unknown>> | null | undefined;
	return typeof u?.select === "function" && typeof u?.input === "function";
}
```

**结论**：只要宿主 `mode === "rpc"` 且提供 `select`/`input`，一律走 RPC，**不再尝试** `ui.custom()`。

### 4.2 RPC 路径为什么不可回退

`rpc-fallback.ts` → `runRpcQuestionnaire()`：

```ts
export async function runRpcQuestionnaire(ui: DialogUI, params: QuestionParams) {
	const answers: QuestionAnswer[] = [];
	for (let qi = 0; qi < params.questions.length; qi++) {
		const q = params.questions[qi];
		const header = q.header ? `[${q.header}] ` : "";
		const answer = q.multiSelect ? await askMultiSelect(ui, q, qi, header)
		                             : await askSingleSelect(ui, q, qi, header);
		if (answer === undefined) return { answers, cancelled: true };
		answers.push(answer);          // ← 确认即入库，循环只增不减
	}
	return { answers, cancelled: false };
}
```

这是一个**单向 `for` 循环**：确认即 `push`，没有任何回退分支。RPC 原语 `select(title, options)` / `input(title, placeholder)` 本身也没有"回到上一屏"的概念。

### 4.3 致命细节：Esc / 取消 = 整份问卷作废

同文件：

```ts
 * Walk the questionnaire one native dialog at a time. Dismissing a question's
 * dialog (the primitive resolves `undefined`) cancels the whole questionnaire —
 * mirroring Esc in the TUI — and the shared envelope emits DECLINE.
```

`tool/response-envelope.ts`：

```ts
export function buildQuestionnaireResponse(result, params) {
	if (!result || result.cancelled) {
		return buildToolResult(DECLINE_MESSAGE, {
			answers: result?.answers ?? [],      // ← 已答题只落在 details 里
			cancelled: true,
			...
		});
	}
	...
```

**后果**：用户在 RPC 路径下想"退一题"而按 Esc，得到的是——模型侧只看到一句 **DECLINE**，已答内容仅存在于 `details` 字段（对话层面等于全部作废）。这是本 bug 最坑的一点。

### 4.4 宿主侧：pi-web 确实以 `mode:"rpc"` 绑定扩展

`@agegr/pi-web@0.15.0` → `.next/server/chunks/6429.js`：

```js
bindExtensions({ uiContext: a, mode: "rpc", commandContextActions: ..., ... })
```

（同时存在一条旧版兼容分支：`"function"!=typeof this.inner.bindExtensions && this.inner.extensionRunner.setUIContext?.(this.createExtensionUiContext(), "rpc")`）

### 4.5 宿主侧：pi-web 的 `select` / `input` 是真实现

`6429.js` → `createExtensionUiContext()` 节选：

```js
select: (a,b,c)=>this.requestExtensionUi({method:"select",title:a,options:b,...}, void 0, a=>"value"in a?a.value:void 0, c?.timeout, c?.signal),
input:  (a,b,c)=>this.requestExtensionUi({method:"input", title:a, ...void 0!==b?{placeholder:b}:{},...}, void 0, a=>"value"in a?a.value:void 0, c?.timeout, c?.signal),
custom: (a,b)=>this.requestExtensionCustomUi(a,b),      // ← 关键：custom 也是真实现
```

⇒ 插件闸门的两个条件**都**满足 ⇒ `viaRpc = true`。

### 4.6 决定性证据：pi-web 的 `ui.custom()` 不是空壳

`6429.js` → `requestExtensionCustomUi()` 的核心：

```js
requestExtensionCustomUi(a,b){
  ...
  Promise.resolve().then(()=> g?void 0 : a(h, theme, aA, j)).then(a=>{
    if(!a || "object"!=typeof a || "function"!=typeof a.render) return void i(void 0);
    let b={component:a,width:e,resolve:a=>i(a),settled:!1};
    this.activeCustomUis.set(d,b), this.emitCustomUiRender(d,b);
  })
}
emitCustomUiRender(a,b){
  let c; try{ c=b.component.render(b.width) }catch(a){ c=[`Extension custom UI render failed: ...`] }
  let d={type:"extension_ui_request",id:a,method:"custom",lines:c};
  this.pendingUiRequests.set(a,d), this.emit(d)
}
handleExtensionUiInput(a,b){
  let c=this.activeCustomUis.get(a);
  if(c && "string"==typeof b) try{ c.component.handleInput?.(b), ...; this.emitCustomUiRender(a,c) }catch(b){ ... }
}
```

即：**接受 pi-tui 风格的组件（有 `render(width)` / `handleInput(data)`），渲染成文本行推给前端，并把前端按键回灌 `handleInput`。**

### 4.7 决定性证据：pi-web 传给组件的 TUI shim 是完整的

`6429.js`，webpack module id `71069`：

```js
c.d(b,{TQ:()=>e, p1:()=>d});
let d = 92;                                     // p1 = 默认宽度
function e(a, b = d, c = 40) {                  // TQ(requestRender, columns=92, rows=40)
  return Object.freeze({
    terminal: Object.freeze({ columns: b, rows: c, kittyProtocolActive: false }),
    requestRender: a
  });
}
```

插件 TUI 路径要用的 `tui.terminal.columns` / `tui.terminal.rows` / `tui.requestRender()` **全部存在**。
（行数预算 `presentation.ts`: `Math.min(rows, Math.max(12, floor(rows*0.7)))` → 40 行时得 28 行，可正常工作。）

### 4.8 决定性证据：前端完整转发 Tab / Shift+Tab / 方向键

`page-b18f4d3b4d3efd1b.js`：

```js
let rA={ArrowUp:"\x1b[A",ArrowDown:"\x1b[B",ArrowRight:"\x1b[C",ArrowLeft:"\x1b[D",
        Home:"\x1b[H",End:"\x1b[F",Insert:"\x1b[2~",Delete:"\x1b[3~",
        PageUp:"\x1b[5~",PageDown:"\x1b[6~",Escape:"\x1b",Backspace:""};
```

```js
return "Enter"===e.key ? (e.shiftKey?"\n":"\r")
     : "Tab"===e.key   ? (e.shiftKey?"\x1b[Z":"\t")     // ← Shift+Tab = CSI Z，Tab = \t
     : rA[e.key] ?? null;
```

并附带：Ctrl+字母 → 控制字符、Alt+key → `ESC` 前缀、粘贴 → 括号粘贴 `\x1b[200~…\x1b[201~`。
前端通过

```js
await aZ(n,{type:"extension_ui_input",id:e.id,data:t})
```

发给服务端 → `handleExtensionUiInput` → `component.handleInput(data)`。

### 4.9 兜底：即使切到 TUI 后渲染失败，也会自动退回现状

`ask-user-question.ts`：

```ts
/**
 * A TUI questionnaire ALWAYS resolves a QuestionnaireResult (cancel included), so
 * `undefined` uniquely means "host cannot render", never "user declined". RPC builds
 * that predate ctx.mode land here: run the dialog walker when the host has the
 * primitives; ...
 */
async function resolveUndefinedResult(ctx: ExtensionContext, typed: QuestionParams) {
	if (hasDialogUI(ctx.ui)) {
		return buildQuestionnaireResponse(await runRpcQuestionnaire(ctx.ui, typed), typed);
	}
	return buildToolResult(ERROR_NO_CUSTOM_UI, { answers: [], cancelled: true, error: "no_custom_ui" });
}
```

⇒ **改一行切到 TUI 路径属于"要么变好、要么维持现状"，没有变坏的分支。**

---

## 5. 能力对照表

| 能力 | 终端 `pi`（TUI 路径） | **Pi Web Desktop（RPC，当前）** | 切到 TUI 后（预期） |
|---|---|---|---|
| 标签栏 / 题目标签 | ✅ | ❌ 不存在 | ✅ |
| **退回上一题改答案** | ✅ `←` / `Shift+Tab` | ❌ 不可能 | ✅ |
| 提交前 Review 页 | ✅ | ❌ | ✅ |
| 回退时光标恢复到原选项 | ✅ | — | ✅ |
| 多选 | 复选框 + `Space` | 文本框手打 `1,3` | ✅ 复选框 |
| 数字键 `1–9` 快选 | ✅ | ❌ | ✅ |
| `n` 加备注 | ✅ | ❌ | ✅ |
| preview 并排预览 | ✅（≥100 列） | ❌ 折叠进标题 | ⚠️ 宽度只有 92 → 上下堆叠 |
| `Ctrl+]` 折叠 | ✅ | ❌（`onTerminalInput` 空实现） | ❌ 仍无效 |
| 鼠标点选项 | ✅ | 宿主原生对话框 | ❌ 只能键盘 |
| Esc 语义 | 取消整份 | 取消整份（等同 TUI） | 取消整份 |

---

## 6. 影响面

1. **交互降级**：多题问卷无法回退，误选只能"事后在聊天里口头纠正"。
2. **数据风险**：RPC 路径下按 Esc 会把已答内容从对话可见层抹掉（仅存 `details`），模型只读到 DECLINE。
3. **agent 侧连带**：因为无法回退，模型侧更容易收到"部分答案 + 取消"的歧义结果。
4. **不影响**：单题问卷（本就无标签栏）、终端 TUI 用户、其他宿主（Zed/VSCode pendant 的 `custom()` 确实不可用，保持 RPC 才是对的）。

---

## 7. 修复方案

### 7.1 方案 A：就地打补丁（✅ 已于 2026-11-08 实施，见 §11）

> 实施时改成了可重复执行的脚本 [`../scripts/patch-rpiv-ask-user-question.mjs`](../scripts/patch-rpiv-ask-user-question.mjs)：
> 直接跑 = 打补丁（幂等），`--check` = 只检查，`--revert` = 还原。
> 插件被 `pi update` / 重装扩展覆盖后，重跑一次即可。下面是当时的原始方案，保留以备查。

文件：`~/.pi/agent/npm/node_modules/@juicesharp/rpiv-ask-user-question/ask-user-question.ts`

```diff
-			const viaRpc = (ctx as { mode?: string }).mode === "rpc" && hasDialogUI(ctx.ui);
+			// Pi Web Desktop (@agegr/pi-web) advertises mode:"rpc" but DOES implement
+			// ui.custom() (component.render/handleInput) and forwards Tab/Shift+Tab/arrows
+			// as ANSI. Prefer the custom UI whenever the host really provides one; the
+			// resolveUndefinedResult() backstop still falls back to the RPC walker.
+			const viaRpc =
+				(ctx as { mode?: string }).mode === "rpc" &&
+				hasDialogUI(ctx.ui) &&
+				typeof (ctx.ui as { custom?: unknown }).custom !== "function";
```

步骤：

```bash
P=~/.pi/agent/npm/node_modules/@juicesharp/rpiv-ask-user-question
cp "$P/ask-user-question.ts" "$P/ask-user-question.ts.bak-$(date +%Y%m%d)"
# 改上面那一处
# 然后重启 Pi Web Desktop（或重载扩展），发一次多题问卷验证
```

回滚：

```bash
mv "$P/ask-user-question.ts.bak-YYYYMMDD" "$P/ask-user-question.ts"
```

**缺点**：插件升级（`npm:` 包重装 / `pi` 包更新）会覆盖补丁，补丁会静默丢失。

### 7.2 方案 B：本地 fork（长期方案）

照 `zl-mnemon` 的既有模式（`~/.pi/agent/settings.json` 里已有 `"../../zWork/ai/pi/plugins/zl-mnemon"` 这种本地路径条目）：

```bash
cp -R ~/.pi/agent/npm/node_modules/@juicesharp/rpiv-ask-user-question \
      ~/zWork/ai/pi/plugins/rpiv-ask-user-question-tui
# 打同样的补丁
```

然后 `~/.pi/agent/settings.json`：

```diff
-    "npm:@juicesharp/rpiv-ask-user-question",
+    "../../zWork/ai/pi/plugins/rpiv-ask-user-question-tui",
```

**注意**：本地路径包需要能解析依赖（`@juicesharp/rpiv-config`、`@earendil-works/pi-tui` 等）—— 可用 `node_modules` 软链回 npm 版本，或确认 pi 的本地包解析规则。

**优点**：升级不覆盖，补丁可加注释、可自己维护。
**缺点**：需手动同步上游更新。

### 7.3 方案 C：零改动（不改代码时的应对约定）

- **不要按 Esc 试图"退一题"** —— 那是整份作废。
- 发现选错：**继续答完**，然后在聊天里直接更正（「第 1 题改选 B」）。问卷结果以 `"问题"="答案"` 文本进入上下文，紧邻的自然语言更正是最高优先级的修正。
- 或者当场用每题末尾的自定义答案行写明「改：B」—— 但会让答案变成 `kind:"custom"`，语义上不再是选项。
- **agent 侧纪律**：在 Pi Web Desktop 下少问、问短；把最容易被推翻的判断留在正文讨论，问卷只用于"选错也无所谓"的轻量确认。

### 7.4 方案 D（未评估）：走上游 issue

插件把 `mode === "rpc"` 当成 `custom()` 不可用的充分条件，这个假设已不成立。可向上游提案：闸门应改成 **"`mode === 'rpc'` 且 `custom()` 实测不可用"**，或引入显式能力协商（如宿主声明 `supportsCustomUi: true`）。
另外上游已有 `HOST_ALLOCATED_ROWS`（`presentation.ts`，`Symbol.for("rpiv-ask-user-question:host-allocated-rows")`）这种"宿主打标记"的先例，可作为能力协商的现成范式。

---

## 8. 验收标准与结果

### 8.1 自动化验证（离线宿主模拟，已通过）

验证工具：[`../scripts/verify-ask-user-question.mjs`](../scripts/verify-ask-user-question.mjs)
—— 按 pi-web 0.15.0 的 extension UI 契约复刻一个最小宿主（`ui.custom` / `tui` shim 92×40 /
纯文本 theme / ANSI 按键映射），直接驱动插件的 `tool.execute()`，**不启动 App**。

| 验收项 | 结果 | 证据（脚本场景） |
|---|---|---|
| 多题问卷出现标签栏，已答题带 ■、当前题高亮、有 Submit 标签 | ✅ | 场景 1（原文档写"✓ / `[]`"，实际渲染是 ■/□ + ←/→ + ✓ Submit） |
| `Shift+Tab`/`←` 退回上一题，且光标停在原选项 | ✅ | 场景 1（`Docker ✔`、`Postgres ✔`） |
| 改选后 `Enter` 前进，答案被覆盖而非新增 | ✅ | 场景 1（3 个答案，第 1 题 Postgres → SQLite） |
| 最后一题后出现 Review 页，Review 页仍可回退 | ✅ | 场景 1 |
| 多选渲染为复选框，`Space` 勾选，Submit 行提交 | ✅ | 场景 6（`[✔] 搜索` / `[✔] 导出`） |
| `Esc` 取消整份问卷（行为不变） | ✅ | 场景 2 |
| 单题问卷不回归 | ✅ | 场景 3 |
| 真 RPC 宿主（Zed/VSCode 类）仍走原生对话框 | ✅ | 场景 4 |
| 宿主 `custom()` 返回 undefined 时兜底回 RPC、不报 `no_custom_ui` | ✅ | 场景 5 |
| 92 列下 preview 上下堆叠、不溢出 | ✅ | 场景 7 |
| 渲染宽度 ≤ 宿主给的 92 列；`render()` 返回字符串数组 | ✅ | 场景 1 / 7 |
| 内联挂载不注册 `onTerminalInput`（不踩 pi-web 的空实现） | ✅ | 场景 1 |
| 数字键 `1–9` 快选 / `n` 备注 | ⚠️ 未单独断言 | 同一套 key-router 代码路径；真机点一遍即可 |
| **回滚**后回到一题一对话框 | ✅ | `--revert` 后重跑脚本 → 场景 1/2/3/6/7 按预期失败（7 passed / 5 failed） |

补丁前对照：`node scripts/verify-ask-user-question.mjs --before`（自动把 `.orig-2.15.0` 复制到临时目录运行）
→ `ui.custom()` 一次都没被调用、3 题 = 3 个独立原生 `select`、中途关框 = `cancelled:true` + 模型只收到
DECLINE 文本（**7/7 断言成立**）—— bug 可复现。

### 8.2 仍需真机验收（子龙在 Pi Web Desktop 里点一遍）

- [ ] 面板观感：pi-web 的 extension 面板是模态框（`chat.extensionPanel`），自定义 UI 以等宽文本行呈现；确认 92 列下中文换行/滚动/底部提示可读。
- [ ] 键盘：`Tab` / `Shift+Tab` / `↑` `↓` / `Enter` / `Esc` 在 GUI 焦点下都生效。
- [ ] 鼠标：pi-web 的 custom UI **只转发键盘**，选项不能鼠标点（宿主限制，不算回归）。
- [ ] 面板右上角按钮：「收起」只隐藏面板；「停止」发的是 `Ctrl-C`（`\x03`），而问卷只认 `Esc` 取消 —— 确认点它不会把问卷卡死（预期 Ctrl-C 被忽略）。
- [ ] 中文/emoji 在 92 列下的对齐（模拟宿主与真机渲染同源，但需人眼确认）。
- [ ] 插件升级/重装后补丁可能被覆盖：`node scripts/patch-rpiv-ask-user-question.mjs --check` 应返回 0。

**原「未验证事项」结案**：
1. ✅ `ui.custom` 在 92×40 下渲染稳定、每行 ≤92 列（场景 1/7，组件 `render()` 零抛错）。
2. ✅ preview 在 92 列下上下堆叠在选项列表下方（场景 7）；≥100 列才并排的上游逻辑未动。
3. ✅ 只改本插件闸门；真 RPC 宿主回归通过（场景 4），不触碰宿主/其他插件。
4. ✅ 内联 presentation 不注册 `onTerminalInput`（`hideOnCollapse` 只在 overlay 模式为真），teardown 无抛错。

---

## 9. 排查时踩过的坑（避免重复）

1. **先入为主查错了库**：本机同时装了 `@narumitw/pi-tui-kit`（内含 `questionnaire.js` / `components/questionnaire.js`，有完整的 `Tab`/`Shift+Tab`/`j`/`k` 翻页实现）。那套代码**根本没被加载**。判断"哪个插件提供了某个工具"要以 `~/.pi/agent/settings.json` 的 `packages` + 实际 `node_modules` 为准，不能靠文件名猜。
2. **上游文档会误导**：插件自带 `docs/keyboard.md` 写着"`Tab`/`Shift+Tab` 切换标签页"—— 那是 TUI 路径的键位。必须先确认宿主走的是哪条路径，再去读对应的键位。
3. **`docs/hosts.md` 里没有 pi-web**：只列了 terminal / RPC-ACP / non-interactive 三类，pi-web 被归入 RPC（因为 `mode:"rpc"`），但它的 `custom()` 能力其实超出了 RPC 类的定义。
4. **人机分工**：`ctx.mode` 是 pi-core 传给扩展的宿主声明，属"约定"而非"能力探测"；任何依赖它的分支都可能因宿主能力超集而判错。
   **但注意（2026-11-08 修订）**：按 pi 官方文档，插件这么做是守约的（§1 复盘）；缺的是 pi-core 没有能力协商这一层，
   所以别把锅算在插件作者头上 —— 上游提案/生态审计见 §12、§13。

---

## 10. 时间线

| 时间 | 事件 |
|---|---|
| 2026-11-08 | 用户反馈：Pi Web Desktop 下多题问卷无法回退（`←`/`Shift+Tab` 无反应） |
| 2026-11-08 | 首轮误判：分析了 `@narumitw/pi-tui-kit` 的 questionnaire 组件，给出错误键位 |
| 2026-11-08 | 确认实际插件为 `@juicesharp/rpiv-ask-user-question@2.15.0` |
| 2026-11-08 | 定位 `ctx.mode === "rpc"` 闸门 → RPC 单向走查；确认 pi-web 的 `custom()` / TUI shim / 键盘映射均可用 |
| 2026-11-08 | 给出方案 A（就地补丁）/ B（本地 fork）/ C（零改动应对）/ D（上游提案） |
| 2026-11-08 | 本文档留档（当时未改代码） |
| 2026-11-08 | 实施方案 A：闸门加「宿主确实没有 `custom()`」条件；备份原版（`.orig-2.15.0`）与补丁后快照（`.bak-20261108-patched`） |
| 2026-11-08 | 仓库新增 `scripts/patch-rpiv-ask-user-question.mjs`（幂等打补丁/检查/还原）与 `scripts/verify-ask-user-question.mjs`（离线宿主模拟验证） |
| 2026-11-08 | 补丁前对照 7/7 成立（`ui.custom` 未调用、3 题 = 3 个原生对话框、关框 = DECLINE） |
| 2026-11-08 | 补丁后离线验证 **51/51 通过**（标签栏 / Shift+Tab 回退 / 改答案覆盖 / Review / 多选 / preview / RPC 回归 / 兜底）；待真机验收 |
| 2026-11-08 | 复盘根因层次：确认插件按 pi-core 契约（rpc ⇒ custom 返回 undefined）写法无误；根因是契约缺能力协商 + pi-web 为超集宿主（§1 修订） |
| 2026-11-08 | 生态审计：`scripts/check-extension-host-compat.mjs` 扫本机 12 个扩展，7 个涉及宿主能力判断；4 个不同作者各自打补丁/降级（§12） |
| 2026-11-08 | 新增宿主 `mode="tui"` 可回滚实验开关 `scripts/patch-pi-web-tui-mode.mjs`；打补丁后 `npm run smoke` 通过、`--revert` 干净还原；真机实验待做（§13） |

---

## 11. 实施与验证记录（2026-11-08，方案 A）

### 11.1 改动内容

唯一代码改动 —— 插件闸门（`~/.pi/agent/npm/node_modules/@juicesharp/rpiv-ask-user-question/ask-user-question.ts`，
npm 2.15.0）：

```diff
-			const viaRpc = (ctx as { mode?: string }).mode === "rpc" && hasDialogUI(ctx.ui);
+			const viaRpc =
+				(ctx as { mode?: string }).mode === "rpc" &&
+				hasDialogUI(ctx.ui) &&
+				typeof (ctx.ui as { custom?: unknown }).custom !== "function";
```

比对 npm 2.15.0 tarball：整包只有 `ask-user-question.ts` 这一个文件不同，且差异只在这一段闸门 ——
其余文件与上游逐字节一致，没有夹带别的改动。

意图：`mode === "rpc"` 不再等价于「`ui.custom()` 不可用」。pi-web 的 `custom()` 是真实现
（渲染 `render(width)` + ANSI 按键回灌），所以它走 TUI；Zed/VSCode 那类 custom 确实不可用的宿主
仍然走 RPC 走查（一开始就判 false 的路径不变），而「custom 存在但解析成 undefined」由插件原有的
`resolveUndefinedResult()` 兜底回 RPC —— 因此没有"变坏"分支（详见 §4.9）。

备份（同目录，供回滚/对照）：

| 文件 | 内容 |
|---|---|
| `ask-user-question.ts.orig-2.15.0` | 补丁前 npm 2.15.0 原版；已与 `npm pack @juicesharp/rpiv-ask-user-question@2.15.0` 的 tarball 逐字节比对一致（SHA256 `f9c13238…f13deb9`），`--revert` 的锚点也来自它 |
| `ask-user-question.ts.bak-20261108-patched` | 打完补丁后的快照（幂等/回滚测试的对照物） |

仓库新增两个脚本：

| 脚本 | 作用 |
|---|---|
| [`../scripts/patch-rpiv-ask-user-question.mjs`](../scripts/patch-rpiv-ask-user-question.mjs) | 幂等打补丁；`--check`（已打=0/未打=2）；`--revert`；锚点/版本不符会报错而不是乱改 |
| [`../scripts/verify-ask-user-question.mjs`](../scripts/verify-ask-user-question.mjs) | 离线复刻 pi-web 宿主契约：7 组场景 51 条断言（含 RPC 回归与兜底）；`--before` 用原版快照复现 bug（7 条断言）；检测到未打补丁时快速失败并提示 |

### 11.2 生效方式

扩展源码在进程启动时加载，改完需要**重启 Pi / Pi Web Desktop**（不是刷新页面）才生效；
终端里的 TUI 用户不受影响（`mode !== "rpc"`，本来就走 TUI 路径）。

### 11.3 回滚

```bash
node scripts/patch-rpiv-ask-user-question.mjs --revert   # 还原成 npm 原版
# 等价的手动回滚：cp ask-user-question.ts.orig-2.15.0 ask-user-question.ts
# 然后重启 Pi Web Desktop，行为回到「一题一个原生对话框」
```

### 11.4 升级后重打（重要）

插件由 `~/.pi/agent/npm` 管理（`package.json` 里是 `^2.15.0`），`pi update` 或重装扩展会**覆盖**
这个补丁。升级后先 `node scripts/patch-rpiv-ask-user-question.mjs --check`；
返回 2 就重跑一次补丁，再跑 `node scripts/verify-ask-user-question.mjs` 确认 51/51。
长期方案仍是 §7.2 的本地 fork（本轮未做，等真机验收通过后再决定是否有必要）。

### 11.5 本轮没做 / 仍待确认

- 真机 GUI 观感与鼠标行为（§8.2 清单）—— 要子龙本人验收。
- 未向上游提 issue（§7.4）；本机补丁与上游修复不冲突，上游若采用"能力探测"写法，未来可去掉本补丁。
- 数字键 / `n` 备注只在代码路径上覆盖，脚本未逐键断言。

---

## 12. 生态影响面：同类插件审计（2026-11-08）

### 12.1 判定依据（上游契约原文，pi 1.5.0）

- `docs/extensions.md` → **Mode Behavior**：

  | Mode | `ctx.mode` | `ctx.hasUI` | Notes |
  |---|---|---|---|
  | Interactive | `"tui"` | `true` | Full TUI with terminal rendering |
  | RPC (`--mode rpc`) | `"rpc"` | `true` | Dialogs / notifications via JSON protocol; **`custom()` returns `undefined`, component factories are ignored** |
  | JSON / Print | `"json"` / `"print"` | `false` | UI 方法为空 |

  原文：`Use ctx.mode === "tui" before TUI-specific features (custom(), component factories, terminal input). Use ctx.hasUI before dialog and notification methods that work in both TUI and RPC modes.`
- `docs/rpc-extension-ui.md` → **Limitations**：`custom()` returns `undefined`；`onTerminalInput()` 返回 no-op unsubscribe；footer/header/编辑器/主题等为空实现。
- pi-web 实际：`custom` **真实现**、`onTerminalInput` 空实现 → 能力是"不完整超集"，任何一档 `mode` 都描述不准。

### 12.2 本机扫描结果

工具：`node scripts/check-extension-host-compat.mjs`（只读；扫 `~/.pi/agent/settings.json` 里启用的扩展）。
本次 12 个扩展中 7 个涉及宿主能力判断：

| 扩展 | 版本 | mode 闸门 | custom | onTermInput | 在 Pi Web 下的实际表现（人工判定） |
|---|---|---|---|---|---|
| `@juicesharp/rpiv-ask-user-question` | 2.15.0 | rpc×1 | 1 | 2 | **本 bug**：退化成"一题一原生对话框"，主交互有损 → 已打补丁（§11） |
| `pi-subagents` | 0.86.0 | rpc×3 / tui×3 | 5 | 6 | 组件 UI 降级；**自带能力开关** `PI_SUBAGENTS_COMPONENT_UI=1` |
| `@narumitw/pi-btw` | 0.63.5 | tui×3 | 10 | 0 | `/btw` 直接提示"requires interactive TUI mode"（需要真终端，合理） |
| `@narumitw/pi-goal` | 0.54.16 | tui×1 | 0 | 0 | 菜单降级为文本输出（安全，但少了交互） |
| `@quintinshaw/pi-dynamic-workflows` | 3.15.0 | rpc×1 / tui×5 | 4 | 0 | 导航器降级为文本列表（安全）；注释明确"RPC 下 custom 不生效即使 hasUI=true" |
| `pi-vision-handoff` | 0.10.13 | tui×1 | 1 | 0 | 模型选择退化为 `select` 对话框（安全） |
| `pi-mcp-adapter` | 7.0.1 | tui×6 | 1 | 0 | 设置面板不可用；**注释写明**：rpc 下 `custom()` 的 `done` 不触发，`new Promise(resolve=>custom(...))` 会永不 settle、命令挂死 |

其余 5 个无相关用法：`pi-web-access`、`@plannotator/pi-extension`、`@juicesharp/rpiv-todo`、`context-mode`、`zl-mnemon`。

### 12.3 结论

1. **这不是单插件问题**：4 个不同作者在同一处各自设闸/降级，`pi-subagents` 甚至自造能力开关、`pi-mcp-adapter` 用注释记录了挂死风险 —— 生态在用自己的办法补 pi-core 缺的那一层。
2. **风险等级不同**：只有本插件是"主交互有损"；其余是"功能缺失但安全降级"。
3. **不能一刀切把宿主声明改成 `tui`**：`pi-btw` 需要全屏终端，`pi-mcp-adapter` 明确写了误判会挂死（§13 详述）。逐插件判断才安全。
4. 复跑方式：`node scripts/check-extension-host-compat.mjs`（或 `--json`）。建议每次装/升级扩展后跑一次，人工判定栏参考本节。

---

## 13. 实验：让宿主直接声明 `mode="tui"`（可回滚，未默认启用）

### 13.1 为什么值得试

pi-core 契约里 `tui` 是"完整 TUI"，`rpc` 是"无 custom"；pi-web 卡在中间。若把它声明成 `tui`，
所有按 `mode` 设闸的插件都会走完整 TUI/custom 路径 —— **如果 pi-web 撑得住，这就是最接近"本地根除"的方案**。
反过来，这也顺带回答了"pi-web 到底算不算 TUI 宿主"。

### 13.2 已知撑不住/没把握的地方（pi-web 的 TUI 是子集）

| 契约预期（tui） | pi-web 实际 |
|---|---|
| `ui.onTerminalInput` 原始按键 | ❌ 空实现 `()=>()=>{}` |
| `custom` 的 `onHandle` / overlay `setHidden()` / `isFocused()` | ❌ 工厂只收到 `(tui, theme, keybindings, done)`，不回调 onHandle |
| `overlayOptions`（anchor/width/maxHeight/margin） | ⚠️ 只认数字 width，其余忽略（默认宽 92） |
| 终端尺寸 | ⚠️ 固定 `92×40` |
| 鼠标、footer/header、`setEditorComponent`、主题切换 | ❌ 无/空实现 |

可能出现：面板能开但键盘操作缺失、组件等不到输入、或命令挂起（用「停止」按钮/Ctrl-C 或重开会话恢复）。
`pi-mcp-adapter` 的注释就是这类风险的现成证据。

### 13.3 怎么开 / 怎么关（默认只改仓库 sidecar，不动 /Applications 里的 App）

```bash
# 1) 打实验补丁
node scripts/patch-pi-web-tui-mode.mjs
node scripts/patch-pi-web-tui-mode.mjs --check    # 0=已开，2=未开

# 2) 起一个独立 pi-web（与 App 共用 ~/.pi；换端口避免冲突）
#    注意 cwd 必须是 pi-web 包目录（侧车 launcher 用 __dirname/.. 定位 .next）
PKG=sidecar/app/node_modules/@agegr/pi-web
(cd "$PKG" && PI_WEB_NO_OPEN=1 PI_WEB_SKIP_VERSION_CHECK=1 \
   ../../../../runtime/node bin/pi-web.js --no-open -p 30999 -H 127.0.0.1)
# 浏览器打开 http://127.0.0.1:30999

# 3) 回滚
node scripts/patch-pi-web-tui-mode.mjs --revert
```

也可以 `--app-dir "/Applications/Pi Web Desktop.app/Contents/Resources/pi-web/app"` 直接改已装 App，
但会让 ad-hoc 签名失效、且 App 升级会覆盖 —— 非必要不建议。

**已做的自动化前置验证**（2026-11-08）：打补丁后 `npm run smoke` 通过（服务器启动 / 页面 200 / 会话 / 导出 / 收尾全绿）；
并且用**真实 `~/.pi`** 起过一次该命令（2s 就绪，`GET /` 200、`/api/sessions` 200），确认 runbook 可用；
`--revert` 后文件与备份一致、`mode:"rpc"` 恢复（`--check` 返回 2），测试服务器已停、run record 已清。

### 13.4 真机验收清单（子龙点，结论记在下面）

- [ ] 侧车方式启动无报错，页面能开、会话能发消息
- [ ] `ask_user_question` 3 题问卷：标签栏 / Shift+Tab 回退 / 改答案 / Review（应与 §11 补丁后一致）
- [ ] `@narumitw/pi-goal`：`/goal` 出现**菜单面板**（而不是文本回退）；Esc 可退出、不卡
- [ ] `@quintinshaw/pi-dynamic-workflows`：`/workflows` 出现导航器；Esc/退出不卡
- [ ] `pi-mcp-adapter`：MCP 设置面板能开能关；若卡住，记录现象与恢复方式
- [ ] `pi-subagents`：组件 UI 能开（`PI_SUBAGENTS_COMPONENT_UI` 不再是必要条件）
- [ ] `@narumitw/pi-btw`：`/btw` 的提示是否变化（预期仍不可用/异常）；确认不破坏主流程
- [ ] 任意会话结束后整体仍正常；「停止」/Ctrl-C 能终止卡住的交互
- [ ] `--revert` 后行为回到现状（`--check`=2，重启生效）

**结论记录（待填）**：

| 项 | 结果 |
|---|---|
| 日期 / 执行人 | |
| 结论 | ⬜ 可作长期方案 ⬜ 部分可用（列不可用项） ⬜ 不可用（列原因） |
| 备注 | |

---

## 14. 相关记忆

- mnemon 关键词：`pi-web-desktop`、`ask_user_question`、`rpiv-ask-user-question`、`rpc 单向路径`、`问卷不可回退`
- 交叉参考：`docs/BUG-2026-11-05-login-path-cache.md`（同样是"宿主声明 / 缓存与实际能力不一致"类问题）
- 上游文档：`@juicesharp/rpiv-ask-user-question/docs/{keyboard.md,hosts.md}`、`docs/configuration.md`（`collapseKey` / `notesKey`，**无 TUI/RPC 切换开关**）
