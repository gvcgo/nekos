# nekos — 路线图

> 里程碑按依赖顺序排列；P0 是本仓库第一阶段交付（对应架构文档 §2 全链路打通）。
> 用户已确认：Vue3+TS / Linux 优先 / Go 控制进程内嵌上游 sing-box。MVP（未勾选）按 v2rayN 最小闭环默认执行，可随时增删。

## P0 — 垂直闭环（当前阶段）
目标：输入「3 条测试 URI/任意订阅链接」，产出「可用代理客户端」的最小闭环。

- [x] 架构与决策文档（docs/architecture.md）
- [x] Core: go module 引入 sing-box（库模式：`box.New/Start/Close` + `option.Options` 类型校验 + `include.Context`，无外部 CLI sidecar）[go.mod 依赖 + build/run/urltest 库调用即落地]；版本 v1.14.0 引入，2026-09-26 bump 至 **v1.14.2**（回归：真订阅 204 节点解析/测速 + 双协议 204 通过）
- [x] Core: parser——anytls/trojan/vless 分享链接 → sing-box outbound JSON（含用户测试 URI 单测；ws `ed` 保留在 path 的实测映射）
- [ ] Core: parser 扩展 vmess/ss/ssr/hysteria2/tuic/wireguard/naive 等（复用 transport/registry 骨架）
- [x] Core: builder——session → sing-box option（mixed/socks 入站 + 选中节点出站 + direct/block）
- [x] Core: runtime——进程内 box.New/Start/Close（include.Context 引导）；CLI `parse/config/run/test/version`
- [x] Core: 冒烟——4 节点（anytls×2/vless/trojan）经内嵌 sing-box 真实 HTTPS 204 全链路连通（337–1117ms，2026-09-06）
- [x] Core: JSON-RPC v0——长驻控制面落地：`serve`（127.0.0.1:0 + token Bearer + SSE）；parse/启停/状态/批量测速/编码全走 RPC；切节点 = 进程内重建（Manager.Replace，坏配置保留旧实例）；CLI `run`/`urltest`/`parse` 保留作调试通道 [2026-09-07，见 architecture §6.2/§6.3]
- [x] 编排层: core 子进程生命周期（启动就绪判定/停止，Rust 集成测试）
- [x] 编排层: SQLite（groups/nodes/settings）schema + 迁移 + CRUD（rusqlite bundled）
- [x] 编排层: 分享链接/订阅文本导入入库命令 + 订阅 URL 抓取（自定义 UA/headers、直连）
- [x] Clash 订阅 proxies 解析（anytls/ss/vmess/vless/trojan/hy2/tuic/wg；真订阅 22 节点实测）
- [x] 平台层: Linux GSettings 系统代理开关（GNOME，已验证 enable/restore）
- [x] UI: 服务页（分组下拉 + 节点表 + 测速/选中/删除 + 启动停止 + 模式 + 代理开关）
- [x] UI: 订阅页（URL/headers 抓取预览 + 保存为新分组）
- [x] UI: 设置页（端口/模式/系统代理/托盘项）
- [x] 托盘最小化 + 关闭进托盘 + 退出清理（恢复系统代理/停 core）
- [x] 端到端验收：DB 预置 4 节点 + Clash 22 节点 → 截图确认节点表/分组/状态渲染；启动链路由集成测试覆盖
- [x] 路由设置（rule 模式自定义分流）：设置页路由卡片 + 档案/规则编辑器；SQLite `route_profiles` +
      session `route`（语义出口 proxy/direct/block）；规则集限内置 geoip/geosite-cn；go 单测 + 真 core
      冒烟（suffix/keyword 命中 → 502，放行 → 200）[2026-09-07]
  （余下 DNS 配置 UI 见 P1「分流三模式」，远程规则集订阅/图形编辑器见 P2）

## P1 — 体验补齐
- [x] 切节点热更新：已落地=进程内优雅重建——`core.start` 运行中调用即 `Manager.Replace`（毫秒级、可断、坏配置保留旧实例），UI 选中节点即重建切换；selector/clash-api 运行时切换未做（architecture §6.3 备选双轨）[2026-09-08]
- [x] 全节点批量测速 + 真连接测试（v2rayN 同款语义）：单 sing-box 实例并发测全组 + 进度逐节点流式（`latency:row`）+ 结果落库；单点测速同引擎；瞬态失败自动重试 [2026-09-08]
- [x] 多订阅分组管理：自动更新（后台定时 + 设置开关）、导入去重（内容哈希 upsert）、userinfo/到期展示已落地 [2026-09-08]
- [x] 订阅 Lua 插件（`$HOME/.config/nekos/subs/*.lua`，mlua/Lua5.4）：插件产出订阅文本 → 仍由 core 解析入库；
      Rust 提供 http/json/base64/config/log 原语；插件组与 URL 组统一按设置页「更新间隔」后台定时抓取（插件不自带调度）；
      示例插件 `plugins/0dy10.lua` 覆盖付费订阅（登录 → 激活订阅 → 拉取 clash 链接，Cookie/Jar 自持）。
      验证：引擎单测 + mock 站点端到端跑真插件 + 真 core 解析 + UI 桩 IPC 交互 [2026-09-27，见 architecture §11]
