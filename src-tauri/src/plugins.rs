//! Subscription Lua plugins (architecture.md §11).
//!
//! Plugins live in `$HOME/.config/nekos/subs/*.lua` and exist for
//! subscriptions that plain URL fetches cannot express (login + activate +
//! token link, signed requests, …). A plugin only *produces* the
//! subscription text; parsing stays in the core process, so the HTTP path
//! and the plugin path share one choke point.
//!
//! Script contract:
//!
//! ```lua
//! local plugin = { name = "0dy10", description = "..." }
//! function plugin.fetch(ctx)          -- ctx = { name, dir, config }
//!   return body                        -- string, or
//!   -- return { body = body, content_type = "text/plain" }
//! end
//! return plugin
//! ```
//!
//! Available primitives (all implemented here, so plugins need no network
//! stack of their own): `http.request`, `json.encode/decode`,
//! `base64.encode/decode`, `config.get/all/set`, `log(...)`, `sleep(secs)`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, Variadic, VmState};
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as Json};

/// File name of the bundled example plugin, written into the plugin dir the
/// first time it is created.
pub const EXAMPLE_FILE: &str = "0dy10.lua";
const EXAMPLE_PLUGIN: &str = include_str!("../../plugins/0dy10.lua");

/// Hard cap on a body a plugin returns or receives (mirrors the HTTP path).
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Wall-clock budget for one plugin run (top-level chunk + `fetch`).
pub const RUN_DEADLINE: Duration = Duration::from_secs(120);
/// Instructions between two timeout-hook callbacks.
const HOOK_INSTRUCTIONS: u32 = 50_000;
/// Per-request default timeout (seconds) when the plugin passes no `timeout`.
const REQUEST_TIMEOUT_SECS: f64 = 30.0;
/// Longest `sleep()` a plugin may request (seconds).
const MAX_SLEEP_SECS: f64 = 30.0;

// ---- plugin dir ---------------------------------------------------------

/// `$HOME/.config/nekos/subs` (`%USERPROFILE%` on Windows, `NEKOS_SUBS_DIR`
/// overrides — used by tests).
pub fn subs_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NEKOS_SUBS_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    for key in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(key).filter(|v| !v.is_empty()) {
            return PathBuf::from(home).join(".config").join("nekos").join("subs");
        }
    }
    PathBuf::from(".config/nekos/subs")
}

/// Create the plugin dir. A freshly created dir is seeded with the bundled
/// example plugin (a dir the user already owns is never touched).
pub fn ensure_dir() -> Result<PathBuf, String> {
    let dir = subs_dir();
    ensure_dir_in(&dir)?;
    Ok(dir)
}

/// Returns true when the dir was created (and therefore seeded).
fn ensure_dir_in(dir: &Path) -> Result<bool, String> {
    let fresh = !dir.is_dir();
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create plugin dir {}: {e}", dir.display()))?;
    if fresh {
        std::fs::write(dir.join(EXAMPLE_FILE), EXAMPLE_PLUGIN)
            .map_err(|e| format!("cannot write the example plugin {}: {e}", dir.join(EXAMPLE_FILE).display()))?;
    }
    Ok(fresh)
}

/// Resolve a plugin file name inside `dir`, rejecting anything that is not a
/// plain `*.lua` file name (no path separators → no traversal).
fn plugin_path(dir: &Path, file: &str) -> Result<PathBuf, String> {
    let name = file.trim();
    if name.is_empty()
        || !name.ends_with(".lua")
        || name.contains('/')
        || name.contains('\\')
        || name == "."
    {
        return Err(format!("invalid plugin file name: {file}"));
    }
    let path = dir.join(name);
    if !path.is_file() {
        return Err(format!("plugin not found: {}", path.display()));
    }
    Ok(path)
}

// ---- listing ------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct PluginInfo {
    /// File name inside the plugin dir, e.g. `0dy10.lua`.
    pub file: String,
    pub name: String,
    pub description: String,
    /// Load/parse error: the plugin is listed but cannot be run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Installed plugins, ordered by file name. Loading a plugin runs its
