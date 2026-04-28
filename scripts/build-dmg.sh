#!/usr/bin/env bash
# 一条龙：svg → icns → cargo build → Ramag.app → Ramag.dmg
#
# 依赖（macOS 自带）：
#   - sips、iconutil（Xcode CLT 提供 iconutil；svg 转 png 用 sips）
#   - hdiutil（DMG 打包）
#   - lipo（Xcode CLT 提供，universal 合并用）
#   - cargo（项目自带 rust-toolchain.toml）
#
# 用法：
#   ./scripts/build-dmg.sh                     # release，当前架构（native）
#   ./scripts/build-dmg.sh --debug             # debug 二进制（更快编译，dmg 体积大）
#   ./scripts/build-dmg.sh --target=x86_64     # 交叉编译到 Intel mac
#   ./scripts/build-dmg.sh --target=arm64      # 交叉编译到 Apple Silicon
#   ./scripts/build-dmg.sh --target=universal  # Intel + Apple Silicon 通用二进制
#
# 产物（带架构后缀，避免互相覆盖）：
#   - native：    target/Ramag.app    / target/Ramag.dmg
#   - x86_64：    target/Ramag-x86_64.app    / target/Ramag-x86_64.dmg
#   - arm64：     target/Ramag-arm64.app     / target/Ramag-arm64.dmg
#   - universal： target/Ramag-universal.app / target/Ramag-universal.dmg
#
# 注意：release profile 是 lto=fat + codegen-units=1，单架构编译已经较慢，
#       universal 需要编两次再 lipo 合并，时间约 2 倍。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# === 参数解析 ========================================================
PROFILE="release"
TARGET="native"
for arg in "$@"; do
    case "$arg" in
        --debug)    PROFILE="debug" ;;
        --target=*) TARGET="${arg#--target=}" ;;
        -h|--help)
            sed -n '1,25p' "$0"
            exit 0
            ;;
        *)
            echo "❌ 未知参数：$arg"
            echo "   见 $0 --help"
            exit 1
            ;;
    esac
done

# 把 --target 标准化成 cargo target triple（"" 代表 native，universal 单独处理）
case "$TARGET" in
    native)
        TARGET_TRIPLE=""
        SUFFIX=""
        ;;
    x86_64|intel|x86_64-apple-darwin)
        TARGET_TRIPLE="x86_64-apple-darwin"
        SUFFIX="-x86_64"
        ;;
    arm64|aarch64|aarch64-apple-darwin)
        TARGET_TRIPLE="aarch64-apple-darwin"
        SUFFIX="-arm64"
        ;;
    universal)
        TARGET_TRIPLE="universal"
        SUFFIX="-universal"
        ;;
    *)
        echo "❌ 未知 --target=${TARGET}（支持：native / x86_64 / arm64 / universal）"
        exit 1
        ;;
esac

if [[ "$PROFILE" == "release" ]]; then
    CARGO_FLAGS="--release"
    PROFILE_DIR="release"
else
    CARGO_FLAGS=""
    PROFILE_DIR="debug"
fi

# === 路径 ============================================================
ICON_DIR="$SCRIPT_DIR/icons"
SVG="$ICON_DIR/ramag.svg"
ICONSET="$ICON_DIR/ramag.iconset"
ICNS="$ICON_DIR/ramag.icns"

APP="$REPO_DIR/target/Ramag${SUFFIX}.app"
DMG="$REPO_DIR/target/Ramag${SUFFIX}.dmg"
STAGING="$REPO_DIR/target/dmg-staging${SUFFIX}"

# === 依赖检查 ========================================================
NEED_CMDS=(sips iconutil hdiutil cargo)
if [[ "$TARGET_TRIPLE" == "universal" ]]; then
    NEED_CMDS+=(lipo)
fi
for cmd in "${NEED_CMDS[@]}"; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "❌ 缺命令：$cmd"
        case "$cmd" in
            iconutil|lipo) echo "   Xcode CLT 提供，运行 xcode-select --install" ;;
            cargo)         echo "   先安装 Rust：https://rustup.rs" ;;
        esac
        exit 1
    fi
done

if [[ ! -f "$SVG" ]]; then
    echo "❌ 找不到 SVG 源：$SVG"
    exit 1
fi

# 确认 rustup target 已安装；缺失则 rustup target add（按 rust-toolchain.toml 当前 toolchain）
ensure_target_installed() {
    local triple="$1"
    if ! rustup target list --installed 2>/dev/null | grep -qx "$triple"; then
        echo "▶ rustup target 未安装：${triple}，自动安装中 ..."
        rustup target add "$triple"
    fi
}

case "$TARGET_TRIPLE" in
    "")
        : # native，不需要额外 target
        ;;
    universal)
        ensure_target_installed "x86_64-apple-darwin"
        ensure_target_installed "aarch64-apple-darwin"
        ;;
    *)
        ensure_target_installed "$TARGET_TRIPLE"
        ;;
esac

