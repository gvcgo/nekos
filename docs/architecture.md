# nekos — 架构与决策记录

> 面向「以 Tauri 为 UI、以 sing-box 为内核、功能对标 v2rayN（暂不支持 xray-core）的跨平台桌面客户端」。
> 本文是唯一事实源（single source of truth）；代码演进必须先改本文。

## 0. 状态与结论速览

| 项 | 决策 | 依据 |
|---|---|---|
| UI 壳 | Tauri v2（Rust 后端 + WebView） | 用户选择；webkit2gtk-4.1 已具备 |
| 前端 | Vue 3 + TypeScript + Vite | 用户选择 |
| 内核集成 | **Go 控制进程内嵌 sing-box 为库**（nekobox 同款），非 CLI sidecar | 用户选择；理由见 §3 |
| sing-box 版本 | **v1.14.0**（最新稳定），Go module 直接依赖 | 2026-09 已核实：v1.15.0-alpha.2 在途 |
| 平台 | Linux 优先（本机可端到端验证）→ Windows/macOS | 用户选择 |
| MVP 闭环 | 导入(链接/订阅) → 节点列表管理 → 选节点启动 → 本机入站 + 系统代理 → 测速 | 用户未指定，取 v2rayN 最小可用子集 |
| 数据 | SQLite；节点存**归一化 NodeSpec**，运行时由 core 转 sing-box option | §5 |
| 控制面 | JSON-RPC 2.0 over 127.0.0.1 随机端口 + token；SSE 事件推送 | §6 |
| 许可证 | 本项目 GPL-3.0 | sing-box/nekobox/v2rayN 皆 GPL；Go 静态链接使本仓库整体受 GPL 约束（§9） |
| Linux GUI 抗上游崩溃 | 启动时（webview 之前）强制 `WEBKIT_FORCE_VBLANK_TIMER=1` | WebKitGTK 2.52.6 DisplayLink 除零 → 显示器热插拔时 GUI 被 SIGFPE 打死（实测两次）；§8.1 |

## 1. 目标与非目标

目标：
- 与 v2rayN 功能面一致：多订阅分组、节点管理/导入导出、快速开关、系统代理、TUN、分流模式、测速、路由规则、日志统计、托盘与自启。
- 协议解析与配置生成永远与**最新 sing-box** 对齐（新协议如 anytls/xhttp 一发布即可导入）。
- 进程级隔离，允许 TUN 提权只重启内核进程（Windows UAC / Linux pkexec），GUI 全程不持特权。

非目标（当前）：
- 不支持 xray-core / v2ray-core。
- 不内置 GUI 无法触达的协议私货（nieru/snell/juicity 等上游未并入的协议不做；除非用户明确要求）。

## 2. 总体分层

```
┌────────────────────────────────────────────────────────────┐
│  UI 层  Vue3+TS (src/)                                      │
│  页面: 服务(节点/分组列表) | 订阅 | 路由规则 | 设置 | 日志统计 │
│  状态: Pinia；调用: @tauri-apps/api → invoke(commands)      │
├────────────────────────────────────────────────────────────┤
│  编排层  Rust (src-tauri)                                   │
│  · 持久化: SQLite(rusqlite) —— 分组/节点/订阅/设置/日志       │
│  · CoreClient: 拉起/守护/退出 core 子进程；JSON-RPC + 事件    │
│  · platform: SystemProxy/TUN/自启/托盘 的平台抽象            │
│  · HTTP: 订阅抓取、规则集/geo 资源下载                        │
├────────────────────────────────────────────────────────────┤
│  控制进程  core/ (Go, 内嵌 sing-box v1.14)                   │
│  · parser: 分享链接/订阅文本 → 归一化 NodeSpec               │
│  · builder: NodeSpec + 会话参数 → sing-box option 全量配置    │
│  · runtime: box.New/启停/URL 测试/状态事件                   │
│  · RPC server: JSON-RPC 2.0 + SSE（loopback, token 鉴权）    │
└────────────────────────────────────────────────────────────┘
```