/// top-level chunk, so a plugin with a broken header shows up as an entry
/// carrying `error` instead of disappearing.
pub fn list_in(dir: &Path) -> Vec<PluginInfo> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".lua"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| match load(dir, file, RUN_DEADLINE) {
            Ok(loaded) => PluginInfo {
                file: file.clone(),
                name: loaded.meta.name,
                description: loaded.meta.description,
                error: None,
            },
            Err(e) => PluginInfo {
                file: file.clone(),
                name: file.trim_end_matches(".lua").to_string(),
                description: String::new(),
                error: Some(e),
            },
        })
        .collect()
}

pub fn list() -> Vec<PluginInfo> {
    if let Err(e) = ensure_dir() {
        eprintln!("plugins: {e}");
    }
    list_in(&subs_dir())
}

// ---- config file --------------------------------------------------------

/// `<stem>.json` next to the plugin holds its settings (credentials, links,
/// cached tokens). Missing/unparsable file → empty object.
fn config_path(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.json"))
}

fn read_config(path: &Path) -> JsonMap<String, Json> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Json>(&s).ok())
        .and_then(|v| match v {
            Json::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_config(path: &Path, map: &JsonMap<String, Json>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Json::Object(map.clone()))
        .map_err(|e| format!("cannot serialize plugin config: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{text}\n"))
        .map_err(|e| format!("cannot write plugin config {}: {e}", tmp.display()))?;
    // Credentials live here: keep the file private (architecture.md §9).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot save plugin config {}: {e}", path.display()))
}

/// Plugin config as pretty JSON text (`{}` when unset).
pub fn config_json(file: &str) -> Result<String, String> {
    let dir = subs_dir();
    let name = file.trim();
    if name.is_empty() || !name.ends_with(".lua") {
        return Err(format!("invalid plugin file name: {file}"));
    }
    let map = read_config(&config_path(&dir, name.trim_end_matches(".lua")));
    Ok(serde_json::to_string_pretty(&Json::Object(map)).unwrap_or_else(|_| "{}".into()))
}

/// Replace a plugin's config with `json` (must be a JSON object).
pub fn set_config_json(file: &str, json: &str) -> Result<String, String> {
    let dir = ensure_dir()?;
    // validate the file name without requiring the plugin to exist yet
    let name = file.trim();
    if name.is_empty() || !name.ends_with(".lua") || name.contains('/') || name.contains('\\') {
        return Err(format!("invalid plugin file name: {file}"));
    }
    let value: Json = if json.trim().is_empty() {
        Json::Object(JsonMap::new())
    } else {
        serde_json::from_str(json).map_err(|e| format!("plugin config is not valid JSON: {e}"))?
    };
    let Json::Object(map) = value else {
        return Err("plugin config must be a JSON object ({ \"key\": value })".into());
    };
    write_config(&config_path(&dir, name.trim_end_matches(".lua")), &map)?;
    Ok(serde_json::to_string_pretty(&Json::Object(map)).unwrap_or_else(|_| "{}".into()))
}

// ---- running ------------------------------------------------------------

#[derive(Clone)]
struct PluginMeta {
    name: String,
    description: String,
}

/// One run's shared state: log sink + config snapshot/file.
struct RunCtx {
    dir: PathBuf,
    stem: String,
    config_path: PathBuf,
    config: Mutex<JsonMap<String, Json>>,
    logs: Mutex<Vec<String>>,
}

impl RunCtx {
    fn log(&self, line: String) {
        self.logs.lock().push(line);
    }

    fn logs_snapshot(&self) -> Vec<String> {
        self.logs.lock().clone()
    }

    /// Append collected logs to an error message so the UI shows why a
    /// plugin failed (login rejected, UA blocked, …).
    fn describe(&self, msg: String) -> String {
        let logs = self.logs_snapshot();
        if logs.is_empty() {
            return msg;
        }
        format!("{msg}\nplugin logs:\n{}", logs.join("\n"))
    }
}

/// What a plugin run produced.
#[derive(Debug)]
pub struct PluginOutput {
    pub body: String,
    pub content_type: Option<String>,
    pub logs: Vec<String>,
}

struct Loaded {
    lua: Lua,
    table: Table,
    meta: PluginMeta,
    ctx: Arc<RunCtx>,
}

pub fn run(file: &str) -> Result<PluginOutput, String> {
    run_in(&subs_dir(), file, RUN_DEADLINE)
}

pub fn run_in(dir: &Path, file: &str, deadline: Duration) -> Result<PluginOutput, String> {
    let loaded = load(dir, file, deadline)?;
    let fetch: Function = match loaded
        .table
        .get::<Value>("fetch")
        .map_err(|e| format!("cannot read fetch of plugin {file}: {e}"))?
    {
        Value::Function(f) => f,
        other => {
            return Err(loaded.ctx.describe(format!(
                "fetch of plugin {file} must be a function (got {})",
                type_name(&other)
            )))
        }
    };

    let ctx_table = build_ctx_table(&loaded.lua, &loaded.ctx)
        .map_err(|e| format!("cannot build ctx for plugin {file}: {e}"))?;
    let value = fetch
        .call::<Value>(ctx_table)
        .map_err(|e| loaded.ctx.describe(format!("fetch of plugin {file} failed: {e}")))?;

    let (body, content_type) = match value {
        Value::String(s) => (s.to_string_lossy(), None),
        Value::Table(t) => {
            let body: String = t
                .get::<Option<String>>("body")
                .map_err(|e| loaded.ctx.describe(format!("plugin {file} returned an invalid body field: {e}")))?
                .or(t
                    .get::<Option<String>>("content")
                    .map_err(|e| loaded.ctx.describe(format!("plugin {file} returned an invalid content field: {e}")))?
                )
                .ok_or_else(|| {
                    loaded
                        .ctx
                        .describe(format!("plugin {file} returned a table without a body field"))
                })?;
            let ct: Option<String> = t.get("content_type").unwrap_or(None);
            (body, ct)
        }
        other => {
            return Err(loaded.ctx.describe(format!(
                "fetch of plugin {file} must return a string or {{ body = ... }} (got {})",
                type_name(&other)
            )))
        }
    };

    if body.len() > MAX_BODY_BYTES {
        return Err(loaded.ctx.describe(format!(
            "plugin {file} returned {:.1} MB, over the {} MB limit",
            body.len() as f64 / (1024.0 * 1024.0),
            MAX_BODY_BYTES / (1024 * 1024)
        )));
    }
    if body.trim().is_empty() {
        return Err(loaded.ctx.describe(format!("plugin {file} returned an empty body")));
    }

    Ok(PluginOutput {
        body,
        content_type,
        logs: loaded.ctx.logs_snapshot(),
    })
}

/// Build a fresh Lua state, register the primitives and evaluate the plugin
/// file. The returned value must be a table (the plugin module).
fn load(dir: &Path, file: &str, deadline: Duration) -> Result<Loaded, String> {
    let path = plugin_path(dir, file)?;
    let script = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read plugin {}: {e}", path.display()))?;
    let stem = file.trim().trim_end_matches(".lua").to_string();
    let cfg_path = config_path(dir, &stem);
    let ctx = Arc::new(RunCtx {
        dir: dir.to_path_buf(),
        stem: stem.clone(),
        config: Mutex::new(read_config(&cfg_path)),
        config_path: cfg_path,
        logs: Mutex::new(Vec::new()),
    });

    let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())
        .map_err(|e| format!("cannot create the Lua state: {e}"))?;
    // CPU-loop guard: a plugin cannot wedge the refresh worker (network
    // waits are covered by the per-request timeout instead).
    let deadline_at = Instant::now() + deadline;
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTIONS),
        move |_lua, _debug| {
            if Instant::now() >= deadline_at {
                Err(mlua::Error::runtime(format!(
                    "plugin exceeded the {}s deadline and was aborted",
                    deadline.as_secs()
                )))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
    register(&lua, &ctx).map_err(|e| format!("cannot register the plugin API: {e}"))?;

    let value = lua
        .load(&script)
        .set_name(format!("@{file}"))
        .eval::<Value>()
        .map_err(|e| ctx.describe(format!("plugin {file} failed to load: {e}")))?;
    let table = match value {
        Value::Table(t) => t,
        other => {
            return Err(ctx.describe(format!(
                "plugin {file} must return a config table (got {})",
                type_name(&other)
            )))
        }
    };

    let meta = PluginMeta {
        name: table
            .get::<Option<String>>("name")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| stem.clone()),
        description: table
            .get::<Option<String>>("description")
            .ok()
            .flatten()
            .unwrap_or_default(),
    };
    Ok(Loaded { lua, table, meta, ctx })
}

