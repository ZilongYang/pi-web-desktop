# BUG 记录 —— 登录 shell PATH 缓存陈旧，导致 App 内新装的全局命令全部消失

- **发现时间**：2026-11-05 04:00 前后
- **现场版本**：Pi Web Desktop 0.3.2（本机 `/Applications/Pi Web Desktop.app`，bundle id `com.zilong.piweb.desktop`）
- **严重级别**：中（不崩溃，但 App 内所有"终端里能跑、App 里跑不了"的命令都会中招，且静默、无提示）
- **状态**：**已在本机用数据文件层面的临时修复解决**（见 §5），**代码层面的根因修复未做**（见 §7）
- **本文用途**：交给另一个会话，按 §7 的方案改 `src-tauri/src/sidecar.rs` 并出包

---

## 1. 摘要（TL;DR）

`path_with_runtime()` 给侧车拼 PATH 时，只要 `login-path.txt` 缓存"没过 7 天"，就**只用缓存**，不再并入兜底目录。
而缓存的刷新依赖一次 `zsh -lic` 探测，这次探测**从 Finder/Dock 启动的 .app 里稳定超时**（实测 1.2s 等不到，见 §4 日志）。

于是形成死循环：

```
nvm 换版本 (v24.14.0 → v24.22.2)
        ↓
缓存里仍写 v24.14.0/bin（目录已不存在）
        ↓
探测又总是超时 → 缓存不刷新（只有 7 天 TTL 兜底自愈）
        ↓
App/侧车/agent 工具 / 终端面板 里：node、pnpm、mnemon、mmx、defuddle、playwright… 全部 command not found
        ↓
但没有任何提示（日志里甚至写着 "nvm 在，mnemon 可见"）
```

本次受害最明显的表现：**Pi 的 mnemon 扩展每轮注入的状态行从
`[mnemon] Memory active (111 insights, 2360 edges).` 退化成裸的 `[mnemon] Memory active.`**
（因为扩展内部跑 `mnemon status` 失败）。这个降级形态可以当成"侧车 PATH 是否健康"的免费探针。

---

## 2. 现象

1. 终端（iTerm/Terminal）里 `mnemon`、`pnpm`、`mmx` 都正常；在 Pi Web Desktop 的会话/终端面板里全都 `command not found`。
2. `~/Library/Application Support/com.zilong.piweb.desktop/login-path.txt` 的 mtime 停在 **2026-11-01 23:54**，内容里是 `/Users/zilong/.nvm/versions/node/v24.14.0/bin`，而该目录**不存在**（nvm 里只剩 v24.22.2）。
3. 重启 App 无效——重启只是重新读同一个缓存文件，不会重新捕获。
4. `sidecar.log` 里反复出现 `登录 shell 取 PATH 超过 1.2s（已等 1223ms），放弃`。

---

## 3. 自查命令（以后遇到同类症状先跑这三条）

```bash
# ① 缓存长什么样、多久没更新
ls -la ~/Library/Application\ Support/com.zilong.piweb.desktop/login-path.txt
tr ':' '\n' < ~/Library/Application\ Support/com.zilong.piweb.desktop/login-path.txt | head -20

# ② 缓存里有多少条路径其实已经不存在（>0 就有问题）
python3 -c "
import os,glob
p=os.path.expanduser('~/Library/Application Support/com.zilong.piweb.desktop/login-path.txt')
ps=open(p).read().split(':')
print('条目',len(ps),'缺失',[x for x in ps if x and not os.path.isdir(x)])"

# ③ 侧车实际拿到的 PATH 与来源（有现成日志，不用猜）
grep -a '\[path\]' ~/Library/Logs/com.zilong.piweb.desktop/sidecar.log | tail -5
```

关键日志格式（`sidecar.rs:616`）：

```
[path] 侧车 PATH 43 项（来源：登录 shell 缓存，nvm 在，mnemon 可见）：/Users/zilong/.nvm/versions/node/v24.22.2/bin : ...
```

> ⚠️ **注意**：这条日志里的 `mnemon 可见` 只判断了"某个 PATH 目录下存在名为 mnemon 的文件"（`Path::join("mnemon").is_file()`），
> 并不判断可执行、也不判断该目录本身是否还存在。本次故障中它一度显示"可见"而实际不可用（见 §6 疑点 2）。

