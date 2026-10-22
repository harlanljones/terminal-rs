//! Derived series: pure, backward-looking derivation over combined event logs. Windows may cross
//! season boundaries; a point's value is undefined (never zero, never NaN) when its window has no
//! defined metric inputs or the point date lies outside the metric's coverage. Metrics also treat
//! out-of-coverage events as undefined *inputs*: coverage intervals mark where the inputs exist.

use crate::entity::Entity;
use crate::event::{Event, EventLog};
use crate::metric::{Metric, MetricId};
use crate::window::{Unit, Window, count_unit};
use std::str::FromStr;
use time::Date;

/// One derived value with its reliability label (DR-3.35).
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub season: u16,
    pub seq: u64,
    pub date: Date,
    /// `None` = undefined (gap), never zero.
    pub value: Option<f64>,
    /// Count of the metric's unit as defined in the point's window.
    pub n: u32,
    /// `n /(n + x)` from the metric's threshold; `None` = "no threshold".
    pub weight: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub entity: Entity,
    pub metric: MetricId,
    pub window: Window,
    pub as_of: Date,
    pub points: Vec<Point>,
}

/// Reliability weight = `n /(n + x)`, with `n` counted in the threshold's unit (DR-3.35, Carleton
/// table). `None` for metrics without a published threshold or thresholds whose unit has no
/// supplied count.
pub fn reliability_weight(metric: &Metric, n_pa: u32, n_ab: u32, n_bip: u32) -> Option<f64> {
    let (unit, x) = metric.threshold?;
    if x == 0 {
        return None;
    }
    let n = match unit {
        Unit::Pa => n_pa,
        Unit::Ab => n_ab,
        Unit::Bip => n_bip,
        // No countable `n` supplied for batter-faced / pitch / game thresholds.
        Unit::Bf | Unit::Pitch | Unit::Game => return None,
    };
    Some(n as f64 / (n + x) as f64)
}

/// Defined-count rolling evaluation over a slice of optional values: only defined entries count.
pub fn defined_window(vals: &[Option<f64>]) -> (u32, Option<f64>) {
    let n: u32 = vals.iter().filter(|v| v.is_some()).count() as u32;
    let sum: f64 = vals.iter().filter_map(|v| *v).sum();
    (n, (n > 0).then_some(sum / n as f64))
}

fn sorted_events(logs: &[EventLog], as_of: Date) -> Vec<&Event> {
    let mut all: Vec<&Event> = logs.iter().flat_map(|l| l.events.iter().filter(|e| e.date <= as_of)).collect();
    // Order by (game date, per-season sequence): sequence numbers restart each season.
    all.sort_unstable_by_key(|e| (e.date, e.seq));
    all
}

/// Derive `metric` over `window` at `as_of` from one or more (entity, season) logs. The logs must
/// share one parseable entity key — an invariant of the loader, asserted here.
pub fn derive(logs: &[EventLog], metric: &Metric, window: Window, as_of: Date) -> Series {
    let entity = match logs.first().map(|l| Entity::from_str(&l.entity_key)) {
        Some(Ok(e)) => e,
        _ => panic!("derive: logs carry no parseable entity key"),
    };
    assert!(logs.iter().all(|l| Entity::from_str(&l.entity_key) == Ok(entity)), "derive: logs carry mixed entities");
    let events = sorted_events(logs, as_of);
    let mut points = Vec::with_capacity(events.len());

    // trailing window for `Events{unit, n}`
    let mut backlog: Vec<&Event> = Vec::with_capacity(64);
    // forward-anchored window for `Season` / `Between`
    let mut anchored: Vec<&Event> = Vec::with_capacity(64);

    for &e in &events {
        match window {
            Window::Events { unit, n } => {
                if e.unit_flags.counts(unit) {
                    backlog.push(e);
                    if backlog.len() > n as usize {
                        backlog.drain(..backlog.len() - n as usize);
                    }
                }
                points.push(point_at(e, &backlog, metric));
            }
            Window::Season(season) => {
                if u16::try_from(e.date.year()) == Ok(season) {
                    anchored.push(e);
                    points.push(point_at(e, &anchored, metric));
                }
            }
            Window::Between { from, to } => {
                if e.date < from {
                    continue;
                }
                if e.date > to {
                    break; // sorted: nothing inside the span remains
                }
                anchored.push(e);
                points.push(point_at(e, &anchored, metric));
            }
        }
    }

    Series { entity, metric: MetricId(metric.id.to_owned()), window, as_of, points }
}

