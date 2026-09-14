# LLENS（屏幕日记）

Tauri 2 + Vue 3 + ECharts 的 macOS 屏幕活动记录与 AI 洞察工具。每 20 秒截屏，
调用 agnes-3.0-flash 生成 5 句总结；数据三层归档（帧 / 10 分钟 / 日记）；
液态玻璃 UI，含概览 / 日记 / 看板 / 问答 / 设置 五个页签。

## 目录结构

```
src-tauri/src/
  lib.rs        # Tauri 入口 + 命令注册 + 启动采集循环
  tstate.rs     # 全局状态、数据根目录、启动采集循环
  store.rs      # Config / 三层数据写入、10 分钟切片、范围读取
  capture.rs    # 20s 采集循环：screencapture → LLM → 小时 JSON → 10 分钟边界
  diary.rs      # 每日日记生成 + pending 天自动补跑
  search.rs     # 本地关键词检索（truncation 到 200 字/条）
  qasearch.rs   # agentic 问答编排：plan → search → refine → answer（最多 3 轮）
  llm.rs        # OpenAI 兼容 chat 客户端（文本 + 图像消息、重试）
  api.rs        # Tauri command 面（前端 invoke）
src/
  App.vue       # 主框架 + 页签
  views/        # Overview / Diary / Board / Qa / Settings
  style.css     # 液态玻璃主题
  assets/       # （背景图见 public/lens_bg.jpg）
public/
  lens_bg.jpg   # 液态玻璃背景图
scripts/
  icon_1024.png # 自定义应用图标源图（光圈+秒针）
  lens.icns     # 由 iconutil 生成的 .icns
  pack.sh       # 一键打包 + 安装 + 桌面快捷方式
```

## 运行

```bash
npm install
npm run dev        # 开发模式（Tauri 窗口 + Vite HMR）
npm run tauri build  # 生成 .app / .dmg
./scripts/pack.sh    # 打包 + 安装到 ~/Applications + 替换图标 + 桌面符号链接
```

## 数据目录

默认 `~/.screenlog`（可在 Tauri 配置 / AppState 中改）：

```
screenshots/YYYY-MM/HHMMSS.png   # 原始截图
logs/YYYY-MM/YYYY-MM-DD_HH.json  # 小时帧日志（summary5/activity/app/project）
summaries/YYYY-MM/YYYY-MM-DD_HH_10min.json  # 10 分钟片段（narrative + 占比 + 分类）
diary/YYYY-MM-DD.json            # 每日日记（brief/top3/highlights/todos/advice/tip）
config.json                      # API 配置（key/base_url/model）
```

## 测试

```bash
cd src-tauri
cargo test --test unit
```

## 注意

- 首次截屏需授予「屏幕录制」权限（系统设置 → 隐私与安全 → 屏幕录制）。
- API key 默认已写入 `config.json`；若需更换请在「设置」页操作，勿硬编码进源码。
