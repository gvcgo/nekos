//! SQLite persistence owned by the orchestrator (groups / nodes /
//! settings). The DB schema mirrors v2rayN's group+node model but stores
//! each node's outbound JSON verbatim (sing-box-native, see architecture.md
//! §5), so the orchestrator never re-implements protocol logic.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Group {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// JSON map of extra headers (without User-Agent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_userinfo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// unix seconds of the last successful fetch/refresh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_epoch: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Node {
    pub id: String,
    pub group_id: i64,
    pub r#type: String,
    pub remark: String,
    /// sing-box outbound options JSON (without tag).
    pub out: String,
}

#[derive(Deserialize, Clone)]
pub struct NewNode {
    pub id: String,
    pub r#type: String,
    pub remark: String,
    pub out: String,
}

fn default_log_level() -> String {
    "warn".into()
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_auto_minutes() -> u32 {
    360 // 6h default
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub current_group_id: i64,
    pub port: u16,
    pub mode: String, // global | direct | rule
    pub proxy_enabled: bool,
    pub close_to_tray: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String, // debug | info | warn | error
    #[serde(default = "default_true")]
    pub sort_by_delay: bool,
    #[serde(default = "default_false")]
    pub filter_ipv6: bool,
    #[serde(default = "default_false")]
    pub auto_update_subscriptions: bool,
    /// interval in minutes between automatic subscription refreshes.
    #[serde(default = "default_auto_minutes")]
    pub auto_update_minutes: u32,
    pub selected_by_group: std::collections::HashMap<i64, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            current_group_id: 1,
            port: 2080,
            mode: "global".into(),
            proxy_enabled: false, // system proxy is opt-in via the toolbar switch
            close_to_tray: true,
            log_level: default_log_level(),
            sort_by_delay: true,
            filter_ipv6: false,
            auto_update_subscriptions: false,
            auto_update_minutes: default_auto_minutes(),
            selected_by_group: Default::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LatencyRow {
    pub node_id: String,
    pub delay_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tested_at: String,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS groups (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT NOT NULL,
                sub_url    TEXT,
                sub_userinfo TEXT,
                updated_at TEXT
            );
            CREATE TABLE IF NOT EXISTS nodes (
                id       TEXT NOT NULL,
                group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                type     TEXT NOT NULL,
                remark   TEXT NOT NULL,
                out      TEXT NOT NULL,
                PRIMARY KEY (group_id, id)
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS latency (
                group_id  INTEGER NOT NULL,
                node_id   TEXT NOT NULL,
                delay_ms  INTEGER,
                error     TEXT,
                tested_at TEXT NOT NULL,
                PRIMARY KEY (group_id, node_id)
            );
            INSERT OR IGNORE INTO groups (id, name) VALUES (1, '默认分组');",
        )?;
        self.ensure_column("groups", "user_agent", "TEXT")?;
        self.ensure_column("groups", "extra_headers", "TEXT")?;
        self.ensure_column("groups", "last_update_epoch", "INTEGER")?;
        Ok(())
    }

    /// ALTER TABLE ADD COLUMN (SQLite has no IF NOT EXISTS here).
    fn ensure_column(&self, table: &str, column: &str, decl: &str) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<_, _>>()?;
        if !names.iter().any(|n| n == column) {
            self.conn
                .execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
        }
        Ok(())
    }

    // ---- groups ----

    pub fn list_groups(&self) -> rusqlite::Result<Vec<Group>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, sub_url, user_agent, extra_headers, sub_userinfo, updated_at,
                    last_update_epoch
             FROM groups ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
                sub_url: row.get(2)?,
                user_agent: row.get(3)?,
                extra_headers: row.get(4)?,
                sub_userinfo: row.get(5)?,
                updated_at: row.get(6)?,
                last_update_epoch: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn create_group(&self, name: &str, sub_url: Option<&str>) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO groups (name, sub_url) VALUES (?1, ?2)",
            params![name, sub_url],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_group(&self, id: i64, name: &str) -> rusqlite::Result<()> {
        if id == 1 {
            return Ok(()); // keep the default group named as-is
        }
        self.conn.execute(
            "UPDATE groups SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    pub fn delete_group(&self, id: i64) -> rusqlite::Result<()> {
        // Nodes cascade; the default group (1) stays to keep a sane state.
        if id == 1 {
            return Ok(());
        }
        self.conn.execute(
            "DELETE FROM latency WHERE group_id = ?1",
            params![id],
        )?;
        self.conn.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn group(&self, id: i64) -> rusqlite::Result<Option<Group>> {
        self.conn
            .query_row(
                "SELECT id, name, sub_url, user_agent, extra_headers, sub_userinfo, updated_at,
                        last_update_epoch
                 FROM groups WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Group {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        sub_url: row.get(2)?,
                        user_agent: row.get(3)?,
                        extra_headers: row.get(4)?,
                        sub_userinfo: row.get(5)?,
                        updated_at: row.get(6)?,
                        last_update_epoch: row.get(7)?,
                    })
                },
            )
            .optional()
    }

    /// Persist a group's subscription meta (url, ua, extra headers,
    /// userinfo) and bump updated_at.
    pub fn update_group_submeta(
        &self,
        id: i64,
        sub_url: Option<&str>,
        user_agent: Option<&str>,
        extra_headers: Option<&str>,
        userinfo: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE groups SET sub_url = ?2, user_agent = ?3, extra_headers = ?4,
             sub_userinfo = ?5,
             updated_at = strftime('%Y-%m-%d %H:%M:%S', 'now', 'localtime'),
             last_update_epoch = CAST(strftime('%s', 'now') AS INTEGER)
             WHERE id = ?1",
            params![id, sub_url, user_agent, extra_headers, userinfo],
        )?;
        Ok(())
    }

    // ---- nodes ----

    pub fn upsert_nodes(&self, group_id: i64, nodes: &[NewNode]) -> rusqlite::Result<usize> {
        let mut inserted = 0;
        {
            let mut stmt = self.conn.prepare_cached(
                "INSERT OR REPLACE INTO nodes (id, group_id, type, remark, out)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for n in nodes {
                stmt.execute(params![n.id, group_id, n.r#type, n.remark, n.out])?;
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    pub fn list_nodes(&self, group_id: i64) -> rusqlite::Result<Vec<Node>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, group_id, type, remark, out FROM nodes
             WHERE group_id = ?1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![group_id], |row| {
            Ok(Node {
                id: row.get(0)?,
                group_id: row.get(1)?,
                r#type: row.get(2)?,
                remark: row.get(3)?,
                out: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn node(&self, group_id: i64, id: &str) -> rusqlite::Result<Option<Node>> {
        self.conn
            .query_row(
                "SELECT id, group_id, type, remark, out FROM nodes WHERE group_id = ?1 AND id = ?2",
                params![group_id, id],
                |row| {
                    Ok(Node {
                        id: row.get(0)?,
                        group_id: row.get(1)?,
                        r#type: row.get(2)?,
                        remark: row.get(3)?,
                        out: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    pub fn delete_node(&self, group_id: i64, id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM nodes WHERE group_id = ?1 AND id = ?2",
            params![group_id, id],
        )?;
        self.conn.execute(
            "DELETE FROM latency WHERE group_id = ?1 AND node_id = ?2",
            params![group_id, id],
        )?;
        Ok(())
    }

    /// Replace a group's node set with freshly fetched subscription nodes
    /// (old latency results for the group are dropped too). Runs in one
    /// transaction so readers never observe an empty window.
    pub fn replace_group_nodes(&self, group_id: i64, nodes: &[NewNode]) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM nodes WHERE group_id = ?1", params![group_id])?;
        tx.execute("DELETE FROM latency WHERE group_id = ?1", params![group_id])?;
        let mut inserted = 0;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO nodes (id, group_id, type, remark, out)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for n in nodes {
                stmt.execute(params![n.id, group_id, n.r#type, n.remark, n.out])?;
                inserted += 1;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn upsert_latency(
        &self,
        group_id: i64,
        node_id: &str,
        delay_ms: Option<i64>,
        error: Option<&str>,
        tested_at: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO latency (group_id, node_id, delay_ms, error, tested_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![group_id, node_id, delay_ms, error, tested_at],
        )?;
        Ok(())
    }

    pub fn list_latency(&self, group_id: i64) -> rusqlite::Result<Vec<LatencyRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, delay_ms, error, tested_at FROM latency WHERE group_id = ?1",
        )?;
        let rows = stmt.query_map(params![group_id], |row| {
            Ok(LatencyRow {
                node_id: row.get(0)?,
                delay_ms: row.get(1)?,
                error: row.get(2)?,
                tested_at: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Copy nodes (by id) from one group into another. Existing target
    /// nodes with the same id are left untouched. Returns
    /// (inserted, skipped_duplicates).
    pub fn copy_group_nodes(
        &self,
        source_group: i64,
        target_group: i64,
        node_ids: &[String],
    ) -> rusqlite::Result<(usize, usize)> {
        if source_group == target_group {
            return Ok((0, node_ids.len()));
        }
        let source: Vec<Node> = self
            .list_nodes(source_group)?
            .into_iter()
            .filter(|n| node_ids.contains(&n.id))
            .collect();
        let existing: std::collections::HashSet<String> = self
            .list_nodes(target_group)?
            .into_iter()
            .map(|n| n.id)
            .collect();
        let mut inserted = 0usize;
        let mut duplicated = 0usize;
        {
            let mut stmt = self.conn.prepare_cached(
                "INSERT OR IGNORE INTO nodes (id, group_id, type, remark, out)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for n in &source {
                if existing.contains(&n.id) {
                    duplicated += 1;
                    continue;
                }
                stmt.execute(params![n.id, target_group, n.r#type, n.remark, n.out])?;
                inserted += 1;
            }
        }
        Ok((inserted, duplicated))
    }

    // ---- settings (single JSON blob) ----

    pub fn load_settings(&self) -> Settings {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |row| {
                row.get(0)
            })
            .optional()
            .unwrap_or(None);
        match raw {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            None => Settings::default(),
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> rusqlite::Result<()> {
        let raw = serde_json::to_string(settings)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES ('app', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![raw],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_tmp() -> Db {
        let dir = std::env::temp_dir().join(format!("nekos-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Db::open(&dir.join("test.db")).unwrap()
    }

    #[test]
    fn groups_and_nodes_roundtrip() {
        let db = open_tmp();
        let groups = db.list_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "默认分组");

        let gid = db.create_group("机场A", Some("https://x/sub")).unwrap();
        let node = NewNode {
            id: "abc123".into(),
            r#type: "anytls".into(),
            remark: "测试节点".into(),
            out: r#"{"type":"anytls","server":"h"}"#.into(),
        };
        assert_eq!(db.upsert_nodes(gid, &[node]).unwrap(), 1);
        // re-insert with same id: upsert keeps one row
        assert_eq!(
            db.upsert_nodes(
                gid,
                &[NewNode {
                    id: "abc123".into(),
                    r#type: "anytls".into(),
                    remark: "改名".into(),
                    out: r#"{"type":"anytls"}"#.into(),
                }]
            )
            .unwrap(),
            1
        );
        let nodes = db.list_nodes(gid).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].remark, "改名");

        db.delete_node(gid, "abc123").unwrap();
        assert!(db.list_nodes(gid).unwrap().is_empty());
        db.delete_group(gid).unwrap();
        assert_eq!(db.list_groups().unwrap().len(), 1);
    }

    #[test]
    fn latency_roundtrip() {
        let db = open_tmp();
        db.upsert_latency(1, "node-a", Some(123), None, "1700000000").unwrap();
        db.upsert_latency(1, "node-b", None, Some("timeout"), "1700000001").unwrap();
        let rows = db.list_latency(1).unwrap();
        assert_eq!(rows.len(), 2);
        let a = rows.iter().find(|r| r.node_id == "node-a").unwrap();
        assert_eq!(a.delay_ms, Some(123));
        let b = rows.iter().find(|r| r.node_id == "node-b").unwrap();
        assert_eq!(b.error.as_deref(), Some("timeout"));
        // re-test overwrites
        db.upsert_latency(1, "node-a", Some(88), None, "1700000002").unwrap();
        assert_eq!(db.list_latency(1).unwrap().len(), 2);
        let rows = db.list_latency(1).unwrap();
        let a = rows.iter().find(|r| r.node_id == "node-a").unwrap();
        assert_eq!(a.delay_ms, Some(88));
        // deleting the node removes its latency row
        db.delete_node(1, "node-a").unwrap();
        assert_eq!(db.list_latency(1).unwrap().len(), 1);
    }

    #[test]
    fn settings_roundtrip() {
        let db = open_tmp();
        let mut s = db.load_settings();
        assert_eq!(s.port, 2080);
        s.port = 10808;
        s.mode = "direct".into();
        s.selected_by_group.insert(1, "node-x".into());
        db.save_settings(&s).unwrap();
        let s2 = db.load_settings();
        assert_eq!(s2.port, 10808);
        assert_eq!(s2.mode, "direct");
        assert_eq!(s2.selected_by_group.get(&1), Some(&"node-x".to_string()));
    }
}
