#!/usr/bin/env bash
# LLENS — one-click build + custom icon + desktop shortcut.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="屏幕日记"
BUNDLE_SRC="src-tauri/target/release/bundle/macos/llens.app"
DEST="$HOME/Applications/llens.app"
DESKTOP="$HOME/Desktop/${APP_NAME}.app"

echo "==> 1/4 生成 .app"
SDKROOT=$(xcrun --sdk macosx --show-sdk-path) npm run tauri build
[ -d "$BUNDLE_SRC" ] || { echo "missing $BUNDLE_SRC"; ls src-tauri/target/release/bundle/macos/; exit 1; }

echo "==> 2/4 生成图标 (.icns)"
ICONSET="scripts/icon.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
python3 - <<'PY'
from PIL import Image
src = Image.open("scripts/icon_1024.png")
sizes = {
    "icon_16x16.png": 16,
    "icon_16x16@2x.png": 32,
    "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64,
    "icon_128x128.png": 128,
    "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256,
    "icon_256x256@2x.png": 512,
    "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}
import os
os.makedirs("scripts/icon.iconset", exist_ok=True)
for name, s in sizes.items():
    src.resize((s, s), Image.LANCZOS).save(os.path.join("scripts/icon.iconset", name))
print("iconset ready")
PY
iconutil -c icns "$ICONSET" -o scripts/lens.icns

# replace bundle icon
cp scripts/lens.icns "src-tauri/icons/icon.icns"

echo "==> 3/4 安装到 ~/Applications + 替换图标"
rm -rf "$DEST"
mkdir -p "$HOME/Applications"
cp -R "$BUNDLE_SRC" "$DEST"
# overwrite icon inside the installed copy
cp scripts/lens.icns "$DEST/Contents/Resources/icon.icns"
touch "$DEST"  # force icon cache refresh

echo "==> 4/4 桌面快捷方式（符号链接）"
rm -f "$DESKTOP"
ln -s "$DEST" "$DESKTOP"
# 让 Finder 把快捷方式显示为应用图标（可选，失败不影响）
osascript -e "tell application \"Finder\" to reveal POSIX file \"$DESKTOP\"" >/dev/null 2>&1 || true

echo "✅ 完成。桌面已出现「${APP_NAME}.app」快捷方式，双击即可启动。"
echo "   首次运行若提示无法截屏，请到 系统设置 → 隐私与安全 → 屏幕录制 中勾选本应用。"