依赖方向唯一：UI → 编排层 → core。core 不感知 UI 与存储；编排层是唯一同时理解两者的一方。

## 3. 为什么是「Go 控制进程内嵌 sing-box」而不是其它形态

对照事实：
- **nekobox(NyameBox)**：Qt GUI + 独立 `nekobox_core`（Go，内嵌 sing-box 分支），GUI↔core 走本地 RPC；TUN 提权 = 单独重启 core 进程。这是参考基准。
- **v2rayN**：GUI 生成配置，把 xray/sing-box 当外部进程启停。
- sing-box 官方自证可内嵌：`github.com/sagernet/sing-box` 的 `box` 包 `box.New(box.Options{…})`；官方 Android 客户端走 `experimental/libbox`。

取舍：
1. **配置生成的 ground truth 是 sing-box 的 Go `option` 类型**。把解析与组装放在 Go 侧，新协议/新字段随 sing-box 发版零漂移；若放 Rust 手工产 JSON，每个版本都要追 schema。
2. 进程级隔离（非 cgo 打进程内）：TUN 提权、core 崩溃自愈、内存/流量上限都可控；与 Rust 之间只有一条薄 RPC。
3. 不依赖 sing-box **实验性** Clash API 做控制面（它只在本项目需要 selector 切换/延迟测试的补充场景启用，见 §6.3）。
4. 弃选「官方 sing-box CLI 二进制 sidecar」：切节点要重写配置+重启进程、测速依赖实验 API、状态回传弱；弃选 cgo 直嵌 Rust：GC/线程/提权模型复杂。

采用上游 `github.com/sagernet/sing-box`（非 nekobox 分支），保证"sing-box 最新功能"语义纯净。

## 4. 数据模型（编排层 SQLite）

分组（对应 v2rayN 的"分组+订阅"）：
- `groups(id, name, sub_url, sub_userinfo?, type, sort_order, enabled)`
- 内置 All 组（id=1，无订阅，不可删改）：作为跨分组合并视图列出所有普通分组的节点
  （行仍带真实 `group_id`，单节点操作按所属分组执行）；向 All 粘贴导入的节点落在此组名下。

节点（导入/解析产物，归一化，见 §5）：
- `nodes(id, group_id, tag, type, spec_json /*NodeSpec*/, remark, created_at, updated_at)`
- `tag` 在组内唯一，作为 sing-box outbound tag 的原料；显示名(remark)与 tag 分离。

其它：`settings(k,v)`；`log(id, ts, level, module, msg)`；`rulesets(id, url, format, local_path, updated_at)`（P2 引入）。

## 5. NodeSpec —— 与 sing-box 同构的节点中间格式

**决策（2026-09-06 修订）**：NodeSpec 不是"手维护的中立子集"，而是**一条 sing-box outbound option 的完整 JSON**（`{"type": "...", …全部字段}`），加两个编排层字段：

```json
{
  "id": "6461aa467fb1dda0",      // 内容哈希，去重用
  "remark": "🇸🇬 A新加坡1",       // 展示名（来自链接 fragment）
  "out": {
    "type": "anytls",
    "server": "ew.ali66mysql.com",
    "server_port": 26019,
    "password": "e0c664d9-…",
    "tls": { "enabled": true, "server_name": "www.apple.com", "insecure": true }
  }
}
```

理由：schema 就是 sing-box 官方 `option` 类型本身（core 解析时由
`UnmarshalExtendedContext` 校验），新协议/新字段随 sing-box 发版**零漂移**；
builder 无需维护映射表。tag 由编排层按 entry.id 派生（`out-<id>`），组内唯一。

- 分享链接/订阅文本只进 core（parser）；Rust 侧只认该 JSON。
- parser 范围（anytls/trojan/vless 已落地 + 单测，2026-09-06）：
  anytls://、trojan://、vless://；P1 补 vmess/ss/ssr/hysteria2/tuic/wireguard 等；
  订阅文本支持 base64 整体解码、逐行、url= 包裹。
