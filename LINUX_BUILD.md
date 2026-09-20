# LLENS Linux 构建 & 部署指南

本文档说明如何构建、安装和运行 LLENS 的 Linux 版本。
Linux 目标平台通过 CI（GitHub Actions）或本地交叉构建产出，当前主开发平台仍是 macOS。

## 一、前置条件

Linux 构建/运行所需系统依赖：

```bash
# Debian / Ubuntu
sudo apt-get install -y \
  libwebkit2gtk-4.0-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  scrot
```

> 截屏使用 `scrot`（X11/Xorg 环境下）。
> 若使用 Wayland（GNOME 43+ / Sway 等），scrot 可能无法直接截屏，
> 可替换为 `maim`、`grim`（Wayland）或 `spectacle`（KDE）。

## 二、本地构建（需 Linux 环境）

在 Linux 机器上：

```bash
cd llens
npm install
npm run tauri:build
```

产物路径：

| 产物 | 路径 |
|---|---|
| 可执行文件 | `src-tauri/target/release/llens` |
| AppImage | `src-tauri/target/release/bundle/appimage/llens.AppImage` |
| DEB 包 | `src-tauri/target/release/bundle/deb/llens_*.deb` |

运行 AppImage：

```bash
chmod +x src-tauri/target/release/bundle/appimage/llens.AppImage
./src-tauri/target/release/bundle/appimage/llens.AppImage
```

## 三、通过 GitHub Actions 构建（推荐）

无需本地 Linux 环境。推送 `v*` 开头的 tag，或手动触发 workflow：

```
GitHub 仓库 → Actions → build → Run workflow
```

完成后在 Run 详情页面下载三个 artifacts：

- `llens-macos`（llens.app + .dmg）
- `llens-windows`（llens.exe + NSIS 安装包）
- `llens-linux`（llens + .AppImage + .deb）

若推的是 `v*` tag，会自动创建 GitHub Release 并附带全部安装包。

## 四、数据与截屏逻辑（Linux 适配说明）

- **截屏命令**：Linux 使用 `scrot`，单屏模式（不支持多屏横向合成，多屏功能仅在 macOS 上完整生效）。
- **熄屏检测**：`display::main_display_asleep()` 在 Linux 上始终返回 `false`（无法可靠检测），因此 Linux 端不会产生"休息帧"。
- **开机自启**：Linux 使用 `tauri-plugin-autostart` 的 XDG autostart 机制（写入 `~/.config/autostart/*.desktop`）。
- **桌面快捷方式**：生成 `~/Desktop/llens.desktop` 并尝试标记为 trusted。

## 五、排错

| 现象 | 可能原因 | 解决 |
|---|---|---|
| 启动报错 `screencapture/scrot not found` | 未安装 scrot | `sudo apt install scrot` |
| 截屏失败，X11 环境 | 使用 Wayland 且 scrot 不支持 | 改用 maim/grim 并修改 `capture.rs` Linux 分支 |
| AppImage 打不开 | 缺少 FUSE | `sudo apt install fuse`，或改用 DEB 安装 |
| 界面无法显示 | WebKit 版本过低 | 升级 `libwebkit2gtk-4.0` 到 2.38+ |

## 六、当前限制

- 多屏截图合成（时间轴横向拼接）仅 macOS 完整支持；Linux 端为单屏简化逻辑。
- 屏幕录制权限检测与"打开系统权限设置"仅 macOS 生效（`screen_permission` / `open_screen_recording_settings`）。
- Linux 端未做代码签名/公证（AppImage + DEB 均为本地信任安装）。
