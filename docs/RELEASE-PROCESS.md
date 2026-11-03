# 发版流程与版本号规则

> 2026-11-03 由子龙确认。这是「以后怎么做」的硬规则，动手发版前先读这一页。
> 背景与方案见 [PLAN-2026-11-03-channels.md](PLAN-2026-11-03-channels.md)（更新通道）。

## 一、版本号规则（默认只递增 patch）

| 场景 | 版本号 | 例子 |
|---|---|---|
| 当前正式版 | `X.Y.Z` | `0.3.0` |
| 下一个测试版 | **`X.Y.(Z+1)-beta.N`** | `0.3.1-beta.1` |
| 同一功能继续迭代 | 后缀递增 | `0.3.1-beta.2`、`0.3.1-beta.3` |
| 该功能转正 | `X.Y.(Z+1)` | `0.3.1` |
| 再下一个功能 | 继续 patch | `0.3.2-beta.1` → `0.3.2` |
| 只有不兼容/重构这类大改动才升 minor | `X.(Y+1).0` | `0.4.0`（少见） |

**默认不要把版本号搞大**：0.3.x 这条线会长期存在，功能更新也走 patch 递增。

技术前提：测试版用「下一个正式号 + 预发布后缀」，所以正式版出来时 `0.3.1 > 0.3.1-beta.N`，
beta 用户切回正式版通道能自然升级，不会卡在「只升不降」。

## 二、标准流程

```
开发（master 提交，ci 绿：侧车 smoke + cargo test）
  ↓
改版本号 → 0.3.1-beta.1（四处：package.json / package-lock.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json）
  ↓
本地验证：npm run smoke / npm run smoke:app / cargo test（必要时真机截图）
  ↓
git tag v0.3.1-beta.1 → push
  ↓ CI 自动：发成 Pre-release，并把 latest.json 覆盖到固定 Release `channel-beta`
本机「Pi Web → 更新通道 → 测试版 beta」→ 在 App 里真实升级一遍
  ↓
试用/验证（有问题就 0.3.1-beta.2 …）
  ↓
版本号改成 0.3.1（去掉后缀）→ tag v0.3.1 → push
  ↓ CI 自动：发 Latest + latest.json（正式版通道）
  ↓
beta 用户切回正式版 → 自然升级到 0.3.1
```

## 三、硬规则

0. **开发完先让子龙在本机验证，通过才发布**（2026-11-04 新增，最高优先级）：
   构建 → 安装到本机 → 交给子龙点一遍 → 他说"通过" → 才 `git tag` + push。
   这条同时适用于 beta 与正式版：**不允许"先发 beta、再让你验"**。
   （自动化的 `cargo test` / `smoke` / `smoke:app` 是前置条件，不能代替本机验收。）
1. **必须真机启动验证过才允许发版**（0.2.9 的教训：CI 全绿但用户双击即崩）。
   发版前至少跑：`cargo test`、`npm run smoke`、`npm run smoke:app`；
   CI 里的「启动冒烟」步骤是最后一道闸。
2. **带新功能/新机制的版本必须先进 beta**，在真机上走一次真实升级，确认后再转正。
3. **紧急热修可以跳过 beta**（仅限「修到无法使用」，如 0.2.9 启动崩溃），
   但三件套必须齐全，并在 RELEASE-CHECKLIST 里写明为什么跳过。
4. **通道语义**：GitHub 的 `Latest` 只认非 pre-release；beta 清单固定在 `channel-beta` 这个
   Release 里，由 CI 覆盖上传，不要手工改。
5. **pi-web 热更新不分通道**：始终跟正式版侧车清单（L3 无 channel 概念）。
6. 发版记录追加到 [RELEASE-CHECKLIST-0.2.0.md](RELEASE-CHECKLIST-0.2.0.md)，验收证据写进
   [QA-2026-11-01.md](QA-2026-11-01.md)，ROADMAP 的「最新版本 / 版本历史」同步更新。

## 四、命令速查

```bash
# 1) 改版本号（V=0.3.1-beta.1）
#    package.json / package-lock.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json

# 2) 本地验证
cd src-tauri && cargo test
cd .. && npm run smoke && npm run smoke:app

# 3) 构建（要 ad-hoc 整包签名，否则资源封印缺失）
APPLE_SIGNING_IDENTITY="-" ./node_modules/.bin/tauri build --bundles app

# 4) 提交 + 打 tag（beta 与正式版同一条命令，版本号决定 CI 行为）
git add -A && git commit -m "..." && git tag -a v<V> -m "v<V>" && git push origin master v<V>

# 5) 发布后核对
gh release list --limit 3          # beta 应为 Pre-release；正式版应为 Latest
gh release download channel-beta -p latest.json -D /tmp && cat /tmp/latest.json | head -3
```

## 五、当前状态（2026-11-03）

- 正式版：`v0.2.11`（含 F20 滚动条修复）
- 测试版：`v0.3.0-beta.2`（更新通道功能；验证完成后转正为 `0.3.0`）
- 之后按本规则：`0.3.1-beta.1` → `0.3.1` → `0.3.2-beta.1` …