- **ws early-data 实测结论**：v2rayN 风格 `path=/?ed=2048` 必须**原样保留在 path**（URL 查询式 ed），
  实测 sing-box 的 `max_early_data` header 式会被 v2rayN 系服务端拒绝（EOF）；见
  core/internal/link/transport.go 注释。

### 5.1 路由档案（rule 模式自定义分流，2026-09-07）

同 §5 哲学：路由规则也是 sing-box 原生 JSON，Rust 只存不改。`settings.route_profile_id`
指向自定义档案，缺省/删除回退**内置绕过大陆**（零迁移）。

- 落库：`route_profiles(id, name, final_out, rules_json, updated_at)`；`rules_json` 为有序
  sing-box route rule 对象数组，`final_out` 用语义名 proxy|direct|block。
- 契约：session 增 `route`（`{final, rules[]}`）；core 组装时把语义出口改写为具体 tag
  （proxy→`out-<selected>`），`rule_set` 仅允许内置 `geoip-cn`/`geosite-cn`（资产路径仍由
  编排层经 `rule_assets` 下发），未知出口/规则集在启动时 fail-fast。
- 规则命中引用 `geosite-cn` 时按内置语义补 DNS 块（CN 域名本地解析）；否则不引入 DNS 块。
- 首期边界：无远程规则集订阅/DNS-Object/嗅探/进程级规则（见 roadmap P1/P2）。

### 5.2 会话记忆（自动恢复上次节点，2026-09-12）

v2rayN 同语义：退出时记住"仍在使用的节点"，下次打开自动用它启动。

- 落库（`settings` JSON）：`last_group_id` + `last_node_id`（上次 `core.start` 实际运行的会话组与选中行；
  auto 策略组记为其宿主分组 + `strat:<gid>` 伪节点）、`resume_on_launch`（退出时内核是否在运行）。
  与 `selected_by_group`（手动选中，可先于启动变更）分离：后者是"下次启动的候选"，前者才是"在用的"。
- 启动恢复：`resume_last_session` 在 `setup` 内异步拉起——内核在运行中退出 → 自动恢复；用户点过「停止」→
  不动（停了就是停了）。记住的分组已不存在（订阅分组被删）→ 回退当前视图分组。
- 节点回退链（`build_session` → `resolve_selection`）：记住的节点仍存在 → 用它；已删除 / 被订阅刷新换掉 /
  被「过滤 IPv6」挡掉 → 同组**最快实测节点**（`latency.delay_ms` 最小；失败与未测不参与）；全组都没有实测 → 首行。
  策略组按成员分组的实测行聚合；`strat:<gid>` 引用失效（策略组被删或成员清空）→ 回退宿主分组自身节点。
- 与 UI 的契约：启动成功后把解析结果写回 `selected_by_group`（分组视图据此高亮当前行）；
  `core.status` 附带运行中的 `group_id`/`node_id`，前端在启动自动恢复后补上"正在使用"标记并轮询状态。

## 6. 控制面协议（编排层 ↔ core）

### 6.1 通道
- core 由编排层拉起，参数：`--rpc <listen>`（编排层传 127.0.0.1:0 自选）或 `--rpc-stdio`；同时下发 `--token <random>`、`--log <file>`。
- 消息：**JSON-RPC 2.0**（请求/响应 + 通知），传输 TCP loopback（Windows 提权后 core 重启仍可回连编排层转发的地址）。
- 事件：`GET /events` 长连接（SSE 简化版：`event: <name>\ndata: <json>\n\n`）。
- 鉴权：每个请求头 `Authorization: Bearer <token>`；token 仅经进程参数/stdin 传递，Rust 侧随机生成。

