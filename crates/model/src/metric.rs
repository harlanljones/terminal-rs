//! Metrics: id, version, counting unit, coverage intervals, Carleton reliability threshold and an
//! evaluation over a slice of events ("undefined is not an error": no defined inputs → `None`).
//!
//! The registry is a plain `static` array of references to `static` metrics: no leaks, no lazy
//! init. Coverage intervals are const-friendly `(year, month, day)` triples converted to
//! [`time::Date`] on demand.

use crate::entity::Role;
use crate::event::{Event, field};
use crate::window::Unit;
use std::fmt;
use time::{Date, Month};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MetricId(pub String);

impl MetricId {
    pub fn new(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl fmt::Display for MetricId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug)]
pub struct MetricNotFound(String);

impl fmt::Display for MetricNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown metric {:?}", self.0)
    }
}

impl std::error::Error for MetricNotFound {}

impl registry::Lookup for MetricId {
    type Err = MetricNotFound;
    fn get(self) -> Result<&'static Metric, MetricNotFound> {
        registry::registry().iter().copied().find(|m| m.id == self.0).ok_or(MetricNotFound(self.0))
    }
}

/// B3: a value is defined only inside one of its intervals; outside, undefined — never zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Coverage {
    /// `(from, to)`; `to = None` means open-ended; inclusive of `from`, exclusive of `to`.
    /// Const-friendly raw civil dates.
    pub intervals: &'static [(DateC, Option<DateC>)],
}

/// Pinned civil date, lexicographically ordered (y, m, d).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateC {
    pub y: i32,
    pub m: u8,
    pub d: u8,
}

pub const fn c(y: i32, m: u8, d: u8) -> DateC {
    DateC { y, m, d }
}

pub fn date_c(d: Date) -> DateC {
    DateC { y: d.year(), m: d.month() as u8, d: d.day() }
}

pub fn to_date(c: DateC) -> Date {
    Date::from_calendar_date(c.y, Month::try_from(c.m).unwrap(), c.d).expect("pinned interval date")
}

impl Coverage {
    /// No restriction: professional era through the open future.
    pub const ALL: Self = Self { intervals: &ALL_INTERVALS };

    /// Bat-tracking coverage per B3: 2023 second half (partial), the measured hole
    /// 2024-03-20 .. 2024-04-02, then open-ended from 2024-04-03. Swing-path metrics share it.
    pub const BAT_TRACKING: Self = Self { intervals: &BAT_TRACKING_INTERVALS };

    pub fn contains(&self, date: &Date) -> bool {
        let kc = date_c(*date);
        self.intervals.iter().any(|(from, to)| kc >= *from && to.as_ref().is_none_or(|to| kc < *to))
    }
}

static ALL_INTERVALS: [(DateC, Option<DateC>); 1] = [(c(1876, 1, 1), None)];
static BAT_TRACKING_INTERVALS: [(DateC, Option<DateC>); 2] =
    [(c(2023, 8, 15), Some(c(2024, 3, 19))), (c(2024, 4, 3), None)];

#[derive(Clone, Copy, Debug)]
pub struct Metric {
    pub id: &'static str,
    pub version: u32,
    pub role: Role,
    pub unit: Unit,
    pub coverage: Coverage,
    /// Carleton table (DR-3.35): shrinkage denominator `x`, counted in the given unit.
    pub threshold: Option<(Unit, u32)>,
    /// Evaluation over a window of events; `None` when no defined inputs.
    pub eval: fn(&[Event]) -> Option<f64>,
}

/// Mean of defined values of field `idx` over events incrementing `unit`.
fn mean_of(events: &[Event], unit: Unit, idx: u16) -> Option<f64> {
    let mut n = 0u32;
    let mut sum = 0f64;
    for e in events {
        if e.unit_flags.counts(unit)
            && let Some(v) = e.field(idx)
        {
            n += 1;
            sum += v;
        }
    }
    (n > 0).then_some(sum / n as f64)
}

pub mod registry {
    //! Spec 4.3 v1 metric set. Carleton thresholds only where published:
    //! K% 60 PA, BB% 120 PA, ISO 160 AB, OBP 460 PA, AVG 910 AB, BABIP 820 BIP.
    use super::*;

    macro_rules! m {
        ($name:ident, $id:literal, $role:ident, $unit:ident, $cov:expr, $thr:expr, $field:ident) => {
            pub static $name: Metric = Metric {
                id: $id,
                version: 1,
                role: Role::$role,
                unit: Unit::$unit,
                coverage: $cov,
                threshold: $thr,
                eval: |ev| mean_of(ev, Unit::$unit, field::$field),
            };
        };
    }

    const NO_THR: Option<(Unit, u32)> = None;

