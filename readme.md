# nekos

以 [Tauri v2](https://v2.tauri.app) 为 UI、[sing-box](https://github.com/sagernet/sing-box) 为内核的跨平台代理客户端，功能对标 [v2rayN](https://github.com/2dust/v2rayN)（暂不支持 xray-core），内核集成方式参考 [NekoBox/NyameBox](https://github.com/qr243vbi/nekobox)（独立 Go 控制进程内嵌 sing-box 为库）。

> 文档：
> - [架构与决策记录](docs/architecture.md) — 事实源
> - [路线图](docs/roadmap.md) — 里程碑与追踪
> - [订阅 Lua 插件写法](docs/plugins.md) — 插件契约 / API / 排错

## 仓库布局

```
src/            Vue 3 + TS 前端（UI 层）
src-tauri/      Rust 编排层（存储/系统代理/托盘/core 进程管理/订阅 Lua 插件）
core/           Go 控制进程（parser / builder / runtime / JSON-RPC）
plugins/        内置订阅 Lua 插件示例（首次运行时写入 $HOME/.config/nekos/subs/）
docs/           架构与路线图
```

## 构建与打包

```bash
./build.sh            # 本机调试：构建 Go 内核 + `tauri dev`
./build.sh release    # 本机 release 二进制（src-tauri/target/release/nekos）
./build.sh core       # 只重建 Go 内核（core/bin/nekos-core）
./pack.sh             # 本机 OS 打包（core sidecar + tauri bundle：deb/rpm/AppImage | app+dmg | nsis）
./pack-macos.sh       # macOS 通用 dmg（x86_64 + arm64 单一 dmg；只编译并打印 dmg 路径）
./pack-arch.sh        # Arch Linux pacman 包（复用 pack.sh 的 deb 重封装）
```

## 状态

P0 进行中：见 [roadmap.md](docs/roadmap.md)。

## 许可证

GPL-3.0（内核 sing-box 为 GPL-3.0-or-later，Go 静态链接使本项目整体受 GPL 约束）。