### 6.2 方法面（v0 已落地 2026-09-07）
```
parse.text        {text} → {nodes: NodeSpec[], errors: []}          // ✅ 链接/订阅混合文本
config.dry        {session} → {config}                              // ✅ 预演生成 sing-box 配置(调试用)
core.start        {session} → {status}                              // ✅ 组装配置并 box.New + 启动；运行中调用 = 进程内重建(切节点)
core.stop         {} → {status}                                     // ✅
core.status       {} → {running, started_at, core}                  // ✅
core.url_test     {entries, url?, timeout_s?, run_id?} → {results}   // ✅ 批量延迟(独立探测实例；sing-box URLTest 计时与 v2rayN 同语义；瞬态失败自动重试；>24 节点并发 10)
core.url_test_progress {run_id} → {found, done, results[]}           // ✅ 批测进度轮询：节点测完即返回，UI 逐行点亮
encode / qr       {remark, out[, size]} → link / base64 png         // ✅ 分享链接/二维码（不再逐次 fork）
core.select       {tag}                                             // 热切换（见 §6.3，P1+）
core.stats        {since} → {up, down}                              // 连接级流量(与 clash api 二选一)，P2
core.rulesets.*   （P2: 规则集下载/加载在编排层做，core 只认本地 srs 文件路径）
core.log          {level, lines}                                    // 现状：编排层读 daemon stderr；SSE /events 已推 core.started/core.stopped
```
`session` 载荷（编排层→core）：`{mode: rule|global|direct|strategy, selected_tag, inbounds: {socks_port?, http_port?, mixed_port?, tun?}, dns: {…}, ruleset_paths: []}`。core 内部由 session+节点库（NodeSpec 列表随 start 一并传，或按 tag 引用先前 parse 结果）组装完整 option。

### 6.3 切节点与测速的两种实现，按里程碑演进
- **现状（2026-09-07 落地）**：编排层拉起**长驻 `nekos-core serve`**（`--rpc 127.0.0.1:0 --token <random>`，stdout 首行回 `{"rpc": "127.0.0.1:PORT"}` 完成就绪握手；§6.1），此后 parse/启停/测速/编码全走 JSON-RPC（token Bearer）。切节点 = `core.start`（运行中调用 → 进程内 `Manager.Replace`：先校验新配置、新 box 启动失败自动回滚上一 session，旧实例在配置非法时不受影响）。批量测速 = `core.url_test`（独立探测实例与运行实例共存）。CLI `run`/`urltest`/`parse` 保留为调试/单测通道，不再是 UI 主路径。
- P1+ 增强：见下（selector 热切换 / loopback clash api）。
- P0/P1 兜底：**重建实例**——更新 session、停旧 `box`、启新 `box`（毫秒级；v2rayN 亦是整核重启）。简单、零内部 API 依赖。
- P1+ 增强：尝试 sing-box 已导出的 selector/urltest 运行时切换（若上游提供对外控制入口，如 clashapi handler 同款逻辑）；**若不可行则启用 loopback-only 的 `experimental.clash_api`**（sing-box 原生支持 selector 切换、`/proxies/{name}/delay`、`/connections` 统计）作为运行时控制面，此时 core 自建的 core.select/core.url_test 变薄代理。两条路都保留，不把宝押在内部未导出 API 上。

## 7. 分流与规则（对照 v2rayN 的三种模式）

- **绕过大陆(rule)**：直连国内。P0 用 sing-box 本地 `rule_set`(`type: local, format: source, inline_rules: …`)内嵌精简直连表（国内 IP 段/顶级域名）+ DNS 分流；P2 升级为可订阅的规则集（geoip/geosite srs，q3 参考 nekobox ruleset 仓库）。
- **全局(global)**：`final: <选中节点>`。
- **规则(自定义)**：规则编辑器产出 sing-box route rules → NodeSpec 无关的规则配置持久化在 Rust，随 session 下发。
DNS：P0 采用 sing-box 默认（host 解析走代理出口）加国内域名直连查询的最小 fakeip 关闭方案；P1 给 v2rayN 风格 DNS 配置 UI。

## 8. 平台适配层（Rust trait `Platform`）

