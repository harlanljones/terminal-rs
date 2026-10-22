//! Events: one log per (entity, season); a Savant-shaped event with unit counters and a field map
//! keyed by the pinned allowlist. Sequence keys are `(game_date, at_bat_number, pitch_number)` per
//! the contract; here they are already reduced to a dense per-(entity, season) `u64`.

use time::Date;

/// Which counters an event increments (spec 4.3 unit markers). Bitset, never auto-derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct UnitFlags(pub u32);

impl UnitFlags {
    pub const PA: u32 = 1 << 0;
    pub const AB: u32 = 1 << 1;
    pub const BIP: u32 = 1 << 2;
    pub const PITCH: u32 = 1 << 3;
    pub const BF: u32 = 1 << 4;
    pub const GAME: u32 = 1 << 5;

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn has(self, bit: u32) -> bool {
        self.0 & bit == bit
    }
}

/// Indices into [`Event::fields`], pinned by the Savant allowlist (contract: 119-column CSV; the
/// deprecated spin/angle/length columns are never read).
pub mod field {
    pub const LAUNCH_SPEED: u16 = 0;
    pub const LAUNCH_ANGLE: u16 = 1;
    pub const WOBA_VALUE: u16 = 2;
    pub const ESTIMATED_WOBA: u16 = 3;
    /// 0/1 flag: PA ended in a strikeout.
    pub const K_FLAG: u16 = 4;
    /// 0/1 flag: PA ended in a walk.
    pub const BB_FLAG: u16 = 5;
    /// 0/1 flag: pitch whiff.
    pub const WHIFF_FLAG: u16 = 6;
    /// 0/1 flag: swing out-of-zone (chase).
    pub const OOB_FLAG: u16 = 7;
    pub const BAT_SPEED: u16 = 8;
    pub const SWING_LENGTH: u16 = 9;
    pub const ATTACK_ANGLE: u16 = 10;
    pub const ATTACK_DIRECTION: u16 = 11;
    pub const SWING_PATH_TILT: u16 = 12;
    pub const RELEASE_SPEED: u16 = 13;
    pub const SPIN_RATE: u16 = 14;
    pub const EXTENSION: u16 = 15;
    /// 0/1 flag: ball in play on the PA.
    pub const BIP_FLAG: u16 = 16;
    /// 0/1 flag: barrel batted ball.
    pub const BARREL_FLAG: u16 = 17;
    /// 0/1 flag: hard-hit (EV ≥ 95 mph) batted ball.
    pub const HARD_HIT_FLAG: u16 = 18;
}

/// One Savant row reduced to the pinned allowlist.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Dense per (entity, season), assigned by sorting on (game_date, game_pk, at_bat_number, pitch_number).
    pub seq: u64,
    pub game_pk: u32,
    pub date: Date,
    pub unit_flags: UnitFlags,
    /// `(field index, value)`; `None` = undefined — never zero, never NaN. Kept as a small
    /// `Vec` of pairs rather than a map: allowlist is fixed, lookups are linear over < 20 entries.
    pub fields: Vec<(u16, Option<f64>)>,
}

impl Event {
    pub fn field(&self, idx: u16) -> Option<f64> {
        self.fields.iter().find(|(i, _)| *i == idx).and_then(|(_, v)| *v)
    }

    pub fn set_field(&mut self, idx: u16, v: Option<f64>) {
        if let Some(slot) = self.fields.iter_mut().find(|(i, _)| *i == idx) {
            slot.1 = v;
        } else {
            self.fields.push((idx, v));
        }
    }
}

/// Provenance of the raw payload a log was normalised from (gotcha H6: mixed provenance).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRev {
    pub source: &'static str,
    pub url: String,
    /// Unix seconds of fetch time.
    pub fetched: u64,
    pub sha256: [u8; 32],
}

/// One (entity, season) event log. The entity key is the canonical text form ([`crate::Entity`]).
#[derive(Clone, Debug)]
pub struct EventLog {
    pub entity_key: String,
    pub season: u16,
    pub source_rev: SourceRev,
    pub events: Vec<Event>,
}
