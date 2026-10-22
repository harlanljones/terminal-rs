//! The raw store: a SQLite index plus zstd content-addressed blob files.
//!
//! Layout under the data dir:
//! ```text
//! <dir>/store.sqlite3        the index (WITHOUT ROWID, keyed by FetchKey string)
//! <dir>/blobs/<sha256>.zst   raw payload bytes, compressed once, shared across keys
//! ```
//! The `rusqlite::Connection` lives inside a [`std::sync::Mutex`]; no lock is ever held across an
//! `.await`, and fetch (network) is a separate step from [`RawStore::put`].

use crate::key::FetchKey;
use crate::policy::Policy;
use crate::provenance::Provenance;
use rusqlite::{Connection, OptionalExtension};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Error type for the store layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("store poisoned by a panicking writer")]
    Poisoned,
    #[error("corrupt blob for {0}")]
    CorruptBlob(String),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// How a loaded payload reached the caller (spec §"Pan render" freshness chips).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Shipped inside the binary/bundled corpus.
    Bundled,
    /// Served from the cache while still within the TTL; age for the chip.
    Cached { age_secs: u64 },
    /// Fetched from the network by this call.
    Fetched,
    /// A re-fetch was due but the source policy refused it (interval/budget exhausted).
    StaleRefused,
}

/// A payload paired with its provenance — the shape every facade value carries (spec §Provenance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded<T> {
    pub value: T,
    pub prov: Provenance,
    pub freshness: Freshness,
}

impl<T> Loaded<T> {
    pub fn new(value: T, prov: Provenance, freshness: Freshness) -> Self {
        Loaded { value, prov, freshness }
    }
}

/// Decompressed payload plus provenance, as returned by [`RawStore::get`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPayload {
    pub bytes: Vec<u8>,
    pub prov: Provenance,
}

struct StoreInner {
    conn: Connection,
    blobs: PathBuf,
}

/// The raw payload store. Sync only; the async facade owns the pacing and wraps these calls with
/// `tokio::task::spawn_blocking` (or equivalent) so the mutex is never held across an `.await`.
pub struct RawStore {
    inner: Mutex<StoreInner>,
}

