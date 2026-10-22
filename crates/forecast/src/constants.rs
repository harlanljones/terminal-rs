//! wOBA linear weights (spec §4.4 "Park factors and constants", DR-3.12).
//!
//! v1: per-season weights from the Retrosheet run-expectancy constants tool are not available in
//! this ticket, so unless a constants file is supplied at the given path a single **PROTOTYPE**
//! constant set is used for every season — the FanGraphs-style weights. These are a stand-in, not
//! a computed artefact; they will be replaced by the constants file when the retro ticket lands.
//! No FanGraphs assets are used anywhere else.

use std::collections::BTreeMap;
use std::path::Path;

/// Linear weights for one season: wOBA contribution of (uBB, HBP, 1B, 2B, 3B, HR).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Unintentional walk (BB − IBB).
    pub bb: f64,
    pub hbp: f64,
    pub single: f64,
    pub double: f64,
    pub triple: f64,
    pub home_run: f64,
}

/// PROTOTYPE weight set (FanGraphs-style), used for every season until the constants file lands.
const PROTOTYPE: Weights = Weights { bb: 0.69, hbp: 0.72, single: 0.89, double: 1.27, triple: 1.57, home_run: 2.08 };

/// Per-season weight table. `Map<u16, Weights>` keyed by season; lookups fall back to the
/// PROTOTYPE set for seasons the table does not cover.
#[derive(Debug, Clone, PartialEq)]
pub struct Constants {
    pub weights: BTreeMap<u16, Weights>,
    /// SHA-256 of the constants file this table was loaded from, if any.
    pub source_sha256: Option<String>,
}

impl Default for Constants {
    fn default() -> Self {
        Self::prototype()
    }
}

impl Constants {
    /// The PROTOTYPE table: one weight set for all seasons, no constants file.
    pub fn prototype() -> Constants {
        Constants { weights: BTreeMap::new(), source_sha256: None }
    }

    /// Load per-season weights from a constants file, if present. File format: one CSV header
    /// `season,bb,hbp,1b,2b,3b,hr` per line. Returns [`Constants::prototype`] when the file is
    /// absent (documented fallback).
    pub fn load(path: &Path) -> std::io::Result<Constants> {
        let raw = match std::fs::read_to_string(path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Constants::prototype()),
            Err(e) => return Err(e),
        };
        let hex = {
            use sha2::{Digest, Sha256};
            let d = Sha256::digest(raw.as_bytes());
            d.iter().map(|x| format!("{x:02x}")).collect::<String>()
        };
        let mut weights = BTreeMap::new();
        for line in raw.strip_prefix('\u{feff}').unwrap_or(&raw).lines().skip(1) {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(',').collect();
            if f.len() != 7 {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("constants line: {line:?}")));
            }
            let num =
                |i: usize| -> f64 { f[i].trim().parse().unwrap_or_else(|_| panic!("constants: bad f64 in {line:?}")) };
            weights.insert(
                f[0].trim().parse().unwrap_or_else(|_| panic!("constants: bad season in {line:?}")),
                Weights { bb: num(1), hbp: num(2), single: num(3), double: num(4), triple: num(5), home_run: num(6) },
            );
        }
        Ok(Constants { weights, source_sha256: Some(hex) })
    }

    /// Weights for a season; falls back to the PROTOTYPE set.
    pub fn for_season(&self, season: u16) -> Weights {
        self.weights.get(&season).copied().unwrap_or(PROTOTYPE)
    }
}
