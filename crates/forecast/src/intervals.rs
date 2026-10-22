//! The empirical interval model (DR-3.21, DR-3.21a).
//!
//! Residuals from **strictly-earlier** walk-forward folds (err = actual − recentred projected),
//! standardised as `z = err · √w` where `w` is the fold's declared weight basis
//! ([`WeightBasis`]): the **same** weight standardises the history and de-standardises the
//! forecast (`q / √w`), so fit and forecast can never disagree about the scale. Empirically the
//! stationary basis is the actual PA for batting and the projected IP for pitching (see the crate
//! deviations). The interval population is **rate-conditional**: rows without an observed rate
//! (actual weight 0) and rows whose actual was floor-substituted (projected player with no season
//! line) are convention rows, not rate draws, and are excluded from both the residual history and
//! the coverage cells; floor rows (no projection, observed rate) stay in, under bucket None.
//! Quantiles per Marcel reliability bucket at p05/p10/p25/p50/p75/p90/p95 with linear
//! interpolation. Buckets below `n_min` (default 50 residuals) merge upward into the next bucket
//! (Low → Med → High → None absorbs everything). Warm-up: seasons with no residual history
//! produce empty intervals (point forecasts only).

/// Which weight standardises the interval residuals, declared per fold so the fit and the
/// de-standardisation always use the same scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeightBasis {
    /// The projection's own weight (projected PA / IP). Pitching: projected IP is stable and
    /// nearly deterministic given the role, so it is the stationary scale.
    Projected,
    /// The actual weight of the observed rate (actual PA). Batting: the binomial-like noise of a
    /// wOBA rate scales with the PA that generated it, so the actual PA is the
    /// variance-stabilising scale.
    Actual,
    /// The **future sample** of an in-season cut fold: the PA still to be played after the cut
    /// (`actual_pa − pa_to_date`). Used symmetrically with the de-standardisation at scoring
    /// time; on a preseason fold there is no future sample, so a Future basis there fails loudly.
    Future,
}

/// Reliability buckets at the fixed thresholds (High ≥ 0.7, Med ≥ 0.45, Low ≥ 0.15, else None).
/// Ordering is the upward-merge direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    High,
    Med,
    Low,
    None,
}

impl Bucket {
    pub const fn of(reliability: f64) -> Bucket {
        if reliability >= 0.7 {
            Bucket::High
        } else if reliability >= 0.45 {
            Bucket::Med
        } else if reliability >= 0.15 {
            Bucket::Low
        } else {
            Bucket::None
        }
    }

    pub const ALL: [Bucket; 4] = [Bucket::High, Bucket::Med, Bucket::Low, Bucket::None];
}

/// Default minimum residuals per bucket before merging upward.
pub const N_MIN: usize = 50;

/// Quantiles carried per bucket.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Quantiles {
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p05: f64,
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p10: f64,
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p25: f64,
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p50: f64,
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p75: f64,
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p90: f64,
    #[serde(default, serialize_with = "crate::intervals::ser_r6", deserialize_with = "crate::intervals::de_r6")]
    pub p95: f64,
}

impl PartialEq for Quantiles {
    /// Compare on the 6-decimal grid the artifact stores: the JSON registry is the contract
    /// (DR-3.22), and the model's own precision is far below 1e-6.
    fn eq(&self, other: &Self) -> bool {
        [
            (self.p05, other.p05),
            (self.p10, other.p10),
            (self.p25, other.p25),
            (self.p50, other.p50),
            (self.p75, other.p75),
            (self.p90, other.p90),
            (self.p95, other.p95),
        ]
        .iter()
        .all(|(a, b)| (a * 1e6).round() == (b * 1e6).round())
    }
}

/// Serialize a quantile at 6-decimal rounding: the JSON artifact is a human-facing registry
/// record (DR-3.22) and the interval model's own precision is far below 1e-6 (quantile
/// interpolation on O(1e3) residuals). Rounding on write makes the artifact round-trip
/// exactly — `stored.intervals == artifact.intervals` — instead of failing on last-bit
/// float noise through serde_json's shortest-repr round trip.
pub(crate) fn ser_r6<S: serde::Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64((v * 1e6).round() / 1e6)
}

