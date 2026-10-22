//! Windows and units. Windows are backward-looking and may cross season boundaries when the
//! derive is given several logs (spec 4.3).

use crate::event::{Event, UnitFlags};
use time::Date;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Pa,
    Bf,
    Pitch,
    Game,
    /// At-bats: DR-3.35 thresholds (ISO 160 AB, AVG 910 AB) count in AB.
    Ab,
    /// Balls in play: the BABIP 820 BIP threshold counts in BIP.
    Bip,
}

impl Unit {
    pub const ALL: [Unit; 6] = [Unit::Pa, Unit::Bf, Unit::Pitch, Unit::Game, Unit::Ab, Unit::Bip];
}

/// Backward-looking window anchored at each point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Window {
    /// Trailing event count in a unit.
    Events { unit: Unit, n: u32 },
    /// Season-to-date from that season's first day.
    Season(u16),
    /// Calendar-date span, forward-anchored at `from` (points only through `to`).
    Between { from: Date, to: Date },
}

/// Count of events in `slice` incrementing `unit`.
pub fn count_unit(events: &[&Event], unit: Unit) -> u32 {
    events.iter().filter(|e| e.unit_flags.counts(unit)).count() as u32
}

impl UnitFlags {
    /// Does this event increment the given unit's counter?
    pub const fn counts(self, unit: Unit) -> bool {
        let bit = match unit {
            Unit::Pa => UnitFlags::PA,
            Unit::Ab => UnitFlags::AB,
            Unit::Bip => UnitFlags::BIP,
            Unit::Pitch => UnitFlags::PITCH,
            Unit::Bf => UnitFlags::BF,
            Unit::Game => UnitFlags::GAME,
        };
        self.has(bit)
    }
}
