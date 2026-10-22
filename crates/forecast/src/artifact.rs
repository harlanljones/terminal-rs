//! The model artifact and registry (DR-3.22): one JSON document per fit, file name = content
//! hash of (model id, version, corpus rev, protocol hash, as_of), plus a SQLite index table using
//! sabr-data's connection-in-Mutex pattern (the same shape as `RawStore`, sync-only, no lock
//! across an await — this crate is sync).

use crate::corpus::Corpus;
use crate::scores::ScoreReport;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

/// A fitted artifact, serialisable to the on-disk JSON.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Artifact {
    /// Model id + semantic version, e.g. `marcel.batting/1.0.0`.
    pub model: String,
    /// As-of date label (`YYYY-MM-DD` or `YYYY-preseason`).
    pub as_of: String,
    /// Corpus revision: tuple of input file SHA-256s, hex, in fixed order.
    pub corpus_rev: [String; 3],
    /// Protocol hash (canonical serialisation of seasons + cut lines).
    pub protocol_hash: String,
    /// Constants file hash when a constants file supplied the weights, else null (PROTOTYPE).
    pub constants_hash: Option<String>,
    pub scores: ScoreReport,
    /// Per-player projections with de-standardised quantiles and bucket, for the final fit.
    pub forecasts: Vec<ForecastRow>,
    /// The final fitted interval model for the artifact's forecast regime (DR-3.40 wiring),
    /// replayed from the walk-forward's last-regime residual history: the 01-01 regime for
    /// next-season projections, the closest in-season cut's own history for rest-of-season
    /// as-of-today. `None` during interval warm-up (fewer than 3 scored seasons of history)
    /// — projections then carry point forecasts only. Optional on read so artifacts written
    /// before the field loading (pre-DR-3.40) still deserialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intervals: Option<crate::intervals::IntervalModel>,
    /// Creation time (UTC, RFC 3339) — informational, excluded from the content hash.
    pub created: String,
}

/// One player's row in the artifact.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ForecastRow {
    pub player_id: String,
    pub season: u16,
    pub woba: f64,
    pub pa: f64,
    pub reliability: f64,
    pub bucket: crate::intervals::Bucket,
    /// De-standardised error quantiles (added to the projection for a band).
    pub q05: f64,
    pub q10: f64,
    pub q25: f64,
    pub q50: f64,
    pub q75: f64,
    pub q90: f64,
    pub q95: f64,
}

/// The content-hash key: SHA-256 over the canonical tuple (model, version, corpus_rev,
/// protocol_hash, as_of). Splitting the model id on `/` yields name and version.
pub fn content_hash(artifact: &Artifact) -> String {
    let (name, version) = artifact.model.split_once('/').unwrap_or((artifact.model.as_str(), "0.0.0"));
    let mut h = Sha256::new();
    h.update(name.as_bytes());
    h.update([0]);
    h.update(version.as_bytes());
    h.update([0]);
    for s in &artifact.corpus_rev {
        h.update(s.as_bytes());
        h.update([0]);
    }
    h.update(artifact.protocol_hash.as_bytes());
    h.update([0]);
    h.update(artifact.as_of.as_bytes());
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// Write the artifact JSON to `dir/<content-hash>.json`; returns the path written.
pub fn write_artifact(artifact: &Artifact, dir: &Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = format!("{}.json", content_hash(artifact));
    let path = dir.join(name);
    let json =
        serde_json::to_string_pretty(artifact).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// The registry index: SQLite table mapping (model, as_of) → artifact path and scores, following
/// sabr-data's `RawStore` connection-in-Mutex pattern. Sync only.
pub struct ArtifactIndex {
    inner: Mutex<Connection>,
}

impl ArtifactIndex {
    /// Open (creating if needed) the index at `path` (a SQLite file).
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS artifacts (
                model TEXT NOT NULL,
                as_of TEXT NOT NULL,
                path TEXT NOT NULL,
                scores_json TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                PRIMARY KEY (model, as_of)
            )",
        )?;
        Ok(ArtifactIndex { inner: Mutex::new(conn) })
    }

    /// Insert or update the row for `(model, as_of)`.
    pub fn upsert(&self, artifact: &Artifact, path: &Path) -> Result<(), rusqlite::Error> {
        let scores_json = serde_json::to_string(&artifact.scores)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let conn = self.inner.lock().map_err(|_| rusqlite::Error::ToSqlConversionFailure("poisoned".into()))?;
        conn.execute(
            "INSERT INTO artifacts (model, as_of, path, scores_json, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (model, as_of) DO UPDATE SET path = ?3, scores_json = ?4, content_hash = ?5",
            rusqlite::params![
                artifact.model,
                artifact.as_of,
                path.display().to_string(),
                scores_json,
                crate::artifact::content_hash(artifact)
            ],
        )?;
        Ok(())
    }

    /// The newest row for one model by as_of label (descending string order, so
    /// `2026-06-01` sorts after `2026-preseason`); `None` when the model was never fitted.
    pub fn latest(&self, model: &str) -> Result<Option<IndexRow>, rusqlite::Error> {
        let conn = self.inner.lock().map_err(|_| rusqlite::Error::ToSqlConversionFailure("poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT as_of, path, scores_json, content_hash FROM artifacts WHERE model = ?1 ORDER BY as_of DESC LIMIT 1",
        )?;
        let row = stmt
            .query_row(rusqlite::params![model], |r| {
                Ok(IndexRow {
                    as_of: r.get(0)?,
                    model: model.to_string(),
                    path: r.get(1)?,
                    scores_json: r.get(2)?,
                    content_hash: r.get(3)?,
                })
            })
            .optional()?;
        Ok(row)
    }
}

/// One row of the artifact registry, as read back by [`ArtifactIndex::latest`].
#[derive(Debug, Clone, PartialEq)]
pub struct IndexRow {
    pub model: String,
    pub as_of: String,
    pub path: String,
    pub scores_json: String,
    pub content_hash: String,
}

/// Convenience: corpus revision from a loaded [`Corpus`].
pub fn corpus_rev(corpus: &Corpus) -> [String; 3] {
    corpus.input_sha256.clone()
}
