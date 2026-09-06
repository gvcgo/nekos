# nekos — 路线图

> 里程碑按依赖顺序排列；P0 是本仓库第一阶段交付（对应架构文档 §2 全链路打通）。
> 用户已确认：Vue3+TS / Linux 优先 / Go 控制进程内嵌上游 sing-box。MVP（未勾选）按 v2rayN 最小闭环默认执行，可随时增删。

## P0 — 垂直闭环（当前阶段）
目标：输入「3 条测试 URI/任意订阅链接」，产出「可用代理客户端」的最小闭环。

- [x] 架构与决策文档（docs/architecture.md）
- [ ] Core: go module 引入 sing-box v1.14.0（库模式）
- [x] Core: parser——anytls/trojan/vless 分享链接 → sing-box outbound JSON（含用户测试 URI 单测；ws `ed` 保留在 path 的实测映射）
- [ ] Core: parser 扩展 vmess/ss/ssr/hysteria2/tuic/wireguard/naive 等（复用 transport/registry 骨架）
- [x] Core: builder——session → sing-box option（mixed/socks 入站 + 选中节点出站 + direct/block）
- [x] Core: runtime——进程内 box.New/Start/Close（include.Context 引导）；CLI `parse/config/run/test/version`
- [x] Core: 冒烟——4 节点（anytls×2/vless/trojan）经内嵌 sing-box 真实 HTTPS 204 全链路连通（337–1117ms，2026-09-06）
- [ ] Core: JSON-RPC v0（长驻控制面；当前阶段以 run-CLI 子进程重建实例先行，见 architecture §6.3）
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

## P1 — 体验补齐
- [ ] 切节点热更新：优先 selector 运行时切换；不可行则优雅重建（毫秒级，连接可断）
- [ ] 全节点批量测速 + 真连接测试（v2rayN 同款语义）
- [ ] 多订阅分组管理：自动更新、去重、userinfo/到期展示
- [ ] 分流三模式（绕过大陆/全局/规则）+ DNS 配置 UI；绕过大陆用内嵌精简直连表起步
- [ ] 节点编辑：可视化表单新增/修改 → 导出分享链接
- [ ] 日志面板（core 日志经事件推送 + 落盘检索）

## P2 — 高级网络
- [ ] TUN 模式：core 配置含 tun 入站；Linux pkexec 提权启动 core；Windows UAC（P4 落地）
- [ ] 规则集订阅：geoip/geosite/自定义 srs 下载与更新（编排层），路由规则图形编辑器
- [ ] 流量统计/连接列表（复用 core stats 或 loopback-only clash api）

## P3 — 桌面集成
- [ ] 托盘菜单增强（快速切节点/测速/模式）
- [ ] 开机自启（XDG autostart）、全局热键、关闭最小化到托盘
- [ ] 通知（订阅更新失败/断线提醒）

## P4 — 平台与发布
- [ ] Windows 适配：系统代理注册表、wintun、UAC 提权重启 core、NSIS/MSI
- [ ] macOS 适配：networksetup 系统代理、TUN 提权提示
- [ ] 打包与发布：tauri bundler（deb/rpm/AppImage → NSIS → dmg）+ GitHub Actions 矩阵 + core 交叉编译 sidecar 布局
- [ ] i18n（zh-CN/en）、主题
- [ ] 自动更新（tauri updater + core 二进制随版本）

## 追踪规则
- 每个稳定版 sing-box 发版后 1 周内 bump 并回归解析/构建。
- 任何里程碑开工前先更新 architecture.md 相关小节。
