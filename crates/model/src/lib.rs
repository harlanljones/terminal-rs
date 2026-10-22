//! sabr-model: the A1′ data model — entities, events, windows, metrics and on-demand derived
//! series (docs/plan 02-design-arena §1 A1′).
//!
//! Storage rule: undefined is never NaN/inf and never zero; it is `None` and renders as a gap.

pub mod entity;
pub mod event;
pub mod metric;
pub mod series;
pub mod window;

pub use entity::{Entity, PlayerId, Role};
pub use event::{Event, EventLog, SourceRev, UnitFlags};
pub use metric::{Coverage, DateC, Metric, MetricId};
pub use window::{Unit, Window};

/// Civil date type used everywhere in the model.
pub type Date = time::Date;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_key_round_trip_catalog() {
        let keys = ["p:660271:bat", "p:592789:pit", "t:119", "g:777288", "m:660271:bat/592789:pit"];
        // matchup display canonicalises bat-first, so the reversed form parses and re-prints canonically
        assert_eq!("m:592789:pit/660271:bat".parse::<Entity>().unwrap().to_string(), "m:660271:bat/592789:pit");
        for k in keys {
            assert_eq!(k.parse::<Entity>().map(|e| e.to_string()).as_deref(), Ok(k), "{k}");
        }
    }
}