---

## 4. 机制（代码地图）

全部集中在 `src-tauri/src/sidecar.rs`：

| 位置 | 作用 | 关键行为 / 常量 |
|---|---|---|
| `Config.inherit_login_shell_path`（L63–67，默认 `true` L115） | 开关：是否继承登录 shell 的 PATH | 存在 `~/Library/Application Support/<bundle>/config.json` 里 |
| `login_path_cache_file()`（L364） | 缓存文件位置 | `app_dirs::config_dir()/login-path.txt` |
| `probe_login_shell_path()`（L372） | 跑 `zsh -lic 'printf %s "$PATH"'` 取 PATH | **超时 1.2s**（L393），超时就放弃 |
| `trigger_login_path_probe()`（L440） | 后台探测，成功后写缓存 | 用 `OnceLock` 保证**每进程只跑一次**；注释明确"下次启动生效" |
| `read_login_path_cache()`（L452） | 读缓存 | **7 天 TTL**（L463）：`age > 7*24*3600` 才返回 None |
| `write_login_path_cache()`（L469） | 写缓存 | `let _ = fs::write(...)` —— **失败被静默吞掉** |
| `fallback_user_bin_dirs()`（L480） | 兜底目录列表 | 按 nvm `alias/default` 优先 + 版本号从新到旧枚举 `~/.nvm/versions/node/*/bin`；末尾还硬编码了已不存在的 `~/.hermes/node/bin`（L564） |
| `path_with_runtime()`（L573） | 最终拼 PATH | 顺序：**登录 shell 缓存 → ‖（缓存失效时才用）兜底目录 → 继承的 PATH → 包内 runtime（最后）** |

设计上的两个致命点：

1. **互斥而非并集**：`if let Some(cached) = read_login_path_cache() { 只用缓存 } else { 用兜底目录 }`。
   缓存一旦"命中且未过 7 天"，兜底目录里那份**自动枚举出来的正确 nvm bin 就不会被采用**。
2. **自愈依赖一个不可靠的探测**：缓存唯一的主动刷新路径是 `probe_login_shell_path()`，而它在 GUI 启动环境下超时（§5 日志）。

顺带（同一处代码里的过时注释/兜底）：
- L485 注释说"终端里 `node` 命中的是 `~/.local/bin/node`（不是 nvm 那个）"——该软链已于 2026-11-05 删除，注释已过期。
- L564 的 `~/.hermes/node/bin` 兜底目录同样已不存在（hermes 自带 node 已迁到 `~/.hermes/tools/node-*`）。

---

## 5. 证据（本机实测，可直接引用）

### 5.1 缓存 vs 现实

```
缓存：50 条，含 /Users/zilong/.nvm/versions/node/v24.14.0/bin   ← 目录不存在
现实：~/.nvm/versions/node/ 下只有 v14.21.2 v16.19.0 v18.19.1 v20.19.6 v22.21.1 v24.22.2 v26.8.2
     npm -g 的 mnemon / mmx / defuddle / command-code / playwright 全在 v24.22.2/bin
缓存中不存在的目录共 8 个：
  ~/.nvm/versions/node/v24.14.0/bin
  ~/.hermes/node/bin                      ← hermes 自带 node 已于 11-05 02:20~02:30 被删
  ~/.x-cmd.root/local/data/triarii/bin
  /pkg/env/global/bin
  ~/.dotnet/tools                         ← 被捕获成字面量 "~/.dotnet/tools"，波浪号未展开
  /var/run/com.apple.security.cryptexd/codex.system/bootstrap/usr/{local/bin,bin,appleinternal/bin}
```

### 5.2 侧车 PATH 与进程 PATH 的关系

App 侧车最终 PATH = **缓存文件原样（50 条）+ 头部注入 `/Users/zilong/.pi/agent/bin` + 尾部追加 `.../pi-web/runtime`**，
说明**缓存里的内容一条不校验、一条不修，直接进 PATH**。

### 5.3 sidecar.log 时间线（关键片段）

