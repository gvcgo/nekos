-- 0dy10.com paid subscription plugin (https://www.0dy10.com)
--
-- The flow is independent of the core: login -> activate subscription ->
-- fetch the clash link. The produced subscription text is still parsed and
-- stored by nekos' core process (same parse path as a plain URL fetch).
--
-- Config (0dy10.json next to this file, editable from the UI "Plugin config"):
--   origin             site root, default https://www.0dy10.com (a local
--                      mock server can be pointed at for testing)
--   link               subscription link, default is the built-in one; swap
--                      the token part when moving to another account
--   email / passwd / code   login form fields; code is the captcha, leave empty if none
--   uid                uid cookie of an existing session: skips the login form
--
-- Refreshes are scheduled by nekos itself (Settings -> update interval), so
-- this plugin does not declare a schedule of its own.
-- The Lua API is provided by nekos: http.request / json / base64 / config / log / sleep.

local plugin = {}

plugin.name = "0dy10"
plugin.description = "0dy10.com paid subscription: login -> activate -> fetch clash link"

local DEFAULT_ORIGIN = "https://www.0dy10.com"
local DEFAULT_LINK = "https://dy.0dy10.com/link/NvFDfKhfFi7xUEnn?clash=2"

-- The site API requires a browser UA; the subscription link only answers
-- clash-like UAs (browser UA / curl / empty UA are rejected by nginx with
-- 504), and that request carries no cookies at all.
local BROWSER_UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36"
local CLASH_UA = "clash-verge/2.5.2"

-- Form percent-encoding (@ in the email, specials in the password).
local function urlencode(s)
  return (tostring(s):gsub("[^%w%-%._~]", function(c)
    return string.format("%%%02X", string.byte(c))
  end))
end

-- Minimal cookie jar: cookies set by login are sent to the following calls.
-- No expiry/domain matching needed (single site, single run).
local function new_jar()
  local jar = {}
  return {
    seed = function(name, value)
      jar[name] = value
    end,
    add = function(resp)
      for _, raw in ipairs(resp.cookies or {}) do
        local name, value = raw:match("^%s*([^;=%s]+)=([^;]*)")
        if name then
          jar[name] = value
        end
      end
    end,
    header = function()
      local parts = {}
      for name, value in pairs(jar) do
        parts[#parts + 1] = name .. "=" .. value
      end
      table.sort(parts) -- pairs() is unordered: sort so requests are reproducible
      return table.concat(parts, "; ")
    end,
  }
end

-- Parse a JSON response; a redirect to the login page yields HTML instead.
local function decode_json(resp, what)
  local ok, data = pcall(json.decode, resp.body or "")
  if not ok or type(data) ~= "table" then
    if resp.status == 200 and (resp.body or ""):find("<html", 1, true) then
      error(what .. " failed: session expired (redirected to the login page), check email/passwd or uid")
    end
    error(string.format("%s failed: response is not JSON (HTTP %d): %s", what, resp.status, (resp.body or ""):sub(1, 200)))
  end
  return data
end

-- POST with an optional form body: sends the jar cookies and collects any
-- new ones. accept/xhr differ per endpoint (login is XHR + JSON, activate is
-- a plain */*), matching the reference implementation.
local function post(origin, jar, path, referer, body, accept, xhr)
  local headers = {
    ["Accept"] = accept,
    ["Accept-Language"] = "zh-CN,zh;q=0.9,en;q=0.8",
    ["Origin"] = origin,
    ["Referer"] = origin .. referer,
    ["User-Agent"] = BROWSER_UA,
  }
  if xhr then
    headers["X-Requested-With"] = "XMLHttpRequest"
  end
  if body then
    headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8"
  end
  local cookie = jar.header()
  if cookie ~= "" then
    headers["Cookie"] = cookie
  end

  local resp = http.request({
    url = origin .. path,
    method = "POST",
    headers = headers,
    body = body,
    timeout = 20,
  })
  jar.add(resp)
  return resp
end

function plugin.fetch(ctx)
  local cfg = ctx.config or {}
  local origin = (cfg.origin ~= nil and tostring(cfg.origin) ~= "") and tostring(cfg.origin) or DEFAULT_ORIGIN
  local link = (cfg.link ~= nil and tostring(cfg.link) ~= "") and tostring(cfg.link) or DEFAULT_LINK
  local jar = new_jar()

  local uid = cfg.uid and tostring(cfg.uid) or ""
  if uid ~= "" then
    jar.seed("uid", uid)
    log("using the stored uid cookie, skipping the login form")
  else
    local email = tostring(cfg.email or "")
    local passwd = tostring(cfg.passwd or "")
    if email == "" or passwd == "" then
      error("missing config: fill in email / passwd (or the uid of an existing session)")
    end
    local form = table.concat({
      "email=" .. urlencode(email),
      "passwd=" .. urlencode(passwd),
      "code=" .. urlencode(tostring(cfg.code or "")),
    }, "&")

    local resp = post(
      origin, jar, "/auth/login", "/auth/login", form,
      "application/json, text/javascript, */*; q=0.01", true
    )
    if resp.status ~= 200 then
      error(string.format("login endpoint returned HTTP %d: %s", resp.status, (resp.body or ""):sub(1, 200)))
    end
    local data = decode_json(resp, "login")
    -- A failed login is HTTP 200 too: the business result is ret/msg (the
    -- success value of ret is not documented upstream).
    if data.ret == 0 and data.msg and data.msg ~= "" then
      error("login failed: " .. tostring(data.msg))
    end
    log("login ok")
  end

  -- Activate the subscription (valid for 5 minutes); a failure here does not
  -- abort the fetch, the link endpoint reports "subscription expired".
  local act = post(origin, jar, "/user/activate_sub", "/user", nil, "*/*", false)
  local ok, adata = pcall(json.decode, act.body or "")
  if ok and type(adata) == "table" and adata.ret == 0 then
    log("activate subscription failed: " .. tostring(adata.msg or "unknown reason"))
  else
    log("activate subscription submitted")
  end

  local resp = http.request({
    url = link,
    headers = {
      ["Accept"] = "*/*",
      ["Accept-Language"] = "zh-CN,zh;q=0.9,en;q=0.8",
      ["User-Agent"] = CLASH_UA,
    },
    timeout = 30,
  })
  if resp.status < 200 or resp.status > 299 then
    error(string.format("subscription link returned HTTP %d: %s", resp.status, (resp.body or ""):sub(1, 512)))
  end
  if resp.body == nil or resp.body == "" then
    error("subscription link returned an empty body")
  end
  log(string.format("fetched subscription: %d bytes", #resp.body))
  return resp.body
end

return plugin