# === 1) svg → icns（若 svg 比 icns 新或 icns 不存在）==================
if [[ ! -f "$ICNS" || "$SVG" -nt "$ICNS" ]]; then
    echo "▶ 1/4 svg → icns ..."
    rm -rf "$ICONSET"
    mkdir -p "$ICONSET"

    # Apple iconset 标准尺寸：16/32/64/128/256/512/1024，含 @2x
    sips -s format png -Z 16   "$SVG" --out "$ICONSET/icon_16x16.png"      >/dev/null
    sips -s format png -Z 32   "$SVG" --out "$ICONSET/icon_16x16@2x.png"   >/dev/null
    sips -s format png -Z 32   "$SVG" --out "$ICONSET/icon_32x32.png"      >/dev/null
    sips -s format png -Z 64   "$SVG" --out "$ICONSET/icon_32x32@2x.png"   >/dev/null
    sips -s format png -Z 128  "$SVG" --out "$ICONSET/icon_128x128.png"    >/dev/null
    sips -s format png -Z 256  "$SVG" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
    sips -s format png -Z 256  "$SVG" --out "$ICONSET/icon_256x256.png"    >/dev/null
    sips -s format png -Z 512  "$SVG" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
    sips -s format png -Z 512  "$SVG" --out "$ICONSET/icon_512x512.png"    >/dev/null
    sips -s format png -Z 1024 "$SVG" --out "$ICONSET/icon_512x512@2x.png" >/dev/null

    iconutil -c icns "$ICONSET" -o "$ICNS"
    rm -rf "$ICONSET"
else
    echo "▶ 1/4 icns 已是最新，跳过"
fi

# === 2) cargo build ==================================================
cd "$REPO_DIR"

# 单架构 build：$1 = triple；输出 binary 路径到 stdout
build_one_triple() {
    local triple="$1"
    echo "▶ cargo build $CARGO_FLAGS --target=$triple -p ramag-bin ..." >&2
    # shellcheck disable=SC2086
    cargo build $CARGO_FLAGS --target="$triple" -p ramag-bin
    echo "$REPO_DIR/target/$triple/$PROFILE_DIR/ramag"
}

if [[ -z "$TARGET_TRIPLE" ]]; then
    # native：不带 --target，产物在 target/$PROFILE_DIR/
    echo "▶ 2/4 cargo build $CARGO_FLAGS -p ramag-bin (native) ..."
    # shellcheck disable=SC2086
    cargo build $CARGO_FLAGS -p ramag-bin
    BIN_PATH="$REPO_DIR/target/$PROFILE_DIR/ramag"
elif [[ "$TARGET_TRIPLE" == "universal" ]]; then
    echo "▶ 2/4 cargo build (Universal: x86_64 + arm64) ..."
    BIN_X86="$(build_one_triple "x86_64-apple-darwin")"
    BIN_ARM="$(build_one_triple "aarch64-apple-darwin")"

    UNI_DIR="$REPO_DIR/target/universal-apple-darwin/$PROFILE_DIR"
    mkdir -p "$UNI_DIR"
    BIN_PATH="$UNI_DIR/ramag"

    echo "▶ lipo -create 合并 universal binary ..."
    lipo -create -output "$BIN_PATH" "$BIN_X86" "$BIN_ARM"
    lipo -info "$BIN_PATH"
else
    echo "▶ 2/4 cargo build $CARGO_FLAGS --target=$TARGET_TRIPLE -p ramag-bin ..."
    BIN_PATH="$(build_one_triple "$TARGET_TRIPLE")"
fi

# === 3) 组装 Ramag.app ===============================================
echo "▶ 3/4 组装 $(basename "$APP") ..."
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"

cp "$BIN_PATH" "$APP/Contents/MacOS/Ramag"
cp "$ICNS" "$APP/Contents/Resources/ramag.icns"

cat > "$APP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Ramag</string>
    <key>CFBundleDisplayName</key>
    <string>Ramag</string>
    <key>CFBundleIdentifier</key>
    <string>com.axemc.ramag</string>
    <key>CFBundleVersion</key>
    <string>0.0.1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.0.1</string>
    <key>CFBundleExecutable</key>
    <string>Ramag</string>
    <key>CFBundleIconFile</key>
    <string>ramag</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
</dict>
</plist>
EOF

# 让 LaunchServices 重新注册，避免 dock 图标缓存
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "$LSREGISTER" ]]; then
    "$LSREGISTER" -f "$APP" 2>/dev/null || true
fi
touch "$APP" 2>/dev/null || true

# === 4) 打包成 DMG ===================================================
echo "▶ 4/4 hdiutil 打包 $(basename "$DMG") ..."
rm -rf "$STAGING"
mkdir -p "$STAGING"
cp -R "$APP" "$STAGING/$(basename "$APP")"
ln -s /Applications "$STAGING/Applications"
rm -f "$DMG"

hdiutil create \
    -volname "Ramag" \
    -srcfolder "$STAGING" \
    -fs HFS+ \
    -format UDZO \
    -imagekey zlib-level=9 \
    "$DMG" >/dev/null

rm -rf "$STAGING"

echo ""
echo "ok 已生成 DMG："
ls -lh "$DMG"
if [[ -n "$TARGET_TRIPLE" && "$TARGET_TRIPLE" != "universal" ]]; then
    echo "架构：${TARGET_TRIPLE}（在目标 mac 上首次运行可能被 Gatekeeper 拦截，可用 xattr -dr com.apple.quarantine /Applications/Ramag.app 解除）"
elif [[ "$TARGET_TRIPLE" == "universal" ]]; then
    echo "架构：universal（Intel + Apple Silicon 双切片，体积约为单架构两倍）"
fi
echo ""
echo "测试：open $DMG"
echo "（挂载后把 $(basename "$APP") 拖到 Applications 即可安装）"