- [ ] 分流三模式（绕过大陆/全局/规则）+ DNS 配置 UI；绕过大陆用内嵌精简直连表起步
  （global/rule/direct 三模式与自定义分流已落地，见 P0「路由设置」；DNS 配置 UI 未做）
- [x] 记住上次使用的节点：退出时的会话（组 + 选中行）与"内核在运行"落库，下次启动自动恢复该节点；节点已删除/换掉
      → 同组最快实测节点（无实测则首行）；「停止」视为显式关闭，下次启动不自动拉起。见 architecture §5.2 [2026-09-12]
- [ ] 节点编辑：可视化表单新增/修改 → 导出分享链接
- [x] 日志面板：实时尾部查看（daemon stderr 环形缓冲 + `log_tail` 轮询 + 级别分类/ANSI 剥离 + 日志级别设置联动）已落地；余下 SSE 事件推送与落盘检索 [2026-09-08]

## P2 — 高级网络
- [ ] TUN 模式：core 配置含 tun 入站；Linux pkexec 提权启动 core；Windows UAC（P4 落地）
- [ ] 规则集订阅：geoip/geosite/自定义 srs 下载与更新（编排层），路由规则图形编辑器
- [ ] 流量统计/连接列表（复用 core stats 或 loopback-only clash api）

## P3 — 桌面集成
- [ ] 托盘菜单增强（快速切节点/测速/模式）
- [x] 开机自启（XDG autostart）与关闭最小化到托盘（托盘化见 P0）：开机自启开关已落地（写入 `~/.config/autostart`，设置页即时生效，Wayland/X11 均可用）；全局热键不提供（Wayland 无法 X 抓键）[2026-09-08]
- [x] 托盘恢复后标题栏按钮失效（Linux/Wayland，tao#1046/#1299 = tauri#15460）：vendor tao 0.36.0 为 0.35.99 走 `[patch.crates-io]`，随 tauri ≥ 2.12（tao ^0.36）后移除 vendor/tao 与 patch [2026-09-08]
- [x] WebKitGTK 显示器热插拔除零崩溃（`DisplayLinkGLib.cpp:64`，Wayland 输出重连时 `gdk_monitor_get_refresh_rate()=0` → `0/0` → SIGFPE 打死 GUI）：`run()` 内守卫 `WEBKIT_FORCE_VBLANK_TIMER=1` 改走 60fps 定时 vblank；上游 2.52.6/main 均未修，随 Arch webkit2gtk 升包移除 [2026-09-11，见 architecture §8.1]
- [ ] 通知（订阅更新失败/断线提醒）

## P4 — 平台与发布
- [ ] Windows 适配：系统代理（HKCU WinINET 注册表 + InternetSetOption 广播，代码已落地 [2026-09-08]）；自启（HKCU Run）已落地；余 wintun、UAC 提权重启 core、NSIS/MSI（需 Windows 真机验证）
- [x] macOS 适配（真机验证 2026-09-26，macOS 26.7/x86_64，见 architecture §8.2）：系统代理（networksetup
      逐服务 web/secureweb/socks + 读回校验；仅权限类失败才整批单次 osascript 提权；横幅行误判服务与逐命令弹窗已修）、
      自启（LaunchAgent + Aqua 会话 + launchctl bootstrap/bootout）、打包 .app 的 sidecar 解析（exe 同目录优先）、
      Dock 点击恢复窗口（RunEvent::Reopen）；构建/测试/打包三链路真机通过。余 TUN 提权提示（TUN 整体未做，P2）
- [ ] 打包与发布：tauri bundler（deb/rpm/AppImage → NSIS → dmg）+ GitHub Actions 矩阵 + core 交叉编译 sidecar 布局已备好：pack.sh（core sidecar 交叉编译 + tauri bundle，打包时经 --config 临时开启 bundle.active）；macOS 通用包 pack-macos.sh（x86_64+arm64：Go sidecar lipo + `tauri build --target universal-apple-darwin`，产物双架构 lipo 断言）[2026-09-26]；图标集 icns/ico 就绪；余 CI 矩阵与各 OS 真机构建验证
- [x] i18n（zh-CN/en）：组件内双语文案 + 设置项即时切换已落地 [2026-09-08]；主题未做
- [ ] 自动更新（tauri updater + core 二进制随版本）

## 追踪规则
- 每个稳定版 sing-box 发版后 1 周内 bump 并回归解析/构建。
- 任何里程碑开工前先更新 architecture.md 相关小节。
