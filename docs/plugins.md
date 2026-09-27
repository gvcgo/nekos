# nekos 订阅 Lua 插件写法

> 事实源：本文描述契约；实现见 `src-tauri/src/plugins.rs`，架构背景见
> [architecture.md §11](architecture.md)。

## 0. 什么时候用插件

URL 订阅（订阅页「URL 订阅」）只能发一个 GET。需要**登录 → 激活 → 取 token 链接**、
签名参数、分页拼装、多接口组合这类流程时，写插件：插件负责“拿到订阅文本”，
nekos 负责解析入库（与 URL 订阅共用同一条解析链路）。

## 1. 目录与加载

```
$HOME/.config/nekos/subs/           # Windows: %USERPROFILE%\.config\nekos\subs\
├── 0dy10.lua                       # 插件：一个 .lua 文件一个插件
└── 0dy10.json                      # 可选：同名配置（凭据/链接等）
```

- 只扫描目录**直接子级**的 `*.lua`（不递归）；文件名不能含 `/`、`\`。
- 首次运行 nekos 时会创建该目录并写入示例插件 `0dy10.lua`；目录已存在则不覆盖任何文件。
- 打开订阅页「＋ 抓取新订阅」会读取插件列表。列表会**执行每个插件的顶层代码**来读取
  `name`/`description`，因此顶层只放定义，不要在那里发网络请求或做耗时计算。
- 顶层报错不会让插件消失：列表里该插件带 ⚠ 与错误原因，点「⟳ 重新扫描插件目录」可重新加载。
- 环境变量 `NEKOS_SUBS_DIR` 可覆盖该目录（调试/测试用）。

## 2. 文件契约

```lua
local plugin = {}

plugin.name = "example"                       -- 列表显示名（缺省用文件名）
plugin.description = "…"                      -- 列表里的说明（可省）

function plugin.fetch(ctx)                    -- 必填
  -- ctx = { name = "example", dir = "<插件目录>", config = { ... } }
  return "…"                                  -- 订阅文本，格式见 §3
end

return plugin                                 -- 必须 return 这个 table
```

## 3. fetch 的返回值

| 返回 | 含义 |
|---|---|
| `"<订阅文本>"` | 直接作为订阅内容（`content_type` 视为未知） |
| `{ body = "<订阅文本>", content_type = "text/plain" }` | 带 Content-Type；`body` 也接受写法 `content`；其它键忽略 |

订阅文本必须是 nekos/core 能解析的格式：**clash YAML**、**base64 编码的分享链接列表**、
**明文分享链接列表**（每行一条）。要求：

- 不能为空（空白也算空）；
- 大小 ≤ 16 MB；
- 解析结果 0 节点时，本次抓取视为失败并保留原有节点 —— 所以别在订阅失效时返回 HTML 错误页。

> 不要在插件里拼 sing-box 节点结构：节点解析与协议映射只在 core 里做，插件只产出订阅文本。

## 4. API 参考

引擎提供 6 个全局：`http` / `json` / `base64` / `config` / `log` / `sleep`；
Lua 5.4 标准库亦可用（`string`/`table`/`math`/`utf8`/`io`/`os`/`coroutine`/`package`），
但 **`debug` 与 `ffi` 不提供**。

### http.request(opts) → resp

```lua
local resp = http.request({
  url     = "https://example.com/api/sub",   -- 必填
  method  = "GET",                           -- 默认 GET（GET/POST/PUT/PATCH/DELETE/HEAD…）
  headers = { ["User-Agent"] = "clash-verge/2.5.2", ["Authorization"] = "Bearer …" },
  body    = nil,                             -- 字符串；nil 表示无请求体
  timeout = 30,                              -- 秒，默认 30，范围 1–600
})

