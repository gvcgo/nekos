# nekos — 路线图

> 里程碑按依赖顺序排列；P0 是本仓库第一阶段交付（对应架构文档 §2 全链路打通）。
> 用户已确认：Vue3+TS / Linux 优先 / Go 控制进程内嵌上游 sing-box。MVP（未勾选）按 v2rayN 最小闭环默认执行，可随时增删。

## P0 — 垂直闭环（当前阶段）
目标：输入「3 条测试 URI/任意订阅链接」，产出「可用代理客户端」的最小闭环。

- [x] 架构与决策文档（docs/architecture.md）
- [ ] Core: go module 引入 sing-box v1.14.0（库模式）
- [ ] Core: parser——anytls/trojan 分享链接 → NodeSpec（含用户测试 URI 单测）
- [ ] Core: parser 骨架覆盖 vmess/vless/ss/hysteria2/tuic（先行单一协议族冒烟后补全）
- [ ] Core: builder——NodeSpec+session → sing-box option（socks/mixed 入站 + 选中节点出站 + 最小 DNS/路由）
- [ ] Core: runtime——进程内 box.New/Start/Close；CLI `parse/config/run` 子命令
- [ ] Core: JSON-RPC v0（parse.text/core.start/stop/status/url_test）+ SSE 事件
- [ ] Core: 冒烟——经 SOCKS 走 anytls 节点真实出站连通（curl 验证）
- [ ] 编排层: 拉起/守护 core 子进程 + RPC client + 配置/DB 布局
- [ ] 编排层: SQLite（groups/nodes/settings）schema + 迁移
- [ ] 编排层: 分享链接/订阅导入命令（剪贴板/URL/文件）
- [ ] 平台层: Linux GSettings 系统代理开关（GNOME）+ 降级提示
- [ ] UI: 服务页（分组树 + 节点表 + 右键菜单：测速/选中/删除/导出）+ 开关按钮
- [ ] UI: 订阅导入对话框（URL/文本/文件/剪贴板）
- [ ] UI: 设置页（监听端口、当前模式、系统代理、日志查看）
- [ ] 托盘最小化 + 退出即清理（恢复系统代理/停 core）
- [ ] 端到端验收：导入 3 条 URI → 测速 → 选中 → 系统代理 → 直连验证

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
