//! # sabr-forecast — walk-forward forecasting (spec §4.4)
//!
//! Batting Marcel (DR-3.19/DR-3.34), naive baseline (DR-3.36 as the playing-time model's control),
//! the walk-forward harness (preseason plus, when the corpus carries cut lines, in-season folds —
//! see below), the empirical interval model
//! (DR-3.21/DR-3.21a), the scores and calibration gate (DR-3.12/DR-3.12a), and the JSON artifact
//! plus SQLite index (DR-3.22).
//!
//! ## Deviations and limitations of this v1 (documented per assignment)
//!
//! * **In-season protocol (DR-3.31).** The bundled `cut-lines-2001-2025.csv` carries real cut
//!   lines: when the corpus is loaded with [`Corpus::from_lahman_with_cuts`], the harness scores
//!   the May–September month-start folds in season through
//!   [`crate::marcel::Model::project_in_season`]. A corpus without cut lines evaluates
//!   preseason-only.
//! * **Rest-of-season (ROS).** Marcel's in-season form is implemented — the current partial season
//!   enters as a fifth (weight-5) term and projected remaining PA is the full projection less PA
//!   to date, floored at 0 — generalised over a partials map
//!   ([`crate::marcel::project_marcel_in_season`]) for the in-season folds; the single-partial
//!   ROS form delegates to it with a one-entry map.
//! * **wOBA weights.** Per-season weights from the Retrosheet constants tool are not available in
//!   this ticket; a single PROTOTYPE constant set (FanGraphs-style) is used unless a constants
//!   file is supplied. See [`constants`].
//! * **Pitching (pitching Marcel, this ticket).** `PitchingMarcel` (`marcel.pitching/1.0.0`)
//!   implements spec §4.4: seasons weighted 3/2/1 on innings, per-9 rate components SO/BB/HR/H/ER
//!   regressed toward the same-window league rates with +60 IP (starters; a season is a starter
//!   iff `2·GS ≥ G`, role = latest season with G > 0 ≤ as-of) / +25 IP (relievers), the batting
//!   age adjustment applied multiplicative, ERA projected from ER, reliability = own share of the
//!   regressed IP denominator. Documented deviations: the playing-time formula is
//!   `0.5·IP₋₁ + 0.1·IP₋₂ + 50` flat (spec gives no pitcher IP formula); the ROS partial season
//!   carries the flat weight 3. The golden fixture check (`golden_pitching`) is a **diagnosis
//!   report**, not a gate: the committed `PitchingMarcel.csv` implies a ~268-IP role-independent
//!   reliability scale and a BaseRuns-style ER composite (see its module doc), which the spec's
//!   60/25 constants do not reproduce. `walk_forward_pitching` scores ERA (ER variant; the corpus
//!   carries ER) and IP MAE through the same interval/gate machinery.
//! * **Interval calibration (DR-3.21/DR-3.21a, amended).** The interval population is
//!   **rate-conditional**: rows without an observed rate (actual weight 0, e.g. defensive-only
//!   lines) and rows whose actual was floor-substituted (projected player with no season line) are
//!   convention rows rather than rate draws — no quantile forecast can cover them at any target
//!   level — so they are excluded from both the residual history and the coverage cells; floor
//!   rows (no projection, observed rate) stay in under bucket None with their residuals in the
//!   history, so the None cell and its history are the same population. The residual weight basis
//!   is the **same variable on the fit and the de-standardisation side** (the DR's actual-PA-fit /
//!   projected-PA-forecast split is what made the bands miscalibrated): per role, the stationary
//!   basis is the **actual PA for batting** (a rate's binomial-like noise scales with the PA that
//!   generated it) and the **projected IP for pitching** (projected IP is near-deterministic given
//!   the role, and actual IP is the noisy quantity). The residual history is a **rolling window**
//!   (4 scored seasons batting, 6 pitching): bucket-conditional centred errors drift season to
//!   season, and pooling all earlier folds overstates recent seasons' spread. Empirical quantiles
//!   commute with re-centring, so no bucket-level bias correction can change coverage; the
//!   population-wide re-centring stays as scored (DR-3.12).
//!
//! Everything here is pure and ordered: corpus rows are sorted by (player_id, season), folds run
//! in ascending season order, quantiles use linear interpolation over sorted residuals, and all
//! hashing is over canonical serialisations.

pub mod artifact;
pub mod constants;
pub mod corpus;
pub mod harness;
pub mod intervals;
pub mod marcel;
pub mod scores;

#[cfg(test)]
pub mod golden_marcel;

#[cfg(test)]
pub mod golden_pitching;

#[cfg(test)]
pub mod harness_tests;

pub use artifact::{Artifact, ArtifactIndex, write_artifact};
pub use constants::Weights;
pub use corpus::{BattingLine, Corpus, PitchingLine, Protocol};
pub use harness::{
    Fold, FoldRow, PitchingFold, PitchingFoldRow, Regime, fit_intervals_for, fit_pitching_intervals_for, walk_forward,
    walk_forward_pitching,
};
pub use intervals::{Bucket, IntervalModel};
pub use marcel::{Forecast, Marcel, Model, PitchingForecast, PitchingMarcel, PitchingModel, PriorSeason};
pub use scores::{GateResult, ScoreReport, actual_woba};
