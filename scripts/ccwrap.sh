"""
LLens 构建前置（macOS 无 Xcode License 授权场景）
====================================================
当 Xcode 未授权许可（sudo xcodebuild -license 未执行）时，Xcode 附带的
cc/clang 会被拒绝，Rust 编译 ring 的 C 依赖及最终链接会失败。

本脚本把 C/C++ 编译转发到 Command Line Tools 的 clang，并显式指定
CLT 的 macOS SDK，绕过 Xcode。

用法（二选一）：

  1. 全局一次性设置后正常构建：
       export CC=/Users/liu/bin/ccwrap.sh
       export CXX=/Users/liu/bin/ccwrap.sh
       export SDKROOT=$(ls -d /Library/Developer/CommandLineTools/SDKs/MacOSX*.sdk | tail -1)

  2. 单次运行（本脚本自动导出后执行）：
       ./ccwrap.sh cargo build          # 内部导出 CC/CXX/SDKROOT 再跑参数
       ./ccwrap.sh cargo test

判定是否需要：
  xcodebuild -version 2>&1 | grep -q "requires agreement" && echo "需要 ccwrap"
  xcrun --sdk macosx clang --version 2>&1 | grep -q "requires agreement" && echo "需要 ccwrap"

修复后（已授权 Xcode License）可不再使用本脚本，直接 cargo build 即可。
"""

SDKROOT_DIR=$(ls -d /Library/Developer/CommandLineTools/SDKs/MacOSX*.sdk 2>/dev/null | tail -1)
CLT_BIN=$(ls -d /Library/Developer/CommandLineTools/usr/bin/clang 2>/dev/null | head -1)

if [ -z "$SDKROOT_DIR" ] || [ -z "$CLT_BIN" ]; then
  echo "ccwrap: 未找到 CLT SDK 或 clang，请确认已安装 Command Line Tools" >&2
  exit 1
fi

export SDKROOT="$SDKROOT_DIR"
export CC="$CLT_BIN"
export CXX="$CLT_BIN"

echo "ccwrap: SDKROOT=$SDKROOT_DIR"
echo "ccwrap: 执行 $*"
exec "$@"