impl RawStore {
    /// Open (creating if needed) the store under `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs)?;
        let conn = Connection::open(dir.join("store.sqlite3"))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS payloads (
                 key TEXT PRIMARY KEY,
                 source TEXT NOT NULL,
                 url TEXT NOT NULL,
                 fetched INTEGER NOT NULL,
                 sha256 BLOB NOT NULL,
                 bytes INTEGER NOT NULL,
                 path TEXT NOT NULL
             ) WITHOUT ROWID;",
        )?;
        Ok(RawStore { inner: Mutex::new(StoreInner { conn, blobs }) })
    }

    /// The path a SHA-256 hashes to in the blob directory.
    pub fn blob_path(blobs: &Path, sha256: &[u8; 32]) -> PathBuf {
        let mut name = String::with_capacity(64 + 4);
        for byte in sha256.iter().copied() {
            name.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
            name.push(char::from_digit((byte & 0xf) as u32, 16).unwrap_or('0'));
        }
        name.push_str(".zst");
        blobs.join(name)
    }

    /// Fetch the stored payload for `key`, decompressing from the content-addressed blob.
    pub fn get(&self, key: &FetchKey) -> Result<Option<StoredPayload>> {
        let key = key.as_str();
        let inner = self.inner.lock().map_err(|_| StoreError::Poisoned)?;
        let row = inner
            .conn
            .query_row(
                "SELECT key, source, url, fetched, sha256, bytes, path FROM payloads WHERE key = ?1",
                [&key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((key, source, url, fetched, sha256, bytes, path)) = row else {
            return Ok(None);
        };
        let sha256: [u8; 32] = sha256.try_into().map_err(|_| StoreError::CorruptBlob(key.clone()))?;
        let attribution = attribution_for_source(&source);
        let mut compressed = Vec::new();
        std::fs::File::open(inner.blobs.join(&path))?.read_to_end(&mut compressed)?;
        let raw = zstd::decode_all(&compressed[..]).map_err(|_| StoreError::CorruptBlob(key))?;
        if raw.len() != bytes as usize {
            return Err(StoreError::CorruptBlob(path));
        }
        Ok(Some(StoredPayload { bytes: raw, prov: Provenance::new(source, url, fetched as u64, sha256, attribution) }))
    }

    /// Compress (zstd level 9) and store `bytes` for `key`. Content-addressing dedupes: if the blob for
    /// `prov.sha256` already exists, nothing new is written.
    pub fn put(&self, key: &FetchKey, prov: &Provenance, bytes: &[u8]) -> Result<()> {
        let key = key.as_str();
        let inner = self.inner.lock().map_err(|_| StoreError::Poisoned)?;
        let path = RawStore::blob_path(&inner.blobs, &prov.sha256);
        if !path.exists() {
            let blob = zstd::bulk::compress(bytes, 9)?;
            std::fs::File::create(&path)?.write_all(&blob)?;
        }
        inner.conn.execute(
            "INSERT INTO payloads (key, source, url, fetched, sha256, bytes, path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(key) DO UPDATE SET source = excluded.source, url = excluded.url, fetched = excluded.fetched,
             sha256 = excluded.sha256, bytes = excluded.bytes, path = excluded.path",
            rusqlite::params![
                &key,
                &prov.source,
                &prov.url,
                prov.fetched as i64,
                &prov.sha256[..],
                bytes.len() as i64,
                path.file_name().and_then(|n| n.to_str()).unwrap_or("")
            ],
        )?;
        Ok(())
    }

    /// Is the cached payload for `key` stale under `policy` at `now` (unix secs) while its period
    /// is open? A missing key counts as stale.
    pub fn stale(&self, key: &FetchKey, policy: &Policy, now: u64) -> Result<bool> {
        self.stale_ttl(key, Some(policy.ttl_open_secs), now)
    }

    /// Same, but the period is closed (final data): the payload lives `ttl_closed_secs`, forever when `None`.
    pub fn stale_closed(&self, key: &FetchKey, policy: &Policy, now: u64) -> Result<bool> {
        self.stale_ttl(key, policy.ttl_closed_secs, now)
    }

    /// Test seam: rewrite a stored row's fetch time (age it past any TTL).
    #[cfg(test)]
    pub fn set_fetched_for_test(&self, key: &FetchKey, fetched: u64) -> Result<()> {
        let inner = self.inner.lock().map_err(|_| StoreError::Poisoned)?;
        inner
            .conn
            .execute("UPDATE payloads SET fetched=?1 WHERE key=?2", rusqlite::params![fetched as i64, key.as_str()])
            .map_err(StoreError::Sqlite)?;
        Ok(())
    }

    fn stale_ttl(&self, key: &FetchKey, ttl_secs: Option<u64>, now: u64) -> Result<bool> {
        let key = key.as_str();
        let inner = self.inner.lock().map_err(|_| StoreError::Poisoned)?;
        let row = inner
            .conn
            .query_row("SELECT fetched FROM payloads WHERE key = ?1", [&key], |row| row.get::<_, i64>(0))
            .optional();
        match row {
            Err(e) => Err(e.into()),
            Ok(None) => Ok(true),
            Ok(Some(fetched)) => match ttl_secs {
                None => Ok(false),
                Some(ttl) => Ok(now.saturating_sub(fetched as u64) > ttl),
            },
        }
    }
}

/// Attribution comes from the stored `source` column at read time.
fn attribution_for_source(source: &str) -> &'static str {
    match source {
        "statsapi" => crate::provenance::Attribution::STATS_API,
        "cots" => crate::provenance::Attribution::COTS,
        "retrosheet" => crate::provenance::Attribution::RETROSHEET,
        "lahman" => crate::provenance::Attribution::LAHMAN,
        "chadwick" => crate::provenance::Attribution::CHADWICK,
        _ => crate::provenance::Attribution::SAVANT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{FetchKey, Role};
    use crate::provenance::Attribution;
    use std::io::Read;

    fn player_key() -> FetchKey {
        FetchKey::StatcastPlayerSeason { mlbam: 660271, role: Role::Batter, season: 2025 }
    }

    fn prov_for(bytes: &[u8]) -> Provenance {
        // sha256 computed by the (Wave 3) fetcher; here any 32 distinct bytes per payload do for testing.
        let mut sha = [0u8; 32];
        sha[0] = bytes.len() as u8;
        Provenance::new("savant", "https://x", 100, sha, Attribution::SAVANT)
    }

    #[test]
    fn put_get_round_trips_through_zstd() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::open(dir.path()).unwrap();
        let payload = b"statcast csv bytes".repeat(100);
        let prov = prov_for(&payload);
        store.put(&player_key(), &prov, &payload).unwrap();
        let got = store.get(&player_key()).unwrap().unwrap();
        assert_eq!(got.bytes, payload);
        assert_eq!(got.prov, prov);
        // The stored blob is smaller than the raw payload for repetitive bytes.
        let zst = RawStore::blob_path(dir.path().join("blobs").as_path(), &prov.sha256);
        assert!(zst.exists());
        assert!(file_len(&zst) < payload.len());
    }

    #[test]
    fn get_of_unknown_key_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::open(dir.path()).unwrap();
        assert!(store.get(&player_key()).unwrap().is_none());
    }

    #[test]
    fn content_addressing_dedupes_identical_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::open(dir.path()).unwrap();
        let bytes = b"identical payload".to_vec();
        let prov = prov_for(&bytes);
        store.put(&player_key(), &prov, &bytes).unwrap();
        store.put(&FetchKey::Draft { year: 2025 }, &prov, &bytes).unwrap();
        let blobs = std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
        assert_eq!(blobs, 1, "identical sha256 must map to one blob");
        store.put(&FetchKey::Draft { year: 2024 }, &prov, &bytes).unwrap();
        assert_eq!(store.get(&FetchKey::Draft { year: 2024 }).unwrap().unwrap().bytes, bytes);
    }

    #[test]
    fn staleness_open_vs_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::open(dir.path()).unwrap();
        let policy = Policy {
            min_interval_secs: 2,
            ttl_open_secs: 300,
            ttl_closed_secs: Some(43_200),
            max_per_command: 1,
            attribution: Attribution::SAVANT,
        };
        let now = 1_800_000_000;
        let bytes = b"".to_vec();
        let prov = Provenance::new("statsapi", "https://x", now - 400, [9; 32], Attribution::STATS_API);
        store.put(&player_key(), &prov, &bytes).unwrap();
        assert!(store.stale(&player_key(), &policy, now).unwrap(), "age 400 > open TTL 300 is stale");
        // Closed TTL (43_200) keeps the same payload fresh for the whole year.
        let closed = Policy { ttl_closed_secs: Some(43_200), ..policy.clone() };
        assert!(!store.stale_closed(&player_key(), &closed, now).unwrap());
        // ttl_closed_secs None (per-source defaults) is never stale.
        assert!(!store.stale_closed(&player_key(), &Policy::statsapi(), now).unwrap());
        // With a short closed TTL the same payload goes stale.
        let expiring = Policy { ttl_closed_secs: Some(100), ..closed };
        assert!(store.stale_closed(&player_key(), &expiring, now).unwrap());
        // Missing keys are stale.
        assert!(store.stale(&FetchKey::Draft { year: 2031 }, &closed, now).unwrap());
    }

    fn file_len(path: &Path) -> usize {
        let mut file = std::fs::File::open(path).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        buf.len()
    }
}
