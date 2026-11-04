#!/usr/bin/env bash
#
# 打发布包到 dist/：ad-hoc 签名的 .app、dmg、zip、SHA256SUMS。
# 侧车包。移植自 zWGestures 的 scripts/make-dist.sh（同一套取舍），针对 Tauri 产物改写。
#
# 为什么发布用 ad-hoc 而不是本机自签名证书
# ----------------------------------------
# 对下载者来说两者等价：都过不了 Gatekeeper 的「Developer ID 签名 + 公证」要求。ad-hoc
# 不需要任何证书，任何机器都能复现构建。本机自签名证书（scripts/create-signing-cert.sh）
# 只用于日常构建，让 TCC/授权在重建之间保持稳定。
#
# 这个脚本把几件容易踩的事做成了断言：
#   1. 产物签名必须是 adhoc、架构正确、版本与 tauri.conf.json 一致。
#   2. 产物里不能有调试 entitlement（get-task-allow），不能出现构建者主目录。
#   3. dmg 用 `diskutil image create from` 而不是 `hdiutil create -srcfolder`：后者在
#      macOS 27 上已废弃并直接失败（本项目实测过）。diskutil 需要挂载卷，所以本脚本
#      不能在受限沙箱里跑，要在普通终端里执行。
#   4. 资产上传两份：版本化名字 + 固定名字。下载按钮指向
#      releases/latest/download/<固定名>，固定名不能变。
#
# 产物（dist/）：
#   Pi-Web-Desktop-<ver>-<arch>.dmg / .zip          版本化
#   Pi-Web-Desktop-<arch>.dmg / .zip                固定名（下载按钮）
#   Pi-Web-Desktop-<ver>-<arch>.app.tar.gz(+.sig)   Tauri updater 包（有签名密钥时）
#   SHA256SUMS
#
# 用法：scripts/make-dist.sh
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
REPO_ROOT="$PWD"
DIST="$REPO_ROOT/dist"
BUNDLE_DIR="$REPO_ROOT/src-tauri/target/release/bundle"
HOME_DIR="${HOME:-}"

note() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m  OK\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m  FAIL %s\033[0m\n' "$*" >&2; exit 1; }

ARCH="$(uname -m)"
case "$ARCH" in
  arm64)  TARGET="darwin-aarch64"; AARCH="aarch64" ;;
  x86_64) TARGET="darwin-x64";     AARCH="x64"     ;;
  *) fail "不支持的架构：$ARCH" ;;
esac

note "安全扫描"
tracked=$(git -C "$REPO_ROOT" ls-files | grep -iE '\.p12$|\.pem$|updater\.key' || true)
[ -z "$tracked" ] || fail "版本库里出现了不该提交的文件：$tracked"
ok "版本库里没有证书 / 私钥"

note "读取版本号"
VERSION=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
PIWEB_VERSION=$(python3 -c "import json;print(json.load(open('sidecar.json'))['piWebVersion'])")
NODE_VERSION=$(python3 -c "import json;print(json.load(open('sidecar/manifest.json'))['nodeVersionActual'])")
[ -n "$VERSION" ] || fail "读不到壳版本"
ok "壳 $VERSION / pi-web $PIWEB_VERSION / node $NODE_VERSION"

APP="$BUNDLE_DIR/macos/Pi Web Desktop.app"
if [ ! -d "$APP" ]; then
  note "构建 .app（ad-hoc 整包签名）"
  # 关键：APPLE_SIGNING_IDENTITY="-" 让 tauri 用 ad-hoc 身份把整个 bundle（含嵌套二进制、
  # 资源封印）完整签名。不设的话 Tauri 只会留下 linker-signed 的主二进制，资源封印缺失，
  # 下载到本机后 Gatekeeper 会报「已损坏，无法打开」。
  ( cd "$REPO_ROOT" && APPLE_SIGNING_IDENTITY="-" ./node_modules/.bin/tauri build --bundles app )     || fail "tauri build 失败"
fi
ok "产物：$APP"

note "校验产物"
signature=$(codesign -dv --verbose=2 "$APP" 2>&1 | sed -n 's/^Signature=//p')
[ "$signature" = "adhoc" ] || fail "签名是「$signature」，应为 adhoc（发布产物不要用本机证书）"
ok "签名 adhoc"

# 只有 linker-signed 主二进制、没有资源封印的 bundle 会让下载者看到「已损坏，无法打开」，
# 必须严格校验整包（这是踩过的坑，别省）。
codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | grep -q "satisfies its Designated Requirement" \
  || { codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | tail -3; fail "整包签名封印不完整（Gatekeeper 会报「已损坏」）"; }
ok "整包签名封印完整（--verify --deep --strict）"

built_arch=$(lipo -archs "$APP/Contents/MacOS/pi-web-desktop")
[ "$built_arch" = "arm64" ] || fail "架构是 $built_arch，应为 arm64"
ok "架构 $built_arch"