| 能力 | Linux(P0) | Windows(P1+) | macOS(P1+) |
|---|---|---|---|
| 系统代理 | GSettings `org.gnome.system.proxy`(+KDE 探测，暂降级文档) | 注册表 `HKCU\...\Internet Settings` | `networksetup -setwebproxy…` |
| TUN 提权 | pkexec 拉起 core(root) + tun 设备 | 提权重启 core(UAC)，wintun | core 提权(sysexits/launchd 提示) |
| 开机自启 | XDG autostart .desktop | 注册表 Run / 计划任务 | LaunchAgent |
| 托盘 | libappindicator (tauri-plugin) | 内置 | 内置 |

「关闭最小化到托盘」「开机不自启直到用户开启」等细节同 v2rayN。

### 8.1 Linux 桌面壳：WebKitGTK DisplayLink 除零崩溃与守卫（2026-09-11）

现象与证据（coredump + journal，两次复现，非 Rust panic）：
- GUI 进程被内核以 `SIGFPE` 打死（`traps: nekos[...] trap divide error ip:... libwebkit2gtk-4.1.so.0`），用户观感是"莫名其妙自动退出"；core 子进程随后靠 stdin EOF 退出，系统代理残留由下次启动的 `cleanup_stale_cores` + `leftover_at` 收拾。
- 根因（源码级）：`Source/WebKit/UIProcess/glib/DisplayLinkGLib.cpp:64`
  `m_fpsThrottleRatio = refreshRate / m_displayNominalFramesPerSecond;` 无零检查；
  `refreshRate` 取自 `gdk_monitor_get_refresh_rate()/1000`（Wayland 下输出重连产生的新 `GdkMonitor`
  在 mode 落地前返回 0 = unknown）→ `0/0`。
- 触发（实测）：显示器/连接器重新枚举。两次 crash 均在 niri `connecting connector: DP-6` 之后 ~20ms；
  本机 DP-6 是 Realtek "UHD HDR demoset" 演示板（EDID 物理尺寸 0×0），会自行拉 HPD。
- 上游状态：webkitgtk-2.52.6 与 `main` 分支同样未加防护 → 只能在本层规避。

守卫：`run()` 起始处（创建 webview 之前）设 `WEBKIT_FORCE_VBLANK_TIMER=1`（仅当环境里未显式设置）。
该开关让 WebKit 走 `DisplayVBlankMonitorTimer`，其 refreshRate 是编译期常量
`WebCore::FullSpeedFramesPerSecond = 60` → `60/60`，结构性不可能除零（与显示器状态无关）。
代价：UI 进程 vblank 由 DRM 真实垂直同步改为 60Hz 定时线程 —— WebKit 自身在 DRM vblank 不可用时的
兜底路径（displayID 0 / 无 CRTC 匹配 / WPE），高刷屏上 UI 刷新回调封顶 60fps。
退出条件：Arch 的 webkit2gtk 修掉该除零后删除守卫；调试时可用 `WEBKIT_FORCE_VBLANK_TIMER=0`
显式退回原行为做对照。

## 9. 安全与合规
- RPC 只绑 127.0.0.1 + token；配置/日志落盘 0600；日志脱敏（password/token 打码）。
- 订阅抓取走编排层（可配 UA/自定义头），内容只送 parser。
- **许可证**：本项目按 GPL-3.0 发布（Go 静态链接 sing-box 后整体受其许可证约束；参考实现 nekobox 亦 GPL-3.0）。复用参考仓库代码前须核对各自版权头；本文档只借架构思想，不直接搬运代码。

## 10. 关键外部事实（含核实时间，2026-09-06）
- sing-box 最新稳定 v1.14.0；tags 显示 v1.15.0-alpha.2 已存在 → 升级节奏：每个稳定版发版后一周内 bump。
- nekobox(qr243vbi) = NyameBox：Qt/C++ GUI + nekobox_core(Go)；FAQ 证实"TUN 模式 UAC 重启 nekobox_core"进程模型；TODO 承认其存储/UI 建模仍在重构 → 我们不必照抄其内部。
- v2rayN 结构：`ServiceLib`(Models/Handler/Services/Manager/Helper/Enums/ViewModels) 纯逻辑层 + `v2rayN.Desktop` UI —— 分层思想借鉴：逻辑与壳分离。
