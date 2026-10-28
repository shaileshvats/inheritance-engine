//! Local-first storage: SQLite holding typed JSON documents plus per-inheritance artifacts
//! (pipeline checkpoints such as the inventory, agent outputs, and the report).

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};

pub struct Db {
    conn: Mutex<Connection>,
}

/// Document kinds stored in the `docs` table.
#[derive(Clone, Copy)]
pub enum Kind {
    Inheritance,
    Job,
    Call,
    Issue,
    Repair,
    Verification,
    Chat,
}

impl Kind {
    fn as_str(&self) -> &'static str {
        match self {
            Kind::Inheritance => "inheritance",
            Kind::Job => "job",
            Kind::Call => "call",
            Kind::Issue => "issue",
            Kind::Repair => "repair",
            Kind::Verification => "verification",
            Kind::Chat => "chat",
        }
    }
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS docs (
               kind TEXT NOT NULL,
               id TEXT NOT NULL,
               inheritance_id TEXT,
               created_at TEXT NOT NULL,
               json TEXT NOT NULL,
               PRIMARY KEY (kind, id)
             );
             CREATE INDEX IF NOT EXISTS docs_by_inheritance ON docs(kind, inheritance_id, created_at);
             CREATE TABLE IF NOT EXISTS artifacts (
               inheritance_id TEXT NOT NULL,
               key TEXT NOT NULL,
               updated_at TEXT NOT NULL,
               json TEXT NOT NULL,
               PRIMARY KEY (inheritance_id, key)
             );
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', '1');",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn put<T: Serialize>(&self, kind: Kind, id: &str, inheritance_id: Option<&str>, created_at: &str, doc: &T) -> anyhow::Result<()> {
        let json = serde_json::to_string(doc)?;
        self.lock().execute(
            "INSERT INTO docs(kind, id, inheritance_id, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(kind, id) DO UPDATE SET json = excluded.json",
            params![kind.as_str(), id, inheritance_id, created_at, json],
        )?;
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, kind: Kind, id: &str) -> anyhow::Result<Option<T>> {
        let json: Option<String> = self
            .lock()
            .query_row("SELECT json FROM docs WHERE kind = ?1 AND id = ?2", params![kind.as_str(), id], |r| r.get(0))
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// All documents of a kind for an inheritance (or all, if `inheritance_id` is None), oldest first.
    pub fn list<T: DeserializeOwned>(&self, kind: Kind, inheritance_id: Option<&str>) -> anyhow::Result<Vec<T>> {
        let conn = self.lock();
        let mut out = Vec::new();
        let rows: Vec<String> = match inheritance_id {
            Some(inh) => {
                let mut st = conn.prepare("SELECT json FROM docs WHERE kind = ?1 AND inheritance_id = ?2 ORDER BY created_at, rowid")?;
                let it = st.query_map(params![kind.as_str(), inh], |r| r.get(0))?;
                it.collect::<Result<_, _>>()?
            }
            None => {
                let mut st = conn.prepare("SELECT json FROM docs WHERE kind = ?1 ORDER BY created_at, rowid")?;
                let it = st.query_map(params![kind.as_str()], |r| r.get(0))?;
                it.collect::<Result<_, _>>()?
            }
        };
        for j in rows {
            out.push(serde_json::from_str(&j)?);
        }
        Ok(out)
    }

    pub fn count(&self, kind: Kind, inheritance_id: &str) -> anyhow::Result<usize> {
        let n: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM docs WHERE kind = ?1 AND inheritance_id = ?2",
            params![kind.as_str(), inheritance_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn put_artifact<T: Serialize>(&self, inheritance_id: &str, key: &str, value: &T) -> anyhow::Result<()> {
        let json = serde_json::to_string(value)?;
        self.lock().execute(
            "INSERT INTO artifacts(inheritance_id, key, updated_at, json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(inheritance_id, key) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
            params![inheritance_id, key, crate::util::now(), json],
        )?;
        Ok(())
    }

    pub fn get_artifact<T: DeserializeOwned>(&self, inheritance_id: &str, key: &str) -> anyhow::Result<Option<T>> {
        let json: Option<String> = self
            .lock()
            .query_row("SELECT json FROM artifacts WHERE inheritance_id = ?1 AND key = ?2", params![inheritance_id, key], |r| r.get(0))
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    /// Remove every document and artifact belonging to an inheritance, and the inheritance itself.
    pub fn delete_inheritance(&self, inheritance_id: &str) -> anyhow::Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM docs WHERE inheritance_id = ?1", params![inheritance_id])?;
        conn.execute("DELETE FROM docs WHERE kind = 'inheritance' AND id = ?1", params![inheritance_id])?;
        conn.execute("DELETE FROM artifacts WHERE inheritance_id = ?1", params![inheritance_id])?;
        Ok(())
    }

    pub fn delete_artifacts_with_prefix(&self, inheritance_id: &str, prefix: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "DELETE FROM artifacts WHERE inheritance_id = ?1 AND substr(key, 1, length(?2)) = ?2",
            params![inheritance_id, prefix],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_docs_and_artifacts() {
        let db = Db::open_in_memory().unwrap();
        db.put(Kind::Chat, "a", Some("inh1"), "2026-01-01T00:00:00Z", &serde_json::json!({"x": 1})).unwrap();
        db.put(Kind::Chat, "b", Some("inh1"), "2026-01-02T00:00:00Z", &serde_json::json!({"x": 2})).unwrap();
        db.put(Kind::Chat, "a", Some("inh1"), "2026-01-01T00:00:00Z", &serde_json::json!({"x": 3})).unwrap();
        let all: Vec<serde_json::Value> = db.list(Kind::Chat, Some("inh1")).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["x"], 3);
        assert_eq!(db.count(Kind::Chat, "inh1").unwrap(), 2);

        db.put_artifact("inh1", "step:plan", &vec![1, 2]).unwrap();
        db.put_artifact("inh1", "step:other", &vec![3]).unwrap();
        let v: Option<Vec<i32>> = db.get_artifact("inh1", "step:plan").unwrap();
        assert_eq!(v, Some(vec![1, 2]));
        db.delete_artifacts_with_prefix("inh1", "step:").unwrap();
        let v: Option<Vec<i32>> = db.get_artifact("inh1", "step:plan").unwrap();
        assert!(v.is_none());
    }
}
