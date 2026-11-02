#!/usr/bin/env bash
#
# 创建一把本机自签名代码签名证书，放进一个独立的钥匙串。
#
# 移植自 zWGestures 的同名脚本（docs/PLAN 里记过原因），这里的目标是同一个：
#
#   1. ad-hoc 签名（codesign -s -）的 CDHash 每次重新编译都会变，macOS 会把每次构建都
#      当成一个全新的 App —— 对我们这个壳来说，表现为 TCC（文件访问）授权、以及「后台
#      项目」之类按 App 身份记的授权每次重建都要重新点。改用一把固定证书后，代码要求
#      （designated requirement）保持稳定，授权一次长期有效。
#   2. 私钥放登录钥匙串时，codesign 取私钥要经过 SecurityAgent 弹窗；在 `tauri build`
#      这种无人值守流程里弹窗不会被应答，签名会随机失败并报 errSecInternalComponent。
#      独立钥匙串的密码由我们掌握，可以设置 key partition list 让 codesign 免弹窗使用。
#
# 只影响当前用户，不需要管理员权限。
#
# 用法：
#   scripts/create-signing-cert.sh            # 创建/复用
#   APPLE_SIGNING_IDENTITY="Pi Web Desktop Local Signing" npm run build   # 用它签名
#
# 撤销：
#   security delete-keychain ~/Library/Keychains/pi-web-desktop.keychain-db
#   security list-keychains -d user -s ~/Library/Keychains/login.keychain-db
#
# 注意：发布产物（dist/）仍然用 ad-hoc 签名，见 scripts/make-dist.sh 头部说明。
set -euo pipefail

CERT_NAME="Pi Web Desktop Local Signing"
KEYCHAIN_PATH="${HOME}/Library/Keychains/pi-web-desktop.keychain-db"
KEYCHAIN_PASSWORD="piwebdesktop"
OPENSSL=/usr/bin/openssl

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
chmod 700 "$WORK"

note() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

note "1/5 准备独立钥匙串…"
if [ -f "$KEYCHAIN_PATH" ]; then
    echo "    已存在：${KEYCHAIN_PATH}"
else
    security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
    echo "    已创建：${KEYCHAIN_PATH}"
fi
security set-keychain-settings -l -u -t 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

EXISTING_CERT=0
if security find-certificate -c "$CERT_NAME" "$KEYCHAIN_PATH" >/dev/null 2>&1; then
    EXISTING_CERT=1
    echo "2/5 证书已存在，复用"
else
    echo "2/5 生成自签名证书与私钥…"
    "$OPENSSL" req -x509 -newkey rsa:2048 -nodes -sha256 -days 3650 \
        -keyout "$WORK/key.pem" -out "$WORK/cert.pem" \
        -subj "/CN=${CERT_NAME}/O=Pi Web Desktop/C=CN" \
        -addext "basicConstraints=critical,CA:false" \
        -addext "keyUsage=critical,digitalSignature" \
        -addext "extendedKeyUsage=critical,codeSigning"

    "$OPENSSL" pkcs12 -export -inkey "$WORK/key.pem" -in "$WORK/cert.pem" \
        -name "$CERT_NAME" -out "$WORK/piweb.p12" -passout pass:piwebdesktop
fi

echo "3/5 导入并设置 key partition list…"
if [ "$EXISTING_CERT" -eq 0 ]; then
    security import "$WORK/piweb.p12" -k "$KEYCHAIN_PATH" -P piwebdesktop \
        -T /usr/bin/codesign -T /usr/bin/security -A
fi
# 没有这一步，codesign 每次取私钥都要经过 SecurityAgent（无人值守时直接失败）
security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
    -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH" >/dev/null

echo "4/5 加入钥匙串搜索列表…"
CURRENT="$(security list-keychains -d user | sed -e 's/^[[:space:]]*//' -e 's/"//g')"
if printf '%s\n' "$CURRENT" | grep -qxF "$KEYCHAIN_PATH"; then
    echo "    已在搜索列表中"
else
    # shellcheck disable=SC2086
    security list-keychains -d user -s $CURRENT "$KEYCHAIN_PATH"
    echo "    已加入"
fi

echo "5/5 验证证书可用…"
if security find-identity -v -p codesigning "$KEYCHAIN_PATH" | grep -qF "$CERT_NAME"; then
    echo "    ✓ $(security find-identity -v -p codesigning "$KEYCHAIN_PATH" | grep -F "$CERT_NAME" | head -1 | sed 's/^ *//')"
else
    echo "    ✗ 找不到可用于 codesigning 的身份，检查钥匙串：$KEYCHAIN_PATH" >&2
    exit 1
fi

cat <<EOF

完成。日常构建想用这把证书（而不是 ad-hoc）：

  export APPLE_SIGNING_IDENTITY="$CERT_NAME"
  npm run build

发布产物（dist/）仍然 ad-hoc 签名，见 scripts/make-dist.sh。
EOF
