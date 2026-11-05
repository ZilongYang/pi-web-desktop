# PLAN-2026-11-05 登录 shell PATH 缓存自愈（BUG-2026-11-05-login-path-cache 根因修复）

> 存档规则：批准时原文存档、不修改；实施偏差只在文末「事后补记」追加。
> 状态：**已批准**（2026-11-05）。批准口径：用户选择「方案 B + 升 0.3.3-beta.1，构建并安装本机验证」。
> 对应 BUG：[BUG-2026-11-05-login-path-cache.md](BUG-2026-11-05-login-path-cache.md)（即 ROADMAP 的 B4）。
> 本文第一节是对该 BUG 文档的**修正**；与 BUG 文档旧结论冲突时，以本文为准。

## 一、背景

侧车的 PATH 由 `path_with_runtime()` 拼装：缓存 `login-path.txt` 未过 7 天 TTL 就只用缓存，缓存失效才跑一次登录 shell 探测并写回。nvm 换版本后，缓存里留的是已被删除的版本目录，而缓存有效期内不会重新探测 → App / 侧车 / 终端面板里 `node`、`pnpm`、`mmx`、`defuddle`、`mnemon` 等全局命令静默消失，日志还会误报「可见」。现象、证据与当时的临时修复见 [BUG 文档](BUG-2026-11-05-login-path-cache.md)；本次做的是**代码层面的根因修复**。

## 二、复核结论（对 BUG 文档的修正）

1. 核心判断成立：`path_with_runtime()` 缓存命中时只用缓存、不并入 `fallback_user_bin_dirs()`，nvm 换版本后死路径会存续到 7 天 TTL 过期。
2. 补充真正的根因：缓存命中时**根本不触发**登录 shell 探测（`trigger_login_path_probe()` 只在缓存失效分支调用），所以缓存有效期内"下次启动生效"永远不会发生；两条叠加 = 换版本后永不自愈。
3. 修正 BUG 文档 §5.3/§6：日志里全部 6 次 `超过 1.2s` 均来自 `Pi-Web-Desktop-Smoke.app`（smoke-app.mjs，PIWEB_CONFIG_DIR 隔离 → 空缓存才走探测分支），发生在构建/测试高负载时；真实 App 因缓存有效从未触发探测。2026-11-05 实测同命令：Finder 最小环境 233ms、终端环境 468ms，1.2s 不是当前瓶颈。
4. 修正 BUG 文档 §6 疑点 1：11-04 三次"已缓存，下次启动生效"同样是 Smoke 实例写入**临时配置目录**，而日志写共享真实日志文件 → 造成"没落地"的假象；`write_login_path_cache()` 无写入 bug。
5. 修正 BUG 文档 §5.2："头部注入 `~/.pi/agent/bin`"在壳代码中不存在（那是 pi-web 自己给会话 PATH 加的）；§6 疑点 2（`mnemon 可见` 误报）成立。

## 三、修复方案（已批准）

- **P0a 并集**：`path_with_runtime()` 中缓存优先 + `fallback_user_bin_dirs()` **始终追加**，统一去重保序（缓存→兜底→继承 PATH→包内 runtime）。
- **P0b 自愈**：缓存命中时也判断"是否陈旧"——缓存内含不存在目录（nvm 换版本）或缓存超过 24h → 后台触发探测刷新缓存；缓存过期（7 天 TTL）仍走兜底 + 探测。
- **P1 探测加固**：超时 1.2s → 5s；stdout 改为并发读取（防 rc 输出塞满 64KB 管道导致子进程卡死被误判超时）；同一后台线程内最多 2 次尝试（不再一次失败终身放弃）；失败日志带耗时/原因/是否读到 stdout。
- **可观测性**：`[path]` 日志写明缓存年龄、失效条目数、是否已后台重新探测；`nvm 在/不在` 只认真正存在的版本目录；`mnemon` 改为按 PATH 查找 + 校验可执行位（区分 可见 / 存在但不可执行 / 不可见）；写缓存加绝对路径、条目/字节数与写后回读校验日志。
- **P2（本次实现）**：菜单「Pi Web → 重新载入登录环境」：清 `login-path.txt` → 重新探测（8s 超时）→ 写缓存 → 重启侧车；后台线程执行、防重复触发、失败弹窗（headless 不弹）。
- **P3 清理**：删除 `fallback_user_bin_dirs()` 里已不存在的 `~/.hermes/node/bin`；修正过时注释（"2s 超时"、`~/.local/bin/node` 软链说明）。
- **单测**：覆盖并集顺序/去重、空串、失效目录检测、TTL 与刷新判定、nvm 兜底枚举（含 alias/default 优先、无 hermes）、可执行文件可见性、nvm 版本识别；用临时假 HOME，不改进程全局环境变量。

## 四、验收标准