```
11-04 03:56:10  [path] 登录 shell 取 PATH 超过 1.2s（已等 1227ms），放弃；改用缓存/常见目录兜底
11-04 03:56:58  [path] 登录 shell PATH 取回 59 项（668ms）
11-04 03:56:58  [path] 登录 shell PATH 已缓存，下次启动生效
11-04 14:02:40  [path] 登录 shell PATH 取回 59 项（665ms）
11-04 14:02:50  [path] 登录 shell PATH 取回 59 项（459ms）
11-04 20:31:14  [path] 登录 shell 取 PATH 超过 1.2s（已等 1223ms），放弃
11-04 21:34:10  [path] 登录 shell 取 PATH 超过 1.2s（已等 1203ms），放弃
11-05 04:03:04  [path] 侧车 PATH 49 项（来源：登录 shell 缓存，nvm 在，mnemon 不可见）  ← hermes node 被删后第一次翻转
11-05 04:18:13  [path] 侧车 PATH 43 项（来源：登录 shell 缓存，nvm 在，mnemon 可见）：/Users/zilong/.nvm/versions/node/v24.22.2/bin : ...  ← 临时修复后
```

全量统计（本机 sidecar.log 80 条 path 日志）：

| 指标 | 数值 |
|---|---|
| 探测超时 | **6 次** |
| 探测成功 | 3 次（全部集中在 11-04 03:56–14:02，同一天） |
| 侧车 PATH 来源 | `登录 shell 缓存` 59 次 / `常见目录兜底（登录 shell 后台探测中）` 9 次 |
| 缓存 mtime | **2026-11-01 23:54**（与 11-04 的三次"已缓存"矛盾，见 §6 疑点 1） |

### 5.4 连带症状（同源，别的进程也踩过）

同一台机器上同类"残留 v24.14.0 路径"的问题至少还有两处，都是 nvm 换版本后留下的：

- `~/Library/LaunchAgents/ai.hermes.gateway.plist` 的 `EnvironmentVariables.PATH`（已于 11-05 手工改正）
- `com.dsh.doctor` 僵尸 LaunchAgent（指向已删除的 `v24.14.0/bin/node`，已 bootout 删除）

> 结论：**"nvm 换版本"在本机是一个高频触发源**，凡是缓存/固定写入 PATH 的地方都必须能自愈。

---

## 6. 未解疑点（请接手时顺带查）

1. **11-04 三次"已缓存，下次启动生效"没有落地**：日志说取回 59 项并已写缓存，但 `login-path.txt` 的 mtime 仍是 11-01、内容仍是 50 项（≠59）。
   只找到这一个 `login-path.txt`（`find ~/Library/Application Support -maxdepth 3 -name login-path.txt` 仅一条）。
   怀疑：① `write_login_path_cache()` 的 `let _ =` 吞掉了写入错误；② 写入时 `app_dirs::config_dir()` 解析到了临时/开发态目录（当天在做 0.3.2 构建）；
   ③ 写入后又被别的实例覆盖。→ 建议先给写缓存加上"写后回读校验 + 记录绝对路径与字节数"的日志（§7 P2）。
2. **`mnemon 可见` 判定失真**：日志在一次路径全部失效的情况下仍报"可见"（它只做 `dir/mnemon` 的 `is_file()` 判断，而该"可见"来自缓存里已被删除的 `~/.hermes/node/bin` 残留）。建议改成"按 PATH 语义查找 + 校验可执行位"，并区分 `可见 / 存在但不可执行 / 不可见`。
3. **探测为什么在 Finder 启动下会卡住 >1.2s**：同一条 `zsh -lic 'printf %s "$PATH"'` 在终端里 0.3–1.2s 就回。候选原因：`.zshrc` 里的 nvm 懒加载首次初始化、antigen、x-cmd、conda hook、以及无 tty 时某些 rc 分支等待。需要一次带计时的分解实验（`zsh -x` 输出到临时文件，仅调试用）。

---

## 7. 待办：代码层面的根因修复方案

### P0（必须，最小改动，5 行以内）：缓存与兜底目录取**并集**

改动点：`path_with_runtime()`（L573 起）。