fn build_ctx_table(lua: &Lua, ctx: &RunCtx) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", ctx.stem.as_str())?;
    t.set("dir", ctx.dir.to_string_lossy().into_owned())?;
    let config = ctx.config.lock().clone();
    t.set("config", json_to_lua(lua, &Json::Object(config))?)?;
    Ok(t)
}

// ---- Lua primitives -----------------------------------------------------

fn register(lua: &Lua, ctx: &Arc<RunCtx>) -> mlua::Result<()> {
    let globals = lua.globals();
    globals.set("http", http_api(lua)?)?;
    globals.set("json", json_api(lua)?)?;
    globals.set("base64", base64_api(lua)?)?;
    globals.set("config", config_api(lua, ctx)?)?;
    globals.set(
        "log",
        lua.create_function({
            let ctx = ctx.clone();
            move |_, args: Variadic<Value>| {
                let line = args
                    .iter()
                    .map(value_to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                ctx.log(line);
                Ok(())
            }
        })?,
    )?;
    globals.set(
        "sleep",
        lua.create_function(|_, secs: f64| {
            let secs = if secs.is_finite() {
                secs.clamp(0.0, MAX_SLEEP_SECS)
            } else {
                0.0
            };
            if secs > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(secs));
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

/// `http.request{ url=…, method=…, headers={…}, body=…, timeout=… }`
/// → `{ status, headers = {lowercase name → value}, cookies = {…}, body }`.
/// Transport failures raise (wrap in `pcall` to handle them).
fn http_api(lua: &Lua) -> mlua::Result<Table> {
    let api = lua.create_table()?;
    let client = reqwest::blocking::Client::builder()
        // Same policy as URL subscriptions: direct connections, env proxies
        // ignored (dev shells often export http_proxy).
        .no_proxy()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(mlua::Error::runtime)?;
    let request = lua.create_function(move |lua, opts: Table| {
        let url: String = opts
            .get::<Option<String>>("url")?
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| mlua::Error::runtime("http.request requires a url"))?;
        let method = opts
            .get::<Option<String>>("method")?
            .unwrap_or_else(|| "GET".into())
            .trim()
            .to_uppercase();
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| mlua::Error::runtime(format!("unsupported HTTP method: {method}")))?;
        let timeout = opts
            .get::<Option<f64>>("timeout")?
            .filter(|t| t.is_finite() && *t > 0.0)
            .unwrap_or(REQUEST_TIMEOUT_SECS)
            .clamp(1.0, 600.0);

        let mut rb = client
            .request(method, &url)
            .timeout(Duration::from_secs_f64(timeout));
        if let Some(headers) = opts.get::<Option<Table>>("headers")? {
            for pair in headers.pairs::<String, Value>() {
                let (k, v) = pair?;
                rb = rb.header(k, value_to_string(&v));
            }
        }
        if let Some(body) = opts.get::<Option<String>>("body")? {
            rb = rb.body(body);
        }

        let resp = rb
            .send()
            .map_err(|e| mlua::Error::runtime(format!("request {url} failed: {e}")))?;
        let status = i64::from(resp.status().as_u16());
        let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in resp.headers() {
            let value = v.to_str().unwrap_or_default().to_string();
            headers
                .entry(k.as_str().to_ascii_lowercase())
                .or_default()
                .push(value);
        }
        let cookies = headers.get("set-cookie").cloned().unwrap_or_default();

        let mut buf = Vec::new();
        resp.take(MAX_BODY_BYTES as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| mlua::Error::runtime(format!("cannot read the response of {url}: {e}")))?;
        if buf.len() > MAX_BODY_BYTES {
            return Err(mlua::Error::runtime(format!(
                "{url} returned more than the {} MB body limit",
                MAX_BODY_BYTES / (1024 * 1024)
            )));
        }

        let out = lua.create_table()?;
        out.set("status", status)?;
        out.set("body", String::from_utf8_lossy(&buf).into_owned())?;
        let hs = lua.create_table()?;
        for (k, vs) in &headers {
            hs.set(k.as_str(), vs.join("\n"))?;
        }
        out.set("headers", hs)?;
        let ck = lua.create_table()?;
        for (i, c) in cookies.iter().enumerate() {
            ck.set(i as i64 + 1, c.as_str())?;
        }
        out.set("cookies", ck)?;
        Ok(out)
    })?;
    api.set("request", request)?;
    Ok(api)
}

fn json_api(lua: &Lua) -> mlua::Result<Table> {
    let api = lua.create_table()?;
    api.set(
        "encode",
        lua.create_function(|_, v: Value| {
            serde_json::to_string(&lua_to_json(&v))
                .map_err(|e| mlua::Error::runtime(format!("json.encode failed: {e}")))
        })?,
    )?;
    api.set(
        "decode",
        lua.create_function(|lua, s: mlua::String| {
            let text = s.to_string_lossy();
            let parsed: Json = serde_json::from_str(&text)
                .map_err(|e| mlua::Error::runtime(format!("json.decode failed: {e}")))?;
            json_to_lua(lua, &parsed)
        })?,
    )?;
    Ok(api)
}

fn base64_api(lua: &Lua) -> mlua::Result<Table> {
    let api = lua.create_table()?;
    api.set(
        "encode",
        lua.create_function(|_, s: mlua::String| Ok(base64_encode(&s.as_bytes())))?,
    )?;
    api.set(
        "decode",
        lua.create_function(|lua, s: mlua::String| {
            let text = s.to_string_lossy();
            let bytes = base64_decode(&text).map_err(mlua::Error::runtime)?;
            lua.create_string(&bytes)
        })?,
    )?;
    Ok(api)
}

/// `config.get(key[, default])`, `config.all()`, `config.set(key, value)`.
/// `set` persists to `<stem>.json` next to the plugin.
fn config_api(lua: &Lua, ctx: &Arc<RunCtx>) -> mlua::Result<Table> {
    let api = lua.create_table()?;
    api.set(
        "get",
        lua.create_function({
            let ctx = ctx.clone();
            move |lua, (key, default): (String, Option<Value>)| {
                let map = ctx.config.lock();
                match map.get(&key) {
                    Some(v) => json_to_lua(lua, v),
                    None => Ok(default.unwrap_or(Value::Nil)),
                }
            }
        })?,
    )?;
    api.set(
        "all",
        lua.create_function({
            let ctx = ctx.clone();
            move |lua, ()| {
                let map = ctx.config.lock().clone();
                json_to_lua(lua, &Json::Object(map))
            }
        })?,
    )?;
    api.set(
        "set",
        lua.create_function({
            let ctx = ctx.clone();
            move |_, (key, value): (String, Value)| {
                let json = lua_to_json(&value);
                let snapshot = {
                    let mut map = ctx.config.lock();
                    if json.is_null() {
                        map.remove(&key);
                    } else {
                        map.insert(key, json);
                    }
                    map.clone()
                };
                write_config(&ctx.config_path, &snapshot).map_err(mlua::Error::runtime)?;
                Ok(())
            }
        })?,
    )?;
    Ok(api)
}

// ---- value conversions --------------------------------------------------

fn type_name(v: &Value) -> &'static str {
    v.type_name()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy(),
        other => other.type_name().into(),
    }
}

