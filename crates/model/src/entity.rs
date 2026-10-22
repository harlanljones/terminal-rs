//! Entities and their text keys (`04.3` spec): a player is an MLBAM id plus a role; teams are
//! MLBAM team ids; games are game pks; a matchup is a batter entity and a pitcher entity.

use std::fmt;
use std::str::FromStr;

/// Batter or pitcher. DR-3.2: identity is (player, role) on the MLBAM id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Bat,
    Pit,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Bat => "bat",
            Role::Pit => "pit",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId {
    pub mlbam: u32,
    pub role: Role,
}

/// DR-3.2 / spec 4.3: player, team, game or matchup entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Entity {
    Player(PlayerId),
    Team(u16),
    Game(u32),
    Matchup(PlayerId, PlayerId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityParseError {
    Empty,
    UnknownTag,
    BadPlayer,
    BadNumber,
}

impl fmt::Display for EntityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            EntityParseError::Empty => "empty entity key",
            EntityParseError::UnknownTag => "unknown entity tag (want p/t/g/m)",
            EntityParseError::BadPlayer => "bad player segment (want id:bat|pit)",
            EntityParseError::BadNumber => "entity id is not a number",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for EntityParseError {}

fn parse_num(raw: &str) -> Result<u32, EntityParseError> {
    raw.parse::<u32>().map_err(|_| EntityParseError::BadNumber)
}

fn parse_player_seg(seg: &str) -> Result<PlayerId, EntityParseError> {
    let (id, role) = seg.split_once(':').ok_or(EntityParseError::BadPlayer)?;
    let role = match role {
        "bat" => Role::Bat,
        "pit" => Role::Pit,
        _ => return Err(EntityParseError::BadPlayer),
    };
    Ok(PlayerId { mlbam: parse_num(id)?, role })
}

fn player_pair(mut a: PlayerId, mut b: PlayerId) -> (PlayerId, PlayerId) {
    if a.role == Role::Pit {
        std::mem::swap(&mut a, &mut b);
    }
    (a, b)
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Entity::Player(p) => write!(f, "p:{}:{}", p.mlbam, p.role),
            Entity::Team(t) => write!(f, "t:{t}"),
            Entity::Game(g) => write!(f, "g:{g}"),
            Entity::Matchup(b, p) => {
                let (bat, pit) = player_pair(b, p);
                write!(f, "m:{}:{}/{}:{}", bat.mlbam, bat.role, pit.mlbam, pit.role)
            }
        }
    }
}

impl FromStr for Entity {
    type Err = EntityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (tag, rest) = s.split_once(':').ok_or(EntityParseError::Empty)?;
        if rest.is_empty() {
            return Err(EntityParseError::BadNumber);
        }
        match tag {
            "p" => Ok(Entity::Player(parse_player_seg(rest)?)),
            "t" => Ok(Entity::Team(u16::try_from(parse_num(rest)?).map_err(|_| EntityParseError::BadNumber)?)),
            "g" => Ok(Entity::Game(parse_num(rest)?)),
            "m" => {
                let (a, b) = rest.split_once('/').ok_or(EntityParseError::BadPlayer)?;
                let (bat, pit) = player_pair(parse_player_seg(a)?, parse_player_seg(b)?);
                Ok(Entity::Matchup(bat, pit))
            }
            _ => Err(EntityParseError::UnknownTag),
        }
    }
}

impl FromStr for PlayerId {
    type Err = EntityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_player_seg(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_round_trip() {
        let e = Entity::Player(PlayerId { mlbam: 660_271, role: Role::Bat });
        let k = e.to_string();
        assert_eq!(k, "p:660271:bat");
        assert_eq!(k.parse::<Entity>().unwrap(), e);
    }

    #[test]
    fn team_game_round_trip() {
        assert_eq!("t:119".to_string(), "t:119");
        assert_eq!("t:119".parse::<Entity>().unwrap(), Entity::Team(119));
        assert_eq!("g:777288".parse::<Entity>().unwrap(), Entity::Game(777_288));
    }

    #[test]
    fn matchup_normalises_bat_first_and_round_trips() {
        let e =
            Entity::Matchup(PlayerId { mlbam: 592_789, role: Role::Pit }, PlayerId { mlbam: 660_271, role: Role::Bat });
        assert_eq!(e.to_string(), "m:660271:bat/592789:pit");
        // parsing canonicalises: the bat player comes first in the stored pair
        let reparsed = "m:660271:bat/592789:pit".parse::<Entity>().unwrap();
        assert_eq!(
            reparsed,
            Entity::Matchup(PlayerId { mlbam: 660_271, role: Role::Bat }, PlayerId { mlbam: 592_789, role: Role::Pit })
        );
        assert_eq!(reparsed.to_string(), e.to_string());
    }

    #[test]
    fn rejects_garbage() {
        let errs = ["", "x:1", "p:abc:bat", "p:1", "t:99999999", "m:1:bat"];
        for k in errs {
            assert!(k.parse::<Entity>().is_err(), "expected {k:?} to fail");
        }
    }
}