    // ---- hitters ----
    m!(BAT_WOBA, "bat:woba", Bat, Pa, Coverage::ALL, NO_THR, WOBA_VALUE);
    m!(BAT_XWOBA, "bat:xwoba", Bat, Pa, Coverage::BAT_TRACKING, NO_THR, ESTIMATED_WOBA);
    m!(BAT_EV, "bat:ev", Bat, Pa, Coverage::BAT_TRACKING, NO_THR, LAUNCH_SPEED);
    m!(BAT_LAUNCH_ANGLE, "bat:launch_angle", Bat, Pa, Coverage::BAT_TRACKING, NO_THR, LAUNCH_ANGLE);
    m!(BAT_BARREL_RATE, "bat:barrel_rate", Bat, Bip, Coverage::BAT_TRACKING, NO_THR, BARREL_FLAG);
    m!(BAT_HARD_HIT_RATE, "bat:hard_hit_rate", Bat, Bip, Coverage::BAT_TRACKING, NO_THR, HARD_HIT_FLAG);
    m!(BAT_K_RATE, "bat:k_rate", Bat, Pa, Coverage::ALL, Some((Unit::Pa, 60)), K_FLAG);
    m!(BAT_BB_RATE, "bat:bb_rate", Bat, Pa, Coverage::ALL, Some((Unit::Pa, 120)), BB_FLAG);
    m!(BAT_WHIFF_RATE, "bat:whiff_rate", Bat, Pitch, Coverage::ALL, NO_THR, WHIFF_FLAG);
    m!(BAT_CHASE_RATE, "bat:chase_rate", Bat, Pitch, Coverage::ALL, NO_THR, OOB_FLAG);
    m!(BAT_SPEED, "bat:bat_speed", Bat, Pitch, Coverage::BAT_TRACKING, NO_THR, BAT_SPEED);
    m!(SWING_LENGTH, "bat:swing_length", Bat, Pitch, Coverage::BAT_TRACKING, NO_THR, SWING_LENGTH);
    m!(ATTACK_ANGLE, "bat:attack_angle", Bat, Pitch, Coverage::BAT_TRACKING, NO_THR, ATTACK_ANGLE);
    m!(ATTACK_DIRECTION, "bat:attack_direction", Bat, Pitch, Coverage::BAT_TRACKING, NO_THR, ATTACK_DIRECTION);
    m!(SWING_PATH_TILT, "bat:swing_path_tilt", Bat, Pitch, Coverage::BAT_TRACKING, NO_THR, SWING_PATH_TILT);
    m!(AVG, "bat:avg", Bat, Ab, Coverage::ALL, Some((Unit::Ab, 910)), WOBA_VALUE);
    m!(OBP, "bat:obp", Bat, Pa, Coverage::ALL, Some((Unit::Pa, 460)), WOBA_VALUE);
    m!(ISO, "bat:iso", Bat, Ab, Coverage::ALL, Some((Unit::Ab, 160)), WOBA_VALUE);
    m!(BABIP, "bat:babip", Bat, Bip, Coverage::ALL, Some((Unit::Bip, 820)), WOBA_VALUE);

    // ---- pitchers ----
    m!(PIT_VELOCITY, "pit:velocity", Pit, Pitch, Coverage::ALL, NO_THR, RELEASE_SPEED);
    m!(PIT_SPIN_RATE, "pit:spin_rate", Pit, Pitch, Coverage::ALL, NO_THR, SPIN_RATE);
    m!(PIT_EXTENSION, "pit:extension", Pit, Pitch, Coverage::ALL, NO_THR, EXTENSION);
    m!(PIT_WHIFF_RATE, "pit:whiff_rate", Pit, Pitch, Coverage::ALL, NO_THR, WHIFF_FLAG);
    m!(PIT_K_RATE, "pit:k_rate", Pit, Bf, Coverage::ALL, NO_THR, K_FLAG);
    m!(PIT_BB_RATE, "pit:bb_rate", Pit, Bf, Coverage::ALL, NO_THR, BB_FLAG);
    m!(PIT_XWOBA_AGAINST, "pit:xwoba_against", Pit, Bf, Coverage::BAT_TRACKING, NO_THR, ESTIMATED_WOBA);

    #[rustfmt::skip]
    pub static REGISTRY: [&Metric; 26] = [
        &BAT_WOBA,
        &BAT_XWOBA,
        &BAT_EV,
        &BAT_LAUNCH_ANGLE,
        &BAT_BARREL_RATE,
        &BAT_HARD_HIT_RATE,
        &BAT_K_RATE,
        &BAT_BB_RATE,
        &BAT_WHIFF_RATE,
        &BAT_CHASE_RATE,
        &BAT_SPEED,
        &SWING_LENGTH,
        &ATTACK_ANGLE,
        &ATTACK_DIRECTION,
        &SWING_PATH_TILT,
        &AVG,
        &OBP,
        &ISO,
        &BABIP,
        &PIT_VELOCITY,
        &PIT_SPIN_RATE,
        &PIT_EXTENSION,
        &PIT_WHIFF_RATE,
        &PIT_K_RATE,
        &PIT_BB_RATE,
        &PIT_XWOBA_AGAINST,
    ];

    /// The spec 4.3 v1 metric set.
    pub const fn registry() -> &'static [&'static Metric] {
        &REGISTRY
    }

    pub trait Lookup {
        type Err;
        fn get(self) -> Result<&'static Metric, Self::Err>;
    }

    pub fn by_id(id: &str) -> Option<&'static Metric> {
        registry().iter().copied().find(|m| m.id == id)
    }

    impl Lookup for &str {
        type Err = MetricNotFound;
        fn get(self) -> Result<&'static Metric, MetricNotFound> {
            by_id(self).ok_or_else(|| MetricNotFound(self.to_owned()))
        }
    }
}

pub use registry::registry;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bat_tracking_coverage_has_the_measured_hole() {
        let cov = Coverage::BAT_TRACKING;
        assert!(cov.contains(&to_date(c(2023, 8, 15))));
        assert!(cov.contains(&to_date(c(2024, 3, 18))));
        assert!(!cov.contains(&to_date(c(2024, 3, 20)))); // hole
        assert!(!cov.contains(&to_date(c(2024, 4, 2))));
        assert!(cov.contains(&to_date(c(2024, 4, 3))));
        assert!(cov.contains(&to_date(c(2026, 10, 21))));
        assert!(!cov.contains(&to_date(c(2022, 6, 1)))); // before the 2023 second half
    }
}
