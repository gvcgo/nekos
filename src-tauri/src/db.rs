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
    pub sub_userinfo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
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

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub current_group_id: i64,
    pub port: u16,
    pub mode: String, // global | direct | rule
    pub proxy_enabled: bool,
    pub close_to_tray: bool,
    pub selected_by_group: std::collections::HashMap<i64, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            current_group_id: 1,
            port: 2080,
            mode: "global".into(),
            proxy_enabled: true,
            close_to_tray: true,
            selected_by_group: Default::default(),
        }
    }
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
            INSERT OR IGNORE INTO groups (id, name) VALUES (1, '默认分组');",
        )
    }

    // ---- groups ----

    pub fn list_groups(&self) -> rusqlite::Result<Vec<Group>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, sub_url, sub_userinfo, updated_at FROM groups ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
                sub_url: row.get(2)?,
                sub_userinfo: row.get(3)?,
                updated_at: row.get(4)?,
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
        self.conn
            .execute("DELETE FROM groups WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn group(&self, id: i64) -> rusqlite::Result<Option<Group>> {
        self.conn
            .query_row(
                "SELECT id, name, sub_url, sub_userinfo, updated_at FROM groups WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Group {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        sub_url: row.get(2)?,
                        sub_userinfo: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    pub fn touch_group_meta(
        &self,
        id: i64,
        userinfo: Option<&str>,
        updated_at: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE groups SET sub_userinfo = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, userinfo, updated_at],
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
        Ok(())
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