/// Build the point anchored at `e` with `window_events` as the eval input slice.
fn point_at(e: &Event, window_events: &[&Event], metric: &Metric) -> Point {
    // Coverage marks where the inputs exist: events outside it are undefined inputs too.
    let in_coverage: Vec<Event> =
        window_events.iter().filter(|w| metric.coverage.contains(&w.date)).map(|w| (*w).clone()).collect();
    let raw = (metric.eval)(&in_coverage).filter(|_| metric.coverage.contains(&e.date));
    let (n_pa, n_ab, n_bip) = (
        count_unit(window_events, Unit::Pa),
        count_unit(window_events, Unit::Ab),
        count_unit(window_events, Unit::Bip),
    );
    Point {
        season: u16::try_from(e.date.year()).unwrap_or(0),
        seq: e.seq,
        date: e.date,
        value: raw,
        n: count_unit(window_events, metric.unit),
        weight: reliability_weight(metric, n_pa, n_ab, n_bip),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{SourceRev, UnitFlags, field};
    use crate::metric::{Coverage, registry};
    use time::Month;

    fn d(y: i32, mo: u8, day: u8) -> Date {
        Date::from_calendar_date(y, Month::try_from(mo).unwrap(), day).unwrap()
    }

    fn rev() -> SourceRev {
        SourceRev { source: "savant", url: String::new(), fetched: 0, sha256: [0; 32] }
    }

    fn ev(date: Date, seq: u64, flags: u32, vals: &[(u16, Option<f64>)]) -> Event {
        Event { seq, game_pk: (seq as u32) + 1000, date, unit_flags: UnitFlags(flags), fields: vals.to_vec() }
    }

    fn log(entity: &str, season: u16, events: Vec<Event>) -> EventLog {
        EventLog { entity_key: entity.to_owned(), season, source_rev: rev(), events }
    }

    const PA_FLAGS: u32 = UnitFlags::PA | UnitFlags::AB | UnitFlags::GAME;
    const BIP_FLAGS: u32 = PA_FLAGS | UnitFlags::BIP;

    #[test]
    fn empty_log_derives_empty_series() {
        let s = derive(
            &[log("p:660271:bat", 2025, vec![])],
            &registry::BAT_WOBA,
            Window::Events { unit: Unit::Pa, n: 50 },
            d(2025, 8, 1),
        );
        assert!(s.points.is_empty());
    }

    #[test]
    fn derive_crosses_season_boundary_and_undefined_never_zero() {
        // Two logs crossing 2023→2024 with an IL/coverage gap: the 2024-03-20 PA sits inside the
        // measured bat-tracking hole, so its point is None; later points still carry pre-season
        // inputs from 2023 across the boundary.
        let mut k = registry::BAT_K_RATE;
        k.coverage = Coverage::BAT_TRACKING;
        let logs = vec![
            log(
                "p:660271:bat",
                2023,
                vec![
                    ev(d(2023, 9, 1), 1, PA_FLAGS, &[(field::K_FLAG, Some(0.0))]),
                    ev(d(2023, 9, 2), 2, PA_FLAGS, &[(field::K_FLAG, Some(0.0))]),
                    ev(d(2023, 9, 3), 3, PA_FLAGS, &[(field::K_FLAG, Some(1.0))]),
                ],
            ),
            log(
                "p:660271:bat",
                2024,
                vec![
                    ev(d(2024, 3, 22), 1, PA_FLAGS, &[(field::K_FLAG, Some(0.0))]), // hole date
                    ev(d(2024, 4, 5), 2, PA_FLAGS, &[(field::K_FLAG, Some(1.0))]),
                    ev(d(2024, 4, 6), 3, PA_FLAGS, &[(field::K_FLAG, Some(0.0))]),
                ],
            ),
        ];
        let s = derive(&logs, &k, Window::Events { unit: Unit::Pa, n: 4 }, d(2024, 10, 1));
        assert_eq!(s.points.len(), 6);
        assert_eq!(s.points[0].value, Some(0.0)); // a defined rate can legitimately be 0.0
        let hole = &s.points[3];
        assert_eq!((hole.date, hole.value), (d(2024, 3, 22), None)); // gap, not zero
        let last = s.points.last().unwrap();
        // window = last 4 PAs (2023-09-03 K=1, hole K excluded from inputs, 04-05 K=1, 04-06 K=0)
        assert_eq!(last.value.map(|v| (v * 3.0).round() as u32), Some(2));
        assert_eq!(last.n, 4);
        // reliability labels exist even on undefined points: n/(n+x) = 4/(4+60)
        assert!((hole.weight.unwrap() - 4.0 / 64.0).abs() < 1e-9);
        assert!((last.weight.unwrap() - 4.0 / 64.0).abs() < 1e-9);
    }

    #[test]
    fn reliability_weights_run_the_carleton_table() {
        let cases: &[(&Metric, u32, u32, u32, f64)] = &[
            (&registry::BAT_K_RATE, 60, 0, 0, 0.5),
            (&registry::BAT_BB_RATE, 120, 0, 0, 0.5),
            (&registry::ISO, 0, 160, 0, 0.5),
            (&registry::OBP, 460, 0, 0, 0.5),
            (&registry::AVG, 0, 910, 0, 0.5),
            (&registry::BABIP, 0, 0, 820, 0.5),
        ];
        for (m, pa, ab, bip, want) in cases {
            assert!((reliability_weight(m, *pa, *ab, *bip).unwrap() - want).abs() < 1e-9, "table point for {}", m.id);
        }
        // Under-supplied data shrinks toward the league (60/(60+x) at n=30 with x=60 → 1/3).
        assert!((reliability_weight(&registry::BAT_K_RATE, 30, 0, 0).unwrap() - 1.0 / 3.0).abs() < 1e-9);
        // Bit-perfect: BB% uses PA n even though AB counts totalled; unit-aware selection.
        assert!(reliability_weight(&registry::BAT_BB_RATE, 120, 999, 0).unwrap() - 0.5 < 1e-9);
        // Without a published threshold: "no threshold".
        assert_eq!(reliability_weight(&registry::BAT_EV, 1000, 0, 0), None);
    }

    #[test]
    fn windows_measured_in_their_own_units() {
        // 60 PA shrinkage must not use AB counts (DR-3.35: n counted in the threshold's unit).
        let mut rate = registry::BAT_K_RATE;
        rate.threshold = Some((Unit::Ab, 60));
        assert_eq!(reliability_weight(&rate, 0, 60, 0), Some(0.5));
        let mut babip = registry::BABIP;
        babip.threshold = Some((Unit::Bip, 820));
        assert_eq!(reliability_weight(&babip, 0, 0, 820), Some(0.5));
    }

    #[test]
    fn reliability_weight_in_series_points_and_season_window() {
        k_rate_pa_series_checks();
        let logs = vec![log(
            "p:660271:bat",
            2025,
            vec![
                ev(d(2025, 4, 1), 1, PA_FLAGS, &[(field::K_FLAG, Some(0.0))]),
                ev(d(2025, 4, 2), 2, BIP_FLAGS, &[(field::K_FLAG, None)]),
                ev(d(2025, 4, 3), 3, PA_FLAGS, &[(field::K_FLAG, Some(1.0))]),
            ],
        )];
        let s = derive(&logs, &registry::BAT_K_RATE, Window::Season(2025), d(2025, 12, 31));
        assert_eq!(s.points.len(), 3); // BIP event also counts as a PA for a PA-unit metric
        let last = s.points.last().unwrap();
        assert_eq!(last.n, 3);
        assert!((last.weight.unwrap() - 3.0 / 63.0).abs() < 1e-9);
        // window crossing: season window must not pull in other seasons
        let s24 = derive(&logs, &registry::BAT_K_RATE, Window::Season(2024), d(2025, 12, 31));
        assert!(s24.points.is_empty());
    }

    fn k_rate_pa_series_checks() {
        // cross-check between the Events and Season windows on the same data
        let logs = vec![log("p:660271:bat", 2025, vec![ev(d(2025, 5, 1), 1, PA_FLAGS, &[(field::K_FLAG, Some(1.0))])])];
        let by_events = derive(&logs, &registry::BAT_K_RATE, Window::Events { unit: Unit::Pa, n: 10 }, d(2025, 6, 1));
        let by_season = derive(&logs, &registry::BAT_K_RATE, Window::Season(2025), d(2025, 6, 1));
        assert_eq!(by_events.points, by_season.points);
    }

    #[test]
    fn between_window_is_forward_anchored_and_respects_to() {
        let logs = vec![log(
            "p:660271:bat",
            2025,
            vec![
                ev(d(2025, 4, 1), 1, PA_FLAGS, &[(field::K_FLAG, Some(0.0))]),
                ev(d(2025, 6, 1), 2, PA_FLAGS, &[(field::K_FLAG, Some(1.0))]),
                ev(d(2025, 9, 1), 3, PA_FLAGS, &[(field::K_FLAG, Some(1.0))]),
            ],
        )];
        let s = derive(
            &logs,
            &registry::BAT_K_RATE,
            Window::Between { from: d(2025, 5, 1), to: d(2025, 8, 1) },
            d(2025, 12, 31),
        );
        assert_eq!(s.points.len(), 1); // only the June 1 event falls in [May 1, Aug 1]
        assert_eq!(s.points[0].value, Some(1.0));
    }

    #[test]
    fn zero_value_windows_are_undefined_not_zero() {
        // A window with the metric's unit counted but no defined inputs → value None, n > 0.
        let logs =
            vec![log("p:660271:bat", 2025, vec![ev(d(2025, 4, 1), 1, PA_FLAGS, &[(field::ESTIMATED_WOBA, None)])])];
        let s = derive(&logs, &registry::BAT_XWOBA, Window::Events { unit: Unit::Pa, n: 5 }, d(2025, 9, 1));
        let p = s.points.last().unwrap();
        assert_eq!((p.n, p.value), (1, None));
    }
}