```rust
if config.inherit_login_shell_path {
    if let Some(cached) = read_login_path_cache() {
        part_extend(&mut parts, &cached);
        source = "登录 shell 缓存 + 兜底目录";
        // 关键：缓存永不自证完整，兜底目录始终并集（后面统一去重，顺序保持"缓存优先"）
        for dir in fallback_user_bin_dirs() {
            parts.push(dir);
        }
    } else {
        for dir in fallback_user_bin_dirs() { parts.push(dir); }
        source = "常见目录兜底（登录 shell 后台探测中）";
        trigger_login_path_probe();
    }
}
```

效果：即使缓存里写的是死版本目录，兜底枚举出来的 `~/.nvm/versions/node/v24.22.2/bin` 依然在 PATH 里（排在缓存之后、继承 PATH 之前），**新装的全局工具立刻可见**。
风险：PATH 条目变多（本机 43 → 约 70 项），但 `retain` 已去重；查找开销可忽略。

可选附带：拼完 PATH 后 `parts.retain(|p| p.is_dir())`（只让日志/环境整洁，对功能无影响——不存在的 PATH 条目本身无害）。

### P1（必须）：让探测真的能成功

- 超时 1.2s → **3~5s**；失败时日志里带上"最后读到几项/是否拿到 stdout"，便于排障。
- 探测改为"失败也记录失败原因"，并且**允许每个进程重试一次**（当前 `OnceLock` 让一次失败就意味着本进程永远不再探测）。
- 长期方案：不必跑整个 `zsh -lic`。既然 `fallback_user_bin_dirs()` 已经能正确枚举 nvm/包管理器目录，可以把**兜底目录当作一等公民**，探测仅用于补充"用户 rc 里自定义的条目"。

### P2（建议）：可观测性 + 手动刷新入口

- `write_login_path_cache()` 改为：写入前记录目标绝对路径、字节数、条目数；写入后回读校验；失败打 `[path] 缓存写入失败：{err}`。
- 写入内容与"本次实际使用的 PATH"不一致时，在托盘/菜单提示一次"PATH 已更新，重启生效"。
- 菜单新增 **「重新载入登录环境」**：清缓存 → 重跑探测 → 重启侧车（复用已有的"重启服务 ⌘⇧R"路径）。

### P3（清理）

- 删除 `fallback_user_bin_dirs()` 里已失效的 `~/.hermes/node/bin`（L564），并修正 L485 关于 `~/.local/bin/node` 的过时注释。
- 缓存文件升级为带元数据的 JSON（`{captured_at, shell, path}`），并增加"内部 nvm 版本目录失效即视为过期"的判断，摆脱只看 mtime 的 7 天 TTL。

---

## 8. 验收标准

**单元测试**（`sidecar.rs` 同模块，可用临时 HOME）：

1. 构造一个 cache 内容含 `~/.nvm/versions/node/v24.99.0/bin`（不存在）→ 断言 `path_with_runtime()` 结果**包含**真实存在的 nvm bin 目录与可执行 `mnemon`。
2. cache 存在但超过 7 天 → 断言走兜底分支且触发探测（`LOGIN_PATH_PROBE_STARTED` 行为可测）。
3. cache 内容条目重复/含空串 → 断言去重且无空条目。

**真机验收**：

```bash
# 造一个"陈旧缓存"现场
cp ~/Library/Application\ Support/com.zilong.piweb.desktop/login-path.txt /tmp/lp.bak
printf '%s' "/Users/zilong/.nvm/versions/node/v24.99.0/bin:/usr/bin:/bin" > ~/Library/Application\ Support/com.zilong.piweb.desktop/login-path.txt
# 启动 App，然后：
grep -a '\[path\]' ~/Library/Logs/com.zilong.piweb.desktop/sidecar.log | tail -3   # 应显示并集来源
# 在 App 的终端面板执行
which mnemon node pnpm                                                             # 三个都应命中 v24.22.2
# 还原
cp /tmp/lp.bak ~/Library/Application\ Support/com.zilong.piweb.desktop/login-path.txt
```

外加一条 **端到端探针**：会话里 mnemon 扩展的状态行应显示 `[mnemon] Memory active (N insights, M edges).`；若退化成不带括号计数的形式，即说明侧车 PATH 又不健康了。

---

## 9. 本机已做的临时修复（数据层面，可回滚）