- `cargo test` 全过（新增 path 相关单测）；`npm run smoke` / `npm run smoke:app` 通过。
- 真机：造"缓存指向不存在 v24.99.0/bin"现场启动新包 → 日志来源为「登录 shell 缓存 + 兜底目录」、缺失条目被识别、后台探测刷新缓存；`pnpm/mmx/defuddle/node` 命中 `~/.nvm/versions/node/v24.22.2/bin`；`mnemon status` 正常；会话内 mnemon 扩展状态行带计数。
- 交给子龙在本机验收通过后，才允许 tag v0.3.3-beta.1。

## 五、风险

- P0a 的并集只保证"兜底目录始终在 PATH 里"，**不等于**能补回 nvm 的版本目录：nvm 路径只能靠 P0b/P1/P2 刷新缓存找回。
- P0b 的「24h」与 7 天 TTL 是两条不同阈值：24h 只触发**后台**探测、不阻塞启动；探测失败不刷新缓存，只能等下个周期或用户手动「重新载入登录环境」。
- P1 把超时放宽到 5s，只影响"缓存失效、必须等探测"这条路径；该路径启动耗时不明显变长属真机验收项。
- P2 会重启侧车，**正在跑的会话会中断**；方案只规定了「后台线程执行、防重复触发、失败弹窗（headless 不弹）」，验收时按这三条核对，不要把它做成自动行为。
- 真机验收要人工构造"缓存指向不存在目录"的现场，验证后需恢复本机正常缓存，别把构造出来的坏缓存留在机器上。
- 本文第二节已推翻 BUG 文档 §5.2/§5.3/§6 的部分旧结论，验收对照时不要再引用那些旧结论当证据。

## 六、事后补记（实施记录，0.3.3-beta.1）

> 按存档规矩，正文不动；实施与批准方案的差异记在这里。

**实施范围**：`src-tauri/src/sidecar.rs`（主要）、`src-tauri/src/menu.rs`（「重新载入登录环境」菜单项 + `on_event` 分支）、`src-tauri/src/update.rs`（`headless()` 改 `pub(crate)` 供菜单动作复用）、五处版本号 → `0.3.3-beta.1`。

**与批准方案的差异（及原因）**：

1. **P2 改为「成功才覆盖」**：原方案是「清 `login-path.txt` → 重新探测 → 写缓存 → 重启侧车」；实现改为**探测成功才覆盖缓存**，失败保留旧缓存（避免一次探测失败把还能用的缓存也清掉），失败弹窗（headless 不弹）。
2. **新增写缓存前清洗 `normalize_login_path()`**（批准方案未含）：清洗不存在/空/重复条目。原因：真实登录 shell PATH 自带 5 条永远不存在的绝对伪目录（cryptexd 3 条、`/pkg/env/global/bin`、x-cmd `triarii/bin`）和字面 `~/.dotnet/tools`，原样写回会让「每次启动都判缓存陈旧、反复探测」；清洗后「失效」只反映捕获时存在、之后消失的目录（nvm 换版本）。
3. **探测节流从 `OnceLock`（每进程一次）改为 10 分钟节流**（`AtomicU64` + `probe_allowed()`）：侧车重启后仍可再探测；P1 的「同一后台线程最多 2 次尝试」保留。
4. **5s 超时不阻塞启动**：探测全程在后台线程，启动路径不等它；只有「缓存失效」这条路径本来就要等探测结果（等不到就用兜底）。
5. **单测**：path/缓存相关 14 项（`sidecar.rs` 内），全 crate `cargo test` **28 项全过**。
6. **评审二轮追加的 3 个防御点**：① `cached_dir_state()` 区分「真的不存在（NotFound）」与「stat 失败（EACCES / TCC / 未挂载）」，清洗与陈旧判断统一用这个规则（stat 失败不丢条目、不算失效）；② `normalize_login_path()` 清洗后为空时返回 `None`，`write_login_path_cache()` 不覆盖旧缓存（防止把 rc 横幅当 PATH 写空）；③ 探测 stdout 改「按字节攒、最后统一 lossy 解码」，并区分读错误 / 空输出。对应新增测试 `dir_state_distinguishes_missing_from_present`、`normalize_refuses_to_overwrite_cache_with_garbage`。

**验收状态**：`cargo test` 28 项、`npm run smoke` 通过；隔离测试实例（`~/Applications/PiWebPathTest.app`，配置/日志/agent 隔离）真机验收通过——陈旧缓存首启即并集 + 后台重探测，二次启动不再跑 zsh。`smoke-app` 首轮构建通过，最终构建因用户 App 占用 30241 未重跑（启动路径未变，隔离实例启动/侧车验证已覆盖）。**安装到本机、子龙本机验收、tag `v0.3.3-beta.1` 仍待办**（用户要求不要动正在用的 App）。证据见 [BUG 文档 §11](BUG-2026-11-05-login-path-cache.md) 与 [QA 2026-11-05](QA-2026-11-01.md)。