fn lua_to_json(v: &Value) -> Json {
    match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::Number((*i).into()),
        Value::Number(n) => serde_json::Number::from_f64(*n).map_or(Json::Null, Json::Number),
        Value::String(s) => Json::String(s.to_string_lossy()),
        Value::Table(t) => {
            if t.raw_len() > 0 {
                let arr: Vec<Json> = t
                    .sequence_values::<Value>()
                    .filter_map(|v| v.ok())
                    .map(|v| lua_to_json(&v))
                    .collect();
                Json::Array(arr)
            } else {
                let mut map = JsonMap::new();
                for (k, val) in t.pairs::<Value, Value>().flatten() {
                    let key = match k {
                        Value::String(s) => s.to_string_lossy(),
                        other => value_to_string(&other),
                    };
                    map.insert(key, lua_to_json(&val));
                }
                Json::Object(map)
            }
        }
        _ => Json::Null,
    }
}

fn json_to_lua(lua: &Lua, v: &Json) -> mlua::Result<Value> {
    Ok(match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(arr) => {
            let t = lua.create_table_with_capacity(arr.len(), 0)?;
            for (i, item) in arr.iter().enumerate() {
                t.set(i as i64 + 1, json_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        Json::Object(map) => {
            let t = lua.create_table_with_capacity(0, map.len())?;
            for (k, item) in map {
                t.set(k.as_str(), json_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
    })
}

// ---- base64 (tiny, no extra dependency) ---------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = input
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        if chunk.len() == 1 {
            return Err("base64.decode: invalid input length".into());
        }
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            let v = val(*c).ok_or_else(|| format!("base64.decode: invalid character {:?}", *c as char))?;
            n |= v << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

// ---- tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn tmpdir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nekos-plugins-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn first_run_seeds_the_example_plugin_once() {
        let dir = tmpdir("seed").join("subs");
        assert!(ensure_dir_in(&dir).unwrap(), "fresh dir is seeded");
        let seeded = dir.join(EXAMPLE_FILE);
        assert!(seeded.is_file());
        // the bundled plugin must be loadable as shipped
        let list = list_in(&dir);
        assert_eq!(list.len(), 1, "{list:?}");
        assert!(list[0].error.is_none(), "{:?}", list[0].error);
        assert_eq!(list[0].name, "0dy10");

        // an existing dir (even one the user emptied) is left alone
        std::fs::remove_file(&seeded).unwrap();
        assert!(!ensure_dir_in(&dir).unwrap());
        assert!(!seeded.exists(), "seeding must not resurrect deleted plugins");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn base64_roundtrip() {
        for s in ["", "a", "ab", "abc", "abcd", "nájsök"] {
            assert_eq!(base64_decode(&base64_encode(s.as_bytes())).unwrap(), s.as_bytes());
        }
        assert_eq!(base64_encode(b"abc"), "YWJj");
        assert!(base64_decode("!!!").is_err());
    }

    #[test]
    fn lists_plugins_and_reports_broken_ones() {
        let dir = tmpdir("list");
        std::fs::write(
            dir.join("ok.lua"),
            r#"
            return { name = "OK", description = "d" }
            "#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.lua"), "error('no module')").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let list = list_in(&dir);
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!(list[0].file, "broken.lua");
        assert!(list[0].error.as_deref().unwrap().contains("no module"));
        assert_eq!(list[1].file, "ok.lua");
        assert_eq!(list[1].name, "OK");
        assert!(list[1].error.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_body_and_exposes_json_base64_config_log() {
        let dir = tmpdir("run");
        std::fs::write(
            dir.join("t.lua"),
            r#"
            return {
              name = "t",
              fetch = function(ctx)
                log("hello", 7, true)
                config.set("seen", { count = 1, tags = {"a", "b"} })
                local payload = json.decode('{"a":[1,2],"b":"x"}')
                local enc = base64.encode(payload.b)
                assert(json.encode(payload.a) == "[1,2]")
                assert(base64.decode(enc) == "x")
                assert(config.get("seen").count == 1)
                assert(config.get("missing", "d") == "d")
                assert(ctx.name == "t")
                assert(ctx.config == nil or type(ctx.config) == "table")
                return "body:" .. enc .. ":" .. ctx.name
              end,
            }
            "#,
        )
        .unwrap();

        let out = run_in(&dir, "t.lua", Duration::from_secs(10)).unwrap();
        assert_eq!(out.body, "body:eA==:t");
        assert_eq!(out.logs, vec!["hello 7 true"]);
        // config.set must have persisted to t.json
        let saved: Json = serde_json::from_str(&std::fs::read_to_string(dir.join("t.json")).unwrap())
            .unwrap();
        assert_eq!(saved["seen"]["tags"][1], Json::String("b".into()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("t.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "plugin config holds credentials");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_return_carries_body_and_content_type() {
        let dir = tmpdir("table");
        std::fs::write(
            dir.join("t.lua"),
            r#"return { fetch = function()
                 return { body = "x", content_type = "text/plain" }
               end }"#,
        )
        .unwrap();
        let out = run_in(&dir, "t.lua", Duration::from_secs(10)).unwrap();
        assert_eq!(out.body, "x");
        assert_eq!(out.content_type.as_deref(), Some("text/plain"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_body_missing_fetch_and_traversal() {
        let dir = tmpdir("bad");
        std::fs::write(dir.join("empty.lua"), r#"return { fetch = function() return "" end }"#).unwrap();
        std::fs::write(dir.join("nofetch.lua"), r#"return { name = "x" }"#).unwrap();
        std::fs::write(dir.join("notable.lua"), r#"return 5"#).unwrap();

        let e = run_in(&dir, "empty.lua", Duration::from_secs(10)).unwrap_err();
        assert!(e.contains("empty body"), "{e}");
        let e = run_in(&dir, "nofetch.lua", Duration::from_secs(10)).unwrap_err();
        assert!(e.contains("fetch"), "{e}");
        let e = run_in(&dir, "notable.lua", Duration::from_secs(10)).unwrap_err();
        assert!(e.contains("must return a config table"), "{e}");
        let e = run_in(&dir, "../etc/passwd.lua", Duration::from_secs(10)).unwrap_err();
        assert!(e.contains("invalid plugin file name"), "{e}");
        let e = run_in(&dir, "missing.lua", Duration::from_secs(10)).unwrap_err();
        assert!(e.contains("not found"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn infinite_loop_is_stopped_by_the_deadline() {
        let dir = tmpdir("loop");
        std::fs::write(
            dir.join("spin.lua"),
            r#"return { fetch = function() while true do end end }"#,
        )
        .unwrap();
        let started = Instant::now();
        let e = run_in(&dir, "spin.lua", Duration::from_secs(1)).unwrap_err();
        assert!(e.contains("deadline"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(30), "hook did not fire");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_request_surfaces_status_headers_cookies_and_body() {
        let site = MockSite::start();
        let dir = tmpdir("http");
        std::fs::write(
            dir.join("t.lua"),
            r#"
            return { fetch = function()
              local r = http.request({
                url = "%BASE%/echo",
                method = "POST",
                headers = { ["X-Test"] = "1", ["User-Agent"] = "nekos-test" },
                body = "payload",
              })
              assert(r.status == 200, "status " .. tostring(r.status))
              assert(r.headers["x-served"] == "yes")
              assert(r.headers["set-cookie"]:find("uid=abc") ~= nil)
              assert(#r.cookies == 1)
              assert(r.body == "echo:1:payload:200", r.body)
              local fail = pcall(function() http.request({ url = "http://127.0.0.1:1/x", timeout = 2 }) end)
              assert(fail == false, "connection failure must raise")
              return "ok"
            end }"#
            .replace("%BASE%", &site.base),
        )
        .unwrap();

        let out = run_in(&dir, "t.lua", Duration::from_secs(30)).unwrap();
        assert_eq!(out.body, "ok");
        assert_eq!(site.hits().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- mock paid-subscription site ------------------------------------

    /// Minimal HTTP/1.1 server modelling the paid site's flow: form login
    /// (sets a session cookie), cookie-protected activate, and a link that
    /// only answers clash-like UAs.
    struct MockSite {
        base: String,
        hits: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockSite {
        const TOKEN_PATH: &'static str = "/link/TOKEN123?clash=2";
        const MARKER: &'static str = "MOCK-SUB-MARKER";

        fn start() -> MockSite {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (hits_t, stop_t) = (hits.clone(), stop.clone());
            let handle = std::thread::spawn(move || {
                while !stop_t.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let hits = hits_t.clone();
                            std::thread::spawn(move || handle_conn(stream, hits));
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            MockSite {
                base: format!("http://{addr}"),
                hits,
                stop,
                handle: Some(handle),
            }
        }

        fn hits(&self) -> Vec<String> {
            self.hits.lock().clone()
        }
    }

    impl Drop for MockSite {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    fn handle_conn(stream: TcpStream, hits: Arc<Mutex<Vec<String>>>) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() || request.trim().is_empty() {
            return;
        }
        let mut len = 0usize;
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let line = line.trim_end().to_string();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim().to_ascii_lowercase();
                headers.insert(k.clone(), v.trim().to_string());
                if k == "content-length" {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0u8; len];
        if len > 0 {
            use std::io::Read as _;
            let _ = reader.read_exact(&mut body);
        }
        let body = String::from_utf8_lossy(&body).into_owned();

        let mut parts = request.trim().split(' ');
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        hits.lock().push(format!("{method} {path}"));

        let cookie = headers.get("cookie").cloned().unwrap_or_default();
        let ua = headers.get("user-agent").cloned().unwrap_or_default();
        let referer = headers.get("referer").cloned().unwrap_or_default();
        let xhr = headers.contains_key("x-requested-with");
        let form_ok = headers
            .get("content-type")
            .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
        let (status, extra, payload) = match (method.as_str(), path.as_str()) {
            // Login: XHR + form body + browser UA (site rejects other shapes).
            ("POST", "/auth/login")
                if body.contains("email=alice%40example.com")
                    && form_ok
                    && xhr
                    && referer.ends_with("/auth/login")
                    && ua.contains("Chrome") =>
            {
                (
                    "200 OK",
                    "Set-Cookie: uid=mockuid123; Path=/\r\n",
                    r#"{"ret":1,"msg":"ok"}"#.to_string(),
                )
            }
            ("POST", "/auth/login") => (
                "200 OK",
                "Set-Cookie: uid=nope; Path=/\r\n",
                r#"{"ret":0,"msg":"no such mailbox"}"#.to_string(),
            ),
            // Activate: session cookie, plain Accept, no XHR header.
            ("POST", "/user/activate_sub")
                if cookie.contains("uid=mockuid123") && !xhr && referer.ends_with("/user") =>
            {
                ("200 OK", "", r#"{"ret":1,"msg":"activated"}"#.to_string())
            }
            ("POST", "/user/activate_sub") => (
                "302 Found",
                "Location: /auth/login\r\n",
                String::new(),
            ),
            // Link: no cookies, clash-only UA.
            ("GET", p)
                if p == MockSite::TOKEN_PATH
                    && ua.to_ascii_lowercase().contains("clash")
                    && !headers.contains_key("cookie") =>
            {
                (
                    "200 OK",
                    "",
                    format!("proxies:\n  - name: mock\n# {}\n", MockSite::MARKER),
                )
            }
            ("GET", p) if p == MockSite::TOKEN_PATH => ("504 Gateway Timeout", "", "bad ua".into()),
            ("POST", "/echo") => (
                "200 OK",
                "Set-Cookie: uid=abc; Path=/\r\nX-Served: yes\r\n",
                format!(
                    "echo:{}:{}:200",
                    headers.get("x-test").cloned().unwrap_or_default(),
                    body
                ),
            ),
            _ => ("404 Not Found", "", "not found".into()),
        };

        let mut stream = stream;
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    }

    #[test]
    fn shipped_plugin_runs_against_mock_site() {
        let site = MockSite::start();
        let dir = tmpdir("e2e");
        std::fs::write(dir.join(EXAMPLE_FILE), EXAMPLE_PLUGIN).unwrap();
        std::fs::write(
            dir.join("0dy10.json"),
            format!(
                r#"{{
                  "origin": "{}",
                  "link": "{}{}",
                  "email": "alice@example.com",
                  "passwd": "secret",
                  "code": ""
                }}"#,
                site.base,
                site.base,
                MockSite::TOKEN_PATH
            ),
        )
        .unwrap();

        let out = run_in(&dir, EXAMPLE_FILE, Duration::from_secs(30)).unwrap();
        assert!(out.body.contains(MockSite::MARKER), "{}", out.body);
        assert!(out.logs.iter().any(|l| l.starts_with("fetched subscription")), "{:?}", out.logs);

        let hits = site.hits();
        assert_eq!(
            hits,
            vec![
                "POST /auth/login".to_string(),
                "POST /user/activate_sub".to_string(),
                format!("GET {}", MockSite::TOKEN_PATH),
            ],
            "login → activate → link order"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_plugin_reports_login_rejection() {
        let site = MockSite::start();
        let dir = tmpdir("e2e-bad");
        std::fs::write(dir.join(EXAMPLE_FILE), EXAMPLE_PLUGIN).unwrap();
        std::fs::write(
            dir.join("0dy10.json"),
            format!(
                r#"{{ "origin": "{}", "link": "{}{}", "email": "bob@example.com", "passwd": "wrong" }}"#,
                site.base,
                site.base,
                MockSite::TOKEN_PATH
            ),
        )
        .unwrap();

        let err = run_in(&dir, EXAMPLE_FILE, Duration::from_secs(30)).unwrap_err();
        assert!(err.contains("no such mailbox") || err.contains("login failed"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