> 起因：本轮对话在定位"mnemon 记忆注入是否生效"时，一路查到 PATH 缓存。修复**只动数据文件**，没有改代码、没有重新出包。

```bash
cd ~/Library/Application\ Support/com.zilong.piweb.desktop

# 1) 备份（改前）
cp -p login-path.txt login-path.txt.bak-20261105-041445     # 1660 B，mtime 11-01 23:54

# 2) 用真实登录 shell 重新捕获，并剔除不存在的目录（50 → 43 条）
zsh -lic 'printf %s "$PATH" > /tmp/pi-path.raw'
#（python 处理：展开 ~、去重保序、只保留 isdir 的项）→ 写入 login-path.txt，无尾换行，644

# 3) 版本无关的兜底 shim（`~/.local/bin/mnemon`，445 B，+x）
#    按 v24.* 逐个覆盖取最后一个（避免 v24.9 > v24.22 的字典序坑），exec 真身
#    注意：新缓存里 nvm bin 排在 ~/.local/bin 之前，所以平时走真身，真身缺失才用 shim
```

修复后实测：

| 检查 | 结果 |
|---|---|
| 新缓存条目 / 不存在的目录 | 43 / **0** |
| `env -i PATH=$(cat login-path.txt)` 下 `which mnemon` | `~/.nvm/versions/node/v24.22.2/bin/mnemon` |
| 同环境下 mmx / defuddle / command-code / playwright / pnpm | 全部可见 |
| `mnemon status` 耗时 | 80 ms（mnemon 扩展超时 5 s，余量充足） |
| 重启 App 后 sidecar.log | `侧车 PATH 43 项（来源：登录 shell 缓存，nvm 在，mnemon 可见）：/Users/zilong/.nvm/versions/node/v24.22.2/bin : …` |
| mnemon 扩展状态行 | 从 `[mnemon] Memory active.` 恢复为 `[mnemon] Memory active (112 insights, 2416 edges).` |

回滚：

```bash
cd ~/Library/Application\ Support/com.zilong.piweb.desktop
cp login-path.txt.bak-20261105-041445 login-path.txt
rm ~/.local/bin/mnemon
```

> 临时修复是"止血"，**根因仍在代码里**（缓存 7 天 TTL + 探测超时 + 互斥而非并集）。下次 nvm 再换版本、或换了台机器，问题会原样复现。

---

## 10. 时间线

| 时间 | 事件 |
|---|---|
| 2026-11-01 23:54 | `login-path.txt` 被捕获成 50 项（含当年有效的 `v24.14.0/bin`） |
| 2026-11-04 03:56–14:02 | 三次探测成功（59 项）并写"已缓存"，但文件 mtime/内容未变（疑点 1） |
| 2026-11-04 20:31 / 21:34 | 探测开始稳定超时 |
| 2026-11-05 02:20–02:30 | 一个 Hermes 会话删除了 `~/.hermes/node`（连带最后一个可用的 mnemon 落点） |
| 2026-11-05 04:03:04 | 日志首次出现 `mnemon 不可见`；mnemon 扩展状态行降级 |
| 2026-11-05 04:14 | 临时修复：重捕获 PATH 缓存（50 → 43 条），备份 `login-path.txt.bak-20261105-041445` |
| 2026-11-05 04:15 | 新增 `~/.local/bin/mnemon` shim |
| 2026-11-05 04:18:13 | 重启 App，日志确认 `43 项 … mnemon 可见`，状态行恢复计数 |

---

## 11. 实施记录（0.3.3-beta.1）

> 本节在实施后追加（正文按规矩不改，旧结论的修正集中写在这里）。
> 状态：**代码修复 + 构建 + 隔离实例真机验收完成**；**安装到本机、子龙本机验收、打 tag/发布待办**——用户明确要求不要动他正在用的 `/Applications` App。

### 11.1 对前文旧结论的修正