-- 返回值：
--   resp.status   整数 HTTP 状态码（4xx/5xx 不报错，自己判断）
--   resp.body     字符串（非 UTF-8 字节按替换字符损失处理）
--   resp.headers  表：头名全部小写；同名多值用 "\n" 连接
--   resp.cookies  数组：本响应所有 Set-Cookie 的原始值（含属性），1 起下标
```

行为与限制：

- **直连**：不继承 `http_proxy` 等环境代理（与 URL 订阅一致）。
- 自动跟随重定向，最多 10 跳；跟随后的最终页面的头/体返回给你。
- 响应体上限 16 MB；超限报错。请求体无上限（别塞超大 body）。
- **网络层失败（DNS、连接被拒、超时）会 `error()`**：要自行降级就 `pcall` 包住。
- 状态码不是错误：`响应 404` 会正常返回，由插件决定怎么处理（建议在非 2xx 时报错，把响应体前若干字节带进错误信息，便于排查）。
- `timeout` 是单次请求上限，整个插件运行另有 120 秒总预算（见 §8）。

### json

```lua
local obj = json.decode('{"ret":1,"list":[1,2]}')   -- 对象/数组 → 表；JSON null → nil
local s   = json.encode({ a = 1, list = { "x" } })  -- 表 → JSON；raw_len()>0 的表按数组编码
```

注意 JSON 的 `null` 在 Lua 里是 `nil`，字段会“消失”；`json.encode` 遇到函数/线程等不可编码值写成 `null`。

### base64

```lua
local s = base64.encode("hello")     -- 标准字母表，带 "=" 填充
local b = base64.decode(s)           -- 容忍空白，也接受 URL-safe 字符（- _）
```

### config

插件的持久化设置，落在同目录 `<stem>.json`（JSON 对象）。

```lua
config.get("token")                  -- 取一个键
config.get("timeout", 30)            -- 缺省值
config.all()                         -- 整份配置（表）
config.set("token", "abc")           -- 写入并落盘（原子替换，Unix 下 0600）
config.set("obsolete", nil)          -- 删除该键
```

- 文件缺失或不是 JSON 对象时视为 `{}`。
- `ctx.config` 是本次运行开始时的快照；用 `config.get` 读的是同一份，`config.set` 会立即落盘。
- 值支持字符串/数字/布尔/表（数组或对象）。

### log / sleep

```lua
log("login ok", 200)     -- 参数用空格连接；表等复杂值显示为 "table"
sleep(1.5)               -- 秒，实际生效范围 0–30（限流用）
```

`log` 的记录会随本次抓取结果返回，在 UI 的「插件日志」里展示；插件执行失败时，错误信息末尾
也会附上已收集的日志（`plugin logs:` 段），这是最主要的排错手段。

## 5. 典型模式：带 Cookie 的登录流程

`http.request` 不维护会话，插件自己拿 `resp.cookies` 即可（示例取自 `plugins/0dy10.lua`）：

```lua
-- Minimal cookie jar: keep name=value from Set-Cookie, send them back later.
local jar = {}
local function jar_add(resp)
  for _, raw in ipairs(resp.cookies) do
    local name, value = raw:match("^%s*([^;=%s]+)=([^;]*)")   -- strip Path/HttpOnly/…
    if name then jar[name] = value end
  end