/// Read back a rounded quantile and normalise it to the same 6-decimal grid the writer used,
/// so `stored == fitted` holds despite float repr noise.
pub(crate) fn de_r6<'d, D: serde::Deserializer<'d>>(d: D) -> Result<f64, D::Error> {
    let v: f64 = serde::de::Deserialize::deserialize(d)?;
    Ok((v * 1e6).round() / 1e6)
}

/// The quantile levels the model fits, in order.
pub const LEVELS: [f64; 7] = [0.05, 0.10, 0.25, 0.50, 0.75, 0.90, 0.95];

/// One standardised residual observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Residual {
    pub z: f64,
    pub bucket: Bucket,
}

/// Quantiles of sorted data with linear interpolation (`R` type 7). Empty slice → None.
pub fn quantile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let h = (n as f64 - 1.0) * p;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    Some(sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo]))
}

/// Fit per-bucket standardised-residual quantiles from strictly-earlier folds' residuals,
/// with upward merge below [`N_MIN`]. Serializable so fitted artifacts (DR-3.22) carry the
/// model for forecast-time de-standardisation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntervalModel {
    /// Per bucket, after merge; `None` when the bucket has no residuals at all.
    pub by_bucket: Vec<(Bucket, Quantiles)>,
    pub n_min: usize,
}

impl IntervalModel {
    /// Fit from residuals. `residuals` may be empty (warm-up) → empty model.
    pub fn fit(residuals: &[Residual], n_min: usize) -> IntervalModel {
        // Deterministic order: sort each bucket's z values ascending.
        let mut by: std::collections::BTreeMap<Bucket, Vec<f64>> = std::collections::BTreeMap::new();
        for r in residuals {
            by.entry(r.bucket).or_default().push(r.z);
        }
        for zs in by.values_mut() {
            zs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Upward merge: a bucket below n_min absorbs into the next bucket in ALL order
        // (High → Med → Low → None absorbs everything). Iterate from the top of the merge chain.
        let merged = merge_up(&by, n_min);

        let mut by_bucket = Vec::new();
        for b in Bucket::ALL {
            let Some(zs) = merged.get(&b) else { continue };
            if zs.is_empty() {
                continue;
            }
            let get = |p: f64| quantile(zs, p).unwrap_or_default();
            by_bucket.push((
                b,
                Quantiles {
                    p05: get(0.05),
                    p10: get(0.10),
                    p25: get(0.25),
                    p50: get(0.50),
                    p75: get(0.75),
                    p90: get(0.90),
                    p95: get(0.95),
                },
            ));
        }
        IntervalModel { by_bucket, n_min }
    }

    /// Quantiles for a bucket; empty buckets inherit the nearest fitted bucket upward, else None.
    pub fn quantiles(&self, bucket: Bucket) -> Option<&Quantiles> {
        self.by_bucket.iter().find(|(b, _)| *b == bucket).map(|(_, q)| q)
    }

    /// De-standardise at forecast time: `q / √w` where `w` is the row's weight on the fold's
    /// declared [`WeightBasis`] — the same basis the residual history was standardised with.
    pub fn destandardise(q: &Quantiles, weight: f64) -> Quantiles {
        let s = weight.max(1.0).sqrt();
        Quantiles {
            p05: q.p05 / s,
            p10: q.p10 / s,
            p25: q.p25 / s,
            p50: q.p50 / s,
            p75: q.p75 / s,
            p90: q.p90 / s,
            p95: q.p95 / s,
        }
    }
}

/// Merge-up pass. Buckets are processed High→None; each under-populated bucket's residuals move
/// into the next bucket in [`Bucket::ALL`] order and counting repeats until stable.
fn merge_up(
    by: &std::collections::BTreeMap<Bucket, Vec<f64>>,
    n_min: usize,
) -> std::collections::BTreeMap<Bucket, Vec<f64>> {
    let mut m: std::collections::BTreeMap<Bucket, Vec<f64>> = by.iter().map(|(b, v)| (*b, v.clone())).collect();
    for i in 0..Bucket::ALL.len() - 1 {
        let b = Bucket::ALL[i];
        let next = Bucket::ALL[i + 1];
        let under = m.get(&b).is_some_and(|v| !v.is_empty() && v.len() < n_min);
        if under {
            let v = m.remove(&b).unwrap_or_default();
            m.entry(next).or_default().extend(v);
        }
    }
    // A fully-populated bucket at the chain start stays; empty buckets are simply absent.
    m.retain(|_, v| !v.is_empty());
    m
}