1. **§5.3 / §6 疑点 3（探测超时）**：日志里全部 6 次「超过 1.2s」均来自 Smoke 实例（`Pi-Web-Desktop-Smoke.app` / `smoke-app.mjs`；`PIWEB_CONFIG_DIR` 隔离 → 空缓存才会走探测分支），发生在构建/测试高负载时。真实 App 因缓存有效，从未触发过探测。2026-11-05 实测同一条命令：Finder 最小环境 233ms、终端环境 468ms —— 1.2s 不是当前瓶颈。
2. **§6 疑点 1（"已缓存"没落地）**：11-04 三次「已缓存，下次启动生效」同样是 Smoke 实例写入**临时配置目录**，而日志写的是共享的真实日志文件 → 造成"没落地"的假象；`write_login_path_cache()` 没有写入 bug。（本次仍按要求补了写后回读校验日志，用于长期可观测性。）
3. **§5.2（头部注入 `~/.pi/agent/bin`）**：壳代码里没有这个注入，那是 pi-web 自己给会话 PATH 加的；缓存内容确实一条不校验地进了 PATH，但"头部注入"这一条不成立。**§6 疑点 2（`mnemon 可见` 误报）成立**，已改成三态判定。
4. **§5.1 的"8 个不存在目录"要分两类**：其中 5 条是**永远不存在**的绝对伪目录（`cryptexd` 3 条、`/pkg/env/global/bin`、x-cmd `triarii/bin`）+ 字面 `~/.dotnet/tools`（波浪号未展开），属于登录 shell PATH 自带的噪音；真正会变的"失效条目"是 nvm 换版本这类**捕获时存在、之后消失**的目录。修复据此加了写前清洗（见 11.3 ①），否则每次启动都会误判"缓存陈旧"、反复探测。

### 11.2 代码改动清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/sidecar.rs`（主要） | P0a 并集；P0b 陈旧判定 + 后台重探测；P1 探测加固；可观测性；P2 菜单动作；P3 清理；写前清洗；10 分钟探测节流；单测 |
| `src-tauri/src/menu.rs` | 新增「重新载入登录环境」菜单项 + `on_event` 分支 |
| `src-tauri/src/update.rs` | `headless()` 改为 `pub(crate)` 供复用（菜单动作在 headless 下不弹失败窗） |
| `package.json` / `package-lock.json` / `src-tauri/Cargo.toml` / `src-tauri/Cargo.lock` / `src-tauri/tauri.conf.json` | 版本号 → `0.3.3-beta.1` |

具体实现：

- **P0a**：`path_with_runtime()` 缓存优先 + `fallback_user_bin_dirs()` **始终并集**，去重保序（缓存 → 兜底 → 继承 PATH → 包内 runtime）。
- **P0b**：缓存命中时也判陈旧（含不存在目录 **或** 超过 24h）→ 后台触发探测；7 天 TTL 逻辑保留。
- **P1**：探测超时 5s；stdout 并发分块读、按字节攒后统一 lossy 解码（拿不到 EOF 最多再等 1s，用已读内容）；同一后台线程最多 2 次尝试；失败日志带耗时/原因/是否拿到输出。
- **可观测性**：`[path]` 日志带缓存年龄、失效条目数、是否已后台重探测、nvm 生效版本、mnemon 三态（可见 / 存在但不可执行 / 不可见，含绝对路径）；写缓存记录路径/条目/字节数 + 写后回读校验。
- **P2 菜单**：清缓存改为「成功才覆盖」（失败保留旧缓存）+ 8s 超时 + 后台线程 + 防重入（Drop guard）+ 失败弹窗（headless 不弹）+ 成功后重启侧车。
- **P3**：删除 `~/.hermes/node/bin` 死兜底；修正过时注释。

### 11.3 评审后追加（批准方案之外、实施中发现必须补的）