end
local function jar_header()
  local parts = {}
  for k, v in pairs(jar) do parts[#parts + 1] = k .. "=" .. v end
  table.sort(parts)                                            -- pairs() is unordered
  return table.concat(parts, "; ")
end

local login = http.request({
  url = "https://example.com/auth/login",
  method = "POST",
  headers = {
    ["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8",
    ["User-Agent"] = "Mozilla/5.0 …",
  },
  body = "email=" .. urlencode(email) .. "&passwd=" .. urlencode(passwd),
  timeout = 20,
})
jar_add(login)
-- 后续接口带上 Cookie
local sub = http.request({ url = link, headers = { ["Cookie"] = jar_header() } })
```

要点：表单参数要 percent 编码；很多站点“失败也返回 HTTP 200”，需同时判断业务字段；
会话失效常表现为被 302 到登录页（响应体变成 HTML），据此给出可读错误。

## 6. 配置与凭据

- 配置文件：`<stem>.json`，JSON 对象，例如：

  ```json
  { "email": "you@example.com", "passwd": "…", "code": "" }
  ```

- 在 UI 里编辑：订阅页 →「＋ 抓取新订阅」→「Lua 插件」→ 选插件 →「插件配置 (JSON)」文本框 →「保存配置」。
- 文件含凭据，Unix 下落盘权限 0600。
- 也可以让插件自己 `config.set` 缓存（例如站点下发的长期 token）。

## 7. 运行、预览与定时更新

- **试跑（不落库）**：选插件 →「运行插件」→ 显示解析到的节点数/错误数；失败时显示原因 + 插件日志。
- **保存为订阅组**：试跑成功后点「保存为新订阅组」，该组会绑定插件文件；列表里显示
  `插件 lua:<file>`。之后点该组的「更新」＝重新运行插件（不是 URL 抓取），节点按内容哈希去重替换。
- **定时更新**：插件**不自带调度**。插件组与 URL 组一样，按设置页的「自动更新 + 更新间隔」
  （`settings.auto_update_minutes`，下限 5 分钟）在后台刷新；后台刷新与手动刷新是同一段代码。
- 组内节点/流量展示：插件无法上报 `subscription-userinfo`，因此插件组不显示流量配额（保留上一次 URL 抓取留下的值，若有）。

## 8. 限制与安全边界

| 项 | 值/行为 |
|---|---|
| 单次运行墙钟上限 | 120 秒（Lua hook 每 5 万条指令检查一次，`while true` 会被中止） |
| 单次 HTTP 超时 | 默认 30 秒，可传 1–600 秒 |
| 响应体上限 | 16 MB（单次响应） |
| 重定向 | ≤10 跳 |
| 沙箱 | **无**：插件与 GUI 同权限（可读写文件、`os.execute` 等）——只放你自己信任的插件 |
| 代理 | 不继承环境代理，直连 |

## 9. 排错

| 现象（引擎报错为英文） | 原因/处理 |
|---|---|
| `invalid plugin file name: …` | 不是 `*.lua`，或名字里含 `/`、`\` |
| `plugin not found: …` | 文件不在插件目录（或在子目录里——不支持） |
| `plugin X failed to load: …` | 顶层语法/运行错误；错误后附 `plugin logs:` |
| `plugin X must return a config table (got number)` | 忘了 `return plugin` |
| `fetch of plugin X must be a function (got nil)` | 没定义 `plugin.fetch` |
| `fetch of plugin X must return a string or { body = ... } (got table)` | 返回的表里没有 `body`/`content` |
| `plugin X returned an empty body` | 返回了空串/空白串 |
| `plugin X returned N MB, over the 16 MB limit` | 体量超限 |
| `plugin exceeded the 120s deadline and was aborted` | 死循环或总时长超预算 |
| `request <url> failed: …` | 网络层失败（DNS/连接/超时）；`pcall` 可自行降级 |
| `json.decode failed: …` / `base64.decode: invalid character …` | 数据格式不符 |
| 抓取成功但“解析 0 节点” | 返回的不是订阅文本（例如错误页 HTML）；看 core 返回的解析错误 |

## 10. 参考实现

- `plugins/0dy10.lua`：0dy10.com 付费订阅（表单登录 → 激活订阅 → 拉取 clash 链接，
  自持 Cookie Jar、可复用 uid、UA 约束），逐段注释说明。
- 最小骨架：

  ```lua
  local plugin = { name = "example", description = "token-protected endpoint" }

  function plugin.fetch(ctx)
    local token = config.get("token")
    if not token or token == "" then
      error('missing config: set {"token": "..."}')
    end
    local resp = http.request({
      url = "https://example.com/api/sub",
      headers = { ["Authorization"] = "Bearer " .. token, ["User-Agent"] = "nekos/0.1" },
      timeout = 20,
    })
    if resp.status ~= 200 then
      error(string.format("HTTP %d: %s", resp.status, resp.body:sub(1, 200)))
    end
    log(string.format("fetched %d bytes", #resp.body))
    return { body = resp.body, content_type = resp.headers["content-type"] }
  end

  return plugin
  ```
