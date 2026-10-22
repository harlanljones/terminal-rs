//! Minimal tracked-entity + alert store on SQLite (spec 4.7 app: `TRACK`/`UNTRACK`/`ALRT`).
//!
//! Two tables: `tracked(entity_key TEXT PRIMARY KEY, added INTEGER)` and
//! `alerts(id INTEGER PRIMARY KEY, entity_key TEXT, created INTEGER, message TEXT, seen INTEGER)`.
//! This stays behind the app worker until the congress store is replaced in a later wave.

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// One alert raised against a tracked entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRow {
    pub id: i64,
    pub entity_key: String,
    /// Unix seconds the alert was raised.
    pub created: i64,
    pub message: String,
    pub seen: bool,
}

/// SQLite-backed tracked-player and alert store. Connection access is serialised so the
/// tokio worker can use it from any task.
pub struct TrackStore {
    conn: Mutex<Connection>,
}

#[derive(Debug, thiserror::Error)]
pub enum TrackError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl TrackStore {
    /// Open (creating the tables if absent) at `path`.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, TrackError> {
        Self::from_conn(Connection::open(path)?)
    }

    /// In-memory store (tests and the degraded app start).
    pub fn open_in_memory() -> Result<Self, TrackError> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self, TrackError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracked(entity_key TEXT PRIMARY KEY, added INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS alerts(id INTEGER PRIMARY KEY AUTOINCREMENT, entity_key TEXT NOT NULL,
                 created INTEGER NOT NULL, message TEXT NOT NULL, seen INTEGER NOT NULL DEFAULT 0);",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Add `entity_key` to the tracked set (idempotent; re-tracking keeps the original
    /// stamp). `added` is unix seconds.
    pub fn track(&self, entity_key: &str, added: i64) -> Result<(), TrackError> {
        let conn = self.conn.lock().expect("track lock");
        conn.execute(
            "INSERT INTO tracked(entity_key, added) VALUES(?1, ?2) ON CONFLICT(entity_key) DO NOTHING",
            [entity_key, &added.to_string()],
        )?;
        Ok(())
    }

    /// Remove `entity_key`; `true` when it was tracked.
    pub fn untrack(&self, entity_key: &str) -> Result<bool, TrackError> {
        let conn = self.conn.lock().expect("track lock");
        let n = conn.execute("DELETE FROM tracked WHERE entity_key = ?1", [entity_key])?;
        Ok(n > 0)
    }

    /// Is this entity currently tracked?
    pub fn is_tracked(&self, entity_key: &str) -> Result<bool, TrackError> {
        let conn = self.conn.lock().expect("track lock");
        let on: Option<i64> =
            conn.query_row("SELECT 1 FROM tracked WHERE entity_key = ?1", [entity_key], |r| r.get(0)).ok();
        Ok(on.is_some())
    }

    /// The tracked set, oldest first.
    pub fn tracked(&self) -> Result<Vec<(String, i64)>, TrackError> {
        let conn = self.conn.lock().expect("track lock");
        let mut st = conn.prepare("SELECT entity_key, added FROM tracked ORDER BY added, entity_key")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Raise one alert; returns its row id. Deduplication of trigger events is the caller's
    /// job (one alert per deduplicated event).
    pub fn add_alert(&self, entity_key: &str, created: i64, message: &str) -> Result<i64, TrackError> {
        let conn = self.conn.lock().expect("track lock");
        conn.execute(
            "INSERT INTO alerts(entity_key, created, message, seen) VALUES(?1, ?2, ?3, 0)",
            rusqlite::params![entity_key, created, message],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Alerts newest first; `unseen_only` filters on the seen flag.
    pub fn alerts(&self, unseen_only: bool, limit: usize) -> Result<Vec<AlertRow>, TrackError> {
        let conn = self.conn.lock().expect("track lock");
        let sql = match unseen_only {
            true => {
                "SELECT id, entity_key, created, message, seen FROM alerts WHERE seen = 0 ORDER BY id DESC LIMIT ?1"
            }
            false => "SELECT id, entity_key, created, message, seen FROM alerts ORDER BY id DESC LIMIT ?1",
        };
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map([limit as i64], |r| {
            Ok(AlertRow {
                id: r.get(0)?,
                entity_key: r.get(1)?,
                created: r.get(2)?,
                message: r.get(3)?,
                seen: r.get::<_, i64>(4)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Mark every alert seen; returns how many rows changed.
    pub fn mark_alerts_seen(&self) -> Result<usize, TrackError> {
        let conn = self.conn.lock().expect("track lock");
        Ok(conn.execute("UPDATE alerts SET seen = 1 WHERE seen = 0", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, TrackStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TrackStore::open(dir.path().join("track.sqlite3")).unwrap();
        (dir, store)
    }

    #[test]
    fn track_untrack_round_trip() {
        let (_d, s) = temp();
        assert!(!s.is_tracked("p:660271:bat").unwrap());
        s.track("p:660271:bat", 1_000).unwrap();
        s.track("p:660271:bat", 2_000).unwrap(); // idempotent: original stamp kept
        assert!(s.is_tracked("p:660271:bat").unwrap());
        assert_eq!(s.tracked().unwrap(), vec![("p:660271:bat".into(), 1_000)]);
        assert!(s.untrack("p:660271:bat").unwrap());
        assert!(!s.untrack("p:660271:bat").unwrap(), "second untrack is not a change");
        assert!(!s.is_tracked("p:660271:bat").unwrap());
    }

    #[test]
    fn alerts_round_trip() {
        let (_d, s) = temp();
        let id = s.add_alert("p:660271:bat", 5_000, "projection delta").unwrap();
        s.add_alert("p:660271:pit", 6_000, "note").unwrap();
        let all = s.alerts(false, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, id + 1, "newest first");
        assert!(!all[0].seen);
        assert_eq!(all[1].entity_key, "p:660271:bat");
        assert_eq!(s.alerts(true, 10).unwrap().len(), 2);
        assert_eq!(s.mark_alerts_seen().unwrap(), 2);
        assert!(s.alerts(true, 10).unwrap().is_empty());
        assert_eq!(s.alerts(false, 1).unwrap().len(), 1, "limit applies");
    }

    #[test]
    fn store_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.sqlite3");
        {
            let s = TrackStore::open(&path).unwrap();
            s.track("p:660271:bat", 1).unwrap();
            s.add_alert("p:660271:bat", 2, "hi").unwrap();
        }
        let s = TrackStore::open(&path).unwrap();
        assert!(s.is_tracked("p:660271:bat").unwrap());
        assert_eq!(s.alerts(false, 10).unwrap().len(), 1);
    }
}