1. **写缓存前 `normalize_login_path()` 清洗**（不存在/空/重复条目）：真实登录 shell PATH 自带 5 条永远不存在的绝对伪目录（cryptexd 3 条、`/pkg/env/global/bin`、x-cmd `triarii/bin`）和字面 `~/.dotnet/tools`；原样写回会让「每次启动都判缓存陈旧、反复探测」。清洗后「失效」只来自**捕获时存在、之后消失**的目录（nvm 换版本）。
2. **探测从「每进程一次 `OnceLock`」改为 10 分钟节流**（`AtomicU64` + `probe_allowed()`）：侧车重启后仍可再探测，不再"一次失败终身放弃"。
3. **探测 stdout 改「按字节攒、最后统一 lossy 解码」**，并区分读错误 / 空输出（二轮评审加固）。
4. **`cached_dir_state()` 区分「真的不存在（NotFound）」与「stat 失败（EACCES / TCC / 未挂载）」**：清洗与陈旧判断统一用这个规则——stat 失败不丢条目、不算失效（二轮评审加固）。
5. **`normalize_login_path()` 清洗后为空时返回 `None`，`write_login_path_cache()` 直接不覆盖旧缓存**：防止把 rc 的横幅/噪音当 PATH 写空（二轮评审加固）。
6. **单测**：path/缓存相关 14 项（`sidecar.rs` 内），全 crate `cargo test` **28 项全过**；二轮新增 `dir_state_distinguishes_missing_from_present`、`normalize_refuses_to_overwrite_cache_with_garbage`。

### 11.4 验收证据（隔离测试实例；用户 App 全程未关闭/未替换）

- 测试实例：`~/Applications/PiWebPathTest.app`（bundle id `com.zilong.piweb.desktop.pathprobe`，配置/日志/agent 目录隔离，单实例锁与用户 App 不冲突）。
- `cargo test`：**28 项全过**（14 秒内）。
- `npm run smoke`：通过（GET / 200、export 278.9 KB、run record 清理、端口释放）。
- `node scripts/smoke-app.mjs`：**首轮构建（review 修复后）通过**（启动行 / 窗口几何 / 侧车端口可连 / 恢复几何 全绿）；**最终构建未重跑**——脚本检测到端口 30241 已有用户正在用的实例在跑会直接拒绝执行（脚本设计如此，不关用户 App 就跑不了）；启动路径未变，隔离实例的启动/侧车验证已覆盖。
- 陈旧缓存现场（第一次启动，缓存 = 不存在的 `v24.99.0/bin:/usr/bin:/bin`）：

```
[path] 侧车 PATH 58 项（来源：登录 shell 缓存 + 兜底目录，缓存 0 分钟前，1 条路径已失效，已后台重新探测，nvm 在 v24.22.2，mnemon 可见 /Users/zilong/.nvm/versions/node/v24.22.2/bin/mnemon）
```

- 探测刷新后缓存：37 项、缺失 0、无字面 `~`。
- 第二次启动：57 项、来源「登录 shell 缓存 + 兜底目录，缓存 0 分钟前」，**无**「已后台重新探测」→ 不再每次启动跑 zsh；`mnemon` 仍解析到 v24.22.2 真身。
- 构建产物：`src-tauri/target/release/bundle/macos/Pi Web Desktop.app`，bundle id `com.zilong.piweb.desktop`，ad-hoc 签名，`codesign --verify --deep --strict` 通过；`CFBundleShortVersionString` = `0.3.3-beta.1`。
- 用户正在用的 `/Applications` App 进程全程未动（验收时长内 `pgrep` 可见）。
- 临时 shim `~/.local/bin/mnemon` 仍在（验收时临时隐藏后已恢复），建议最终安装验证后再由子龙决定是否删除。

### 11.5 当前状态与待办

- 已完成：代码修复、`0.3.3-beta.1` 构建、隔离实例真机验收（含陈旧缓存自愈、二次启动不再探测）。
- **待办**：① 安装到本机（等用户 App 空闲，用户明确要求不要动正在用的 App）；② 子龙在真实 App 里验收；③ 通过后才 tag `v0.3.3-beta.1` 并发布。
- 关联方案存档：[PLAN-2026-11-05-path-selfheal.md](PLAN-2026-11-05-path-selfheal.md)（含事后补记）。

---

## 附：相关记忆（mnemon）

- `4f7c42c1-dc41-481e-b8ec-5421672a7638`（fact, imp 4）：本 bug 的机制、8 条死链清单、修复手法、"状态行降级 = PATH 不健康"探针
- `efb04439-6e4b-49a1-88d5-d730ca5e6b8e`（decision）：同一类"残留 v24.14.0"问题，那次修的是 `ai.hermes.gateway.plist`
- `2d5617fd-e76c-4461-9a7a-da03594a2ceb`（context）：11-05 npm-cli.js 被写坏事件，同一"多套 toolchain + PATH 顺序"根因域