built_version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")
[ "$built_version" = "$VERSION" ] || fail "产物版本 $built_version 与 tauri.conf.json 的 $VERSION 不一致"
ok "版本 $built_version"

if codesign -d --entitlements - "$APP" 2>/dev/null | grep -q "get-task-allow"; then
  fail "产物带着调试用 entitlement（get-task-allow），不该分发"
fi
ok "没有调试 entitlement"

# Rust 依赖的 panic 位置字符串会带 .cargo/.rustup 工具链路径，这是正常的；
# 要拦的是"构建者的项目路径 / 用户数据路径"被嵌进产物。
leaked=$(strings -a "$APP/Contents/MacOS/pi-web-desktop" 2>/dev/null \
  | grep -o "$HOME_DIR/[A-Za-z0-9._-]*" \
  | grep -v -E "^$HOME_DIR/\.(cargo|rustup)" \
  | sort -u || true)
if [ -n "$leaked" ]; then
  echo "$leaked" >&2
  fail "产物里出现了构建者的项目/用户路径，它会随下载包一起公开"
fi
ok "产物里没有构建者的项目/用户路径（只剩 .cargo/.rustup 工具链路径）"

[ -x "$APP/Contents/Resources/pi-web/runtime/node" ] || fail "侧车 runtime/node 不可执行"
[ -x "$APP/Contents/Resources/pi-web/app/node_modules/node-pty/prebuilds/darwin-arm64/spawn-helper" ] \
  || fail "node-pty spawn-helper 不可执行（终端功能会静默失效）"
ok "侧车可执行位正确"

note "打包"
rm -rf "$DIST"
mkdir -p "$DIST/staging"
ditto "$APP" "$DIST/staging/Pi Web Desktop.app"
ln -s /Applications "$DIST/staging/Applications"

DMG="$DIST/Pi-Web-Desktop-$VERSION-$AARCH.dmg"
ZIP="$DIST/Pi-Web-Desktop-$VERSION-$AARCH.zip"
ditto -c -k --keepParent "$APP" "$ZIP"
ok "zip  $(basename "$ZIP")"

DMG_LOG="$DIST/dmg.log"
# macOS 27 的 diskutil 新语法（hdiutil 已废弃）；CI/旧系统上 diskutil 还不支持 --volumeName，
# 就退回 hdiutil。两条路都在日志里留痕。
if diskutil image create from --help 2>&1 | grep -q "volumeName"; then
  diskutil image create from --format UDZO --volumeName "Pi Web Desktop" \
    "$DIST/staging" "$DMG" > "$DMG_LOG" 2>&1
else
  hdiutil create -volname "Pi Web Desktop" -srcfolder "$DIST/staging" \
    -ov -format UDZO "$DMG" > "$DMG_LOG" 2>&1
fi || { cat "$DMG_LOG" >&2; fail "创建 dmg 失败（需要能挂载磁盘映像的环境，受限沙箱里不行）"; }
ok "dmg  $(basename "$DMG")"
rm -rf "$DIST/staging" "$DMG_LOG"

cp "$ZIP" "$DIST/Pi-Web-Desktop-$AARCH.zip"
cp "$DMG" "$DIST/Pi-Web-Desktop-$AARCH.dmg"
ok "固定文件名别名（下载按钮指向 releases/latest/download/）"

# L3（只换 pi-web 的热更新）已于 2026-11-04 取消，不再产出侧车包与清单。
# 见 docs/PLAN-2026-11-04-drop-l3.md

if [ -f "$BUNDLE_DIR/macos/Pi Web Desktop.app.tar.gz" ]; then
  cp "$BUNDLE_DIR/macos/Pi Web Desktop.app.tar.gz" "$DIST/Pi-Web-Desktop-$VERSION-$AARCH.app.tar.gz"
  if [ -f "$BUNDLE_DIR/macos/Pi Web Desktop.app.tar.gz.sig" ]; then
    cp "$BUNDLE_DIR/macos/Pi Web Desktop.app.tar.gz.sig" "$DIST/Pi-Web-Desktop-$VERSION-$AARCH.app.tar.gz.sig"
  fi
  ok "updater 包"
else
  echo "  - 没有 updater 包（构建时未设置 TAURI_SIGNING_PRIVATE_KEY，跳过）"
fi

( cd "$DIST" && shasum -a 256 ./*.dmg ./*.zip ./*.tar.gz > SHA256SUMS 2>/dev/null ) || true
ok "SHA256SUMS"

note "完成，产物在 dist/"
ls -lh "$DIST"
echo
cat "$DIST/SHA256SUMS"
echo
echo "下一步（发版）："
echo "  git tag v$VERSION && git push origin v$VERSION   # CI 会自动构建并发布 Release"
