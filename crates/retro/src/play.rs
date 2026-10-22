//! Play-string parser for Retrosheet event tokens (spec 4.5).
//!
//! Covers the measured 2024 token families. Unknown tokens fail loudly with
//! [`ParseError::UnknownToken`] naming the token; nothing is silently dropped.

use std::fmt;

/// Runner-advance flags from parenthesised markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvFlag {
    /// Unearned run (runner already on base when he reached).
    Ur,
    /// No RBI credited for this run.
    Nr,
    /// Tie-breaking (extra-inning placed) unearned run.
    Tur,
    /// Run scored on a wild pitch / passed ball / balk.
    Wp,
    /// Error number involved in the advance.
    Err(u8),
    /// Throw annotation.
    Throw,
    /// `(RBI)` — the run IS an RBI (obstruction scores); no counting effect here.
    Rbi,
}

/// One runner movement: `1-3`, `2XH`, `B-1`.
#[derive(Debug, Clone)]
pub struct Advance {
    /// Origin: 0 = batter, 1..=3 = base.
    pub from: u8,
    /// Destination: 1..=3, 4 = home.
    pub to: u8,
    /// `X` advance: runner put out (unless erased by an error flag).
    pub out: bool,
    pub flags: Vec<AdvFlag>,
}

impl Advance {
    pub fn erased_by_error(&self) -> bool {
        self.out && self.flags.iter().any(|f| matches!(f, AdvFlag::Err(_)))
    }

    /// True when this advance is marked unearned / no-RBI / error.
    pub fn unearned_flagged(&self) -> bool {
        self.flags.iter().any(|f| matches!(f, AdvFlag::Ur | AdvFlag::Nr | AdvFlag::Tur | AdvFlag::Wp | AdvFlag::Err(_)))
    }
}

/// Base event kind (the token before any `/` modifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Bare fielder sequence: batter retired.
    Out,
    Single,
    Double,
    Triple,
    /// Home run, including inside-the-park.
    Homer,
    GroundRuleDouble,
    Walk,
    IntWalk,
    Hbp,
    Strikeout,
    SacHit,
    SacFly,
    /// Catcher interference: PA, not an AB, batter awarded first.
    Interference,
    /// Batter reached on error.
    Error,
    FieldersChoice,
    ForceOut,
    /// Batter retired on a double play.
    DoublePlay,
    TriplePlay,
    /// No double play (batter retired, DP prevented).
    NoDoublePlay,
    /// Bunt pop fly, batter retired.
    BuntPop,
    /// Bunt roller, batter retired.
    BuntRoller,
    /// Batter interference, retired.
    BatterInterference,
    /// Non-PA runner events.
    Steal(u8),
    CaughtStealing(u8),
    Pickoff(u8),
    PickoffCs(u8),
    WildPitch,
    PassedBall,
    Balk,
    Indifference,
    OtherAdvance,
    /// Error on a foul fly; the PA continues.
    FoulError,
    Obstruction,
    RunnerInterference,
}

impl Kind {
    pub fn is_pa(self) -> bool {
        !matches!(
            self,
            Kind::Steal(_)
                | Kind::CaughtStealing(_)
                | Kind::Pickoff(_)
                | Kind::PickoffCs(_)
                | Kind::WildPitch
                | Kind::PassedBall
                | Kind::Balk
                | Kind::Indifference
                | Kind::OtherAdvance
                | Kind::FoulError
                | Kind::Obstruction
                | Kind::RunnerInterference
        )
    }
}

/// A parsed play.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub kind: Kind,
    /// Compound suffix after `K+`/`W+` (e.g. `K+SB2` also steals second).
    pub suffix: Option<Kind>,
    /// Additional runner events in the main section (`SB3;SB2`).
    pub extras: Vec<Kind>,
    /// Explicit batter movement (`B-1`, `BX2`, ...).
    pub batter: Option<Advance>,
    /// Default batter destination: 1..=3, 4 = home, 0 = out.
    pub dest: u8,
    pub advances: Vec<Advance>,
    /// Error flag attached to the base token (erases implicit outs).
    pub base_error: bool,
    /// Double-play marker among the base modifiers (`XD`, `/DP`, `GDP`, ...).
    pub dp: bool,
    /// Sac-fly marker among the modifiers.
    pub sf: bool,
    /// Sac-hit marker among the modifiers.
    pub sh: bool,
    /// Inside-the-park home-run marker among the modifiers.
    pub iphr: bool,
}

impl Parsed {
    /// True when the batter reaches base safely (his movement is not an out).
    pub fn batter_safe(&self) -> bool {
        match &self.batter {
            Some(a) => !a.out || a.erased_by_error(),
            None => self.dest >= 1,
        }
    }

    /// Outs recorded on this play (errors erase outs).
    pub fn outs(&self) -> u32 {
        let x = self.advances.iter().filter(|a| a.out && !a.erased_by_error()).count() as u32;
        match self.kind {
            Kind::Strikeout => 1 + x,
            Kind::DoublePlay => 2 + x,
            Kind::TriplePlay => 3 + x,
            Kind::ForceOut => {
                // FO: batter reaches on the force (implicit out of the lead
                // runner) or is himself retired when no B advance is given.
                1 + x
            }
            Kind::SacHit | Kind::BuntPop | Kind::BuntRoller | Kind::BatterInterference => 1 + x,
            Kind::Out | Kind::NoDoublePlay => {
                if self.dp {
                    2 + x
                } else {
                    1 + x
                }
            }
            Kind::CaughtStealing(_) | Kind::Pickoff(_) | Kind::PickoffCs(_) => {
                if self.base_error {
                    x
                } else {
                    1 + x
                }
            }
            _ => x,
        }
    }

    /// Runs on this play are not credited as RBI.
    pub fn no_rbi(&self) -> bool {
        matches!(
            self.kind,
            Kind::Strikeout
                | Kind::DoublePlay
                | Kind::TriplePlay
                | Kind::Error
                | Kind::WildPitch
                | Kind::PassedBall
                | Kind::Balk
                | Kind::Steal(_)
                | Kind::CaughtStealing(_)
                | Kind::Pickoff(_)
                | Kind::PickoffCs(_)
                | Kind::Indifference
                | Kind::OtherAdvance
                | Kind::FoulError
                | Kind::Obstruction
                | Kind::RunnerInterference
        )
    }

    /// Batter scoring run counts as unearned when he reached via error.
    pub fn unearned_batter(&self) -> bool {
        matches!(self.kind, Kind::Error)
    }
}

#[derive(Debug)]
pub enum ParseError {
    UnknownToken { token: String, raw: String },
    BadAdvance { token: String, raw: String },
    BadParen { token: String, raw: String },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownToken { token, raw } => write!(f, "unknown play token `{token}` in `{raw}`"),
            ParseError::BadAdvance { token, raw } => write!(f, "unknown runner advance `{token}` in `{raw}`"),
            ParseError::BadParen { token, raw } => {
                write!(f, "unknown parenthesised marker `({token})` in `{raw}`")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Split at the first `.` outside parentheses into (main, advances).
fn split_dot(ev: &str) -> (&str, Option<&str>) {
    let bytes = ev.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'.' if depth == 0 => return (&ev[..i], Some(&ev[i + 1..])),
            _ => {}
        }
    }
    (ev, None)
}

/// Extract all parenthesised groups from `s`, returning (text without groups, group contents).
fn extract_parens(s: &str) -> Result<(String, Vec<String>), ParseError> {
    let mut text = String::with_capacity(s.len());
    let mut groups = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                if depth == 0 {
                    cur.clear();
                } else {
                    cur.push(ch);
                }
                depth += 1;
            }
            ')' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    groups.push(std::mem::take(&mut cur));
                } else {
                    cur.push(ch);
                }
            }
            _ => {
                if depth == 0 {
                    text.push(ch);
                } else {
                    cur.push(ch);
                }
            }
        }
    }
    Ok((text, groups))
}

/// Base words, longest first so prefix matching is unambiguous.
const WORDS: &[&str] = &[
    "IW", "HR", "SH", "SF", "GDP", "NDP", "POCS", "FLE", "DGR", "SB", "CS", "PO", "FC", "WP", "PB", "BK", "OA", "DI",
    "BINT", "RINT", "OBS", "IPHR", "K+", "W+", "HP", "K", "W", "S", "D", "T", "E", "C", "F", "L", "P", "G", "FL", "BP",
    "BR", "BL", "DP", "TP", "BG", "TH", "FO",
];

/// Modifier-position words (also accepted in base position).
const MOD_WORDS: &[&str] = &[
    "IPHR", "BG", "BINT", "RINT", "BR", "BL", "BP", "OBS", "TH", "DP", "GDP", "NDP", "TP", "GTP", "LTP", "FTP", "FL",
    "SH", "SF", "DGR", "FO",
];

/// Validate a fielder/position tail like `8XD+`, `34S-`, `5D`.
fn valid_tail(tail: &str) -> bool {
    tail.chars().all(|c| {
        matches!(
            c,
            '0'..='9' | 'L' | 'R' | 'F' | 'S' | 'M' | 'D' | 'G' | 'P' | 'X' | 'H' | 'W' | '+' | '-' | '!' | '#' | '?'
        )
    })
}

/// Validate a modifier segment: a known word plus an optional position tail.
fn valid_modifier(seg: &str) -> bool {
    // `C`/`AP`/`INT`/`FINT`/`IF` are annotation-only modifiers. `MREV`/`UREV` = manager /
    // umpire replay-review annotations; `BOOT`/`PASS`/`UINT` are rare challenge
    // annotations. The review outcome is reflected in the play text itself, so they
    // carry no counting effect here.
    if matches!(seg, "C" | "AP" | "INT" | "FINT" | "IF" | "MREV" | "UREV" | "BOOT" | "PASS" | "UINT" | "BF") {
        return true;
    }
    // `E2` after interference: the throw continues for an error.
    if let Some(tail) = seg.strip_prefix('E')
        && tail.chars().all(|c| c.is_ascii_digit())
        && !tail.is_empty()
    {
        return true;
    }
    for w in MOD_WORDS {
        if let Some(tail) = seg.strip_prefix(w)
            && valid_tail(tail)
        {
            return true;
        }
    }
    valid_tail(seg)
}

fn base_kind(word: &str) -> Option<Kind> {
    let kind = match word {
        "IW" => Kind::IntWalk,
        "HP" => Kind::Hbp,
        "HR" | "IPHR" => Kind::Homer,
        "SH" => Kind::SacHit,
        "SF" => Kind::SacFly,
        "GDP" => Kind::DoublePlay,
        "DP" => Kind::DoublePlay,
        "NDP" => Kind::NoDoublePlay,
        "TP" => Kind::TriplePlay,
        "F" => Kind::Out,
        "L" => Kind::Out,
        "P" | "FL" => Kind::Out,
        "G" => Kind::Out,
        "BP" => Kind::BuntPop,
        "BR" | "BL" => Kind::BuntRoller,
        "BINT" => Kind::BatterInterference,
        "W+" | "W" => Kind::Walk,
        "K+" | "K" => Kind::Strikeout,
        "S" => Kind::Single,
        "DGR" => Kind::GroundRuleDouble,
        "D" => Kind::Double,
        "T" => Kind::Triple,
        "E" => Kind::Error,
        "FC" => Kind::FieldersChoice,
        "FO" => Kind::ForceOut,
        "C" => Kind::Interference,
        "SB" => Kind::Steal(0),
        "CS" => Kind::CaughtStealing(0),
        "PO" => Kind::Pickoff(0),
        "POCS" => Kind::PickoffCs(0),
        "WP" => Kind::WildPitch,
        "PB" => Kind::PassedBall,
        "BK" => Kind::Balk,
        "DI" => Kind::Indifference,
        "OA" => Kind::OtherAdvance,
        "FLE" => Kind::FoulError,
        "OBS" => Kind::Obstruction,
        "RINT" => Kind::RunnerInterference,
        _ => return None,
    };
    Some(kind)
}

/// Non-PA runner event words for compound suffixes and `;` extras.
const RUNNER_WORDS: &[&str] = &["SB", "CS", "POCS", "PO", "WP", "PB", "BK", "DI", "OA"];

/// Parse a runner movement spec like `1-3(E2/TH)` or `BX2(26)`.
fn parse_advance(spec: &str, raw: &str) -> Result<Advance, ParseError> {
    // A trailing `#` is a scoring-annotation marker ("time play"), not an advance.
    let spec = spec.strip_suffix('#').unwrap_or(spec);
    let (spec, groups) = extract_parens(spec)?;
    let bad = |t: String| ParseError::BadAdvance { token: t, raw: raw.to_string() };
    let bytes = spec.as_bytes();
    if bytes.len() != 3 {
        return Err(bad(spec.to_string()));
    }
    let from = match bytes[0] {
        b'B' => 0u8,
        b'1'..=b'3' => bytes[0] - b'0',
        _ => return Err(bad(spec.to_string())),
    };
    let (out, to) = match bytes[1] {
        b'-' => (false, bytes[2]),
        b'X' => (true, bytes[2]),
        _ => return Err(bad(spec.to_string())),
    };
    let to = match to {
        b'1'..=b'3' => to - b'0',
        b'H' => 4u8,
        _ => return Err(bad(spec.to_string())),
    };
    let mut flags = Vec::new();
    for g in &groups {
        // Numeric groups are putout/throw annotations; skip them.
        if g.chars().all(|c| c.is_ascii_digit()) && !g.is_empty() {
            continue;
        }
        // `<digits?>E<digits>[/THH?]` error annotation, e.g. `92E6/TH`: erases the out.
        if let Some((pre, rest)) = g.split_once('E') {
            let (num, throw) = match rest.split_once('/') {
                Some((n, t)) => (n, t),
                None => (rest, ""),
            };
            if let Ok(n) = num.parse::<u8>()
                && !num.is_empty()
                && (pre.is_empty() || pre == "B" || pre.chars().all(|c| c.is_ascii_digit()))
                && is_throw_suffix(throw)
            {
                flags.push(AdvFlag::Err(n));
                continue;
            }
        }
        flags.push(parse_adv_flag(g, raw)?);
    }
    Ok(Advance { from, to, out, flags })
}

/// Throw suffix after an error number: ``, `TH`, `THH`, or `TH<base>` (`E2/TH3`).
fn is_throw_suffix(t: &str) -> bool {
    t.is_empty() || t == "TH" || t == "THH" || (t.starts_with("TH") && t[2..].chars().all(|c| c.is_ascii_digit()))
}

fn parse_adv_flag(g: &str, raw: &str) -> Result<AdvFlag, ParseError> {
    let bad = |g: String| ParseError::BadParen { token: g, raw: raw.to_string() };
    // Throw annotations may carry a base digit or review words: `E2/TH3`, `96/TH`,
    // `SB2/C`, `SB2/MREV` — split on `/` and require every part to be acceptable.
    if g.contains('/') {
        for part in g.split('/') {
            if part.is_empty() {
                return Err(bad(g.to_string()));
            }
            parse_adv_flag(part, raw)?;
        }
        return Ok(AdvFlag::Throw);
    }
    // Pure throw/putout annotations: digits (optionally with `!` force markers).
    if !g.is_empty() && g.chars().all(|c| c.is_ascii_digit() || c == '!') {
        return Ok(AdvFlag::Throw);
    }
    // `(SB2)`/`(CS3)`/`(CSH)`/`(SBH)`: "this advance was a steal / caught stealing"
    // annotation on an advance the parser already counts from its shape — informational.
    if (g.starts_with("SB") || g.starts_with("CS"))
        && g.len() == 3
        && g[2..].chars().all(|c| c.is_ascii_digit() || c == 'H')
    {
        return Ok(AdvFlag::Throw);
    }
    // Prose review annotations ("tag play", player names, …) carry no counting
    // information; accept and ignore. Unknown UPPER-CASE tokens still fail loudly.
    if g.chars().any(|c| c.is_ascii_lowercase() || c == ' ') {
        return Ok(AdvFlag::Throw);
    }
    if let Some(rest) = g.strip_prefix('E') {
        let (num, throw) = match rest.split_once('/') {
            Some((n, t)) => (n, t),
            None => (rest, ""),
        };
        let Ok(n) = num.parse::<u8>() else { return Err(bad(g.to_string())) };
        if !is_throw_suffix(throw) {
            return Err(bad(g.to_string()));
        }
        return Ok(AdvFlag::Err(n));
    }
    match g {
        "UR" => Ok(AdvFlag::Ur),
        "NR" => Ok(AdvFlag::Nr),
        "TUR" => Ok(AdvFlag::Tur),
        "WP" | "PB" | "BK" => Ok(AdvFlag::Wp),
        "RBI" => Ok(AdvFlag::Rbi),
        _ if is_throw_suffix(g) => Ok(AdvFlag::Throw), // `TH`, `TH1`…`TH3`, `THH`
        _ => Err(bad(g.to_string())),
    }
}

/// Parse a whole event string (the sixth field of a `play,` line).
pub fn parse_event(ev: &str) -> Result<Parsed, ParseError> {
    let (main, adv_str) = split_dot(ev);
    let mut advances = Vec::new();
    if let Some(a) = adv_str {
        for spec in a.split(';') {
            advances.push(parse_advance(spec, ev)?);
        }
    }

    // A `;` in the main section lists additional runner events on the same
    // play (double steal `SB3;SB2`). Only non-PA runner words are accepted.
    let (first, rest_raw) = match main.split_once(';') {
        Some((a, b)) => (a, b),
        None => (main, ""),
    };
    let extras = parse_extras(rest_raw, ev)?;

    let (base_part, groups) = extract_parens(first)?;
    let mut segs = base_part.split('/');
    let head = segs.next().unwrap_or_default();
    let mods: Vec<&str> = segs.collect();
    for m in &mods {
        if !valid_modifier(m) {
            return Err(ParseError::UnknownToken { token: (*m).to_string(), raw: ev.to_string() });
        }
    }

    let mut p = Parsed {
        kind: Kind::Out,
        suffix: None,
        extras,
        batter: None,
        dest: 0,
        advances,
        base_error: false,
        dp: false,
        sf: false,
        sh: false,
        iphr: false,
    };
    for m in &mods {
        match *m {
            "SF" => p.sf = true,
            "SH" => p.sh = true,
            "GDP" | "DP" => p.dp = true,
            "IPHR" => p.iphr = true,
            _ => {}
        }
        if m.ends_with("XD") {
            p.dp = true;
        }
    }

    // Base-token paren groups.
    for g in &groups {
        parse_base_paren(g, ev, &mut p)?;
    }

    parse_head(head, ev, &mut p)?;
    if p.sf {
        p.kind = Kind::SacFly;
    }
    if p.sh {
        p.kind = Kind::SacHit;
    }
    if p.iphr {
        p.kind = Kind::Homer;
    }
    p.dest = default_dest(p.kind);
    Ok(p)
}

fn parse_base_paren(g: &str, raw: &str, p: &mut Parsed) -> Result<(), ParseError> {
    // Advance-like content inside the base parens.
    if g.contains('-') || g.contains('X') {
        let a = parse_advance(g, raw)?;
        if a.from == 0 {
            p.batter = Some(a);
        } else {
            p.advances.push(a);
        }
        return Ok(());
    }
    // Error-flagged runner out, e.g. `(1E4)`, `(26E3)`: out erased by error.
    if let Some((pre, rest)) = g.split_once('E') {
        let (num, throw) = match rest.split_once('/') {
            Some((n, t)) => (n, t),
            None => (rest, ""),
        };
        if !num.is_empty()
            && num.chars().all(|c| c.is_ascii_digit())
            && (pre.is_empty() || pre == "B" || pre.chars().all(|c| c.is_ascii_digit()))
            && is_throw_suffix(throw)
        {
            p.base_error = true;
            return Ok(());
        }
    }
    if g.chars().all(|c| c.is_ascii_digit()) && !g.is_empty() {
        return Ok(()); // throw sequence / putout annotation
    }
    // Review/booth and prose annotations ("tag play", "force play", "hit by pitch",
    // "stadium boundary call", player names in review contexts, "Threw helmet"): they
    // carry no counting information — every counting marker (UR/NR/TUR/WP/PB/BK/RBI/
    // E#/TH/throws) is handled above. A group containing a lowercase letter or a space
    // is prose; accept and ignore. Unknown UPPER-CASE tokens still fail loudly.
    if g.chars().any(|c| c.is_ascii_lowercase() || c == ' ') {
        return Ok(());
    }
    match g {
        "B" | "WP" => Ok(()), // putout annotation
        _ => match parse_adv_flag(g, raw) {
            Ok(AdvFlag::Err(_)) => {
                p.base_error = true;
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        },
    }
}

fn base_kind_word(seg: &str) -> Option<(Kind, &str)> {
    for w in WORDS {
        if let Some(tail) = seg.strip_prefix(w)
            && let Some(kind) = base_kind(w)
        {
            return Some((kind, tail));
        }
    }
    None
}

fn parse_head(head: &str, raw: &str, p: &mut Parsed) -> Result<(), ParseError> {
    let unknown = || ParseError::UnknownToken { token: head.to_string(), raw: raw.to_string() };
    // Bare fielder sequence: batter retired.
    if head.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        if let Some((pre, post)) = head.split_once('E') {
            // `6E3`: ball fielded to 6, error by 3 → batter safe. A `!` on the fielder
            // sequence marks a forced play (e.g. `3!E1`).
            let pre = pre.trim_end_matches('!');
            if !pre.is_empty()
                && pre.chars().all(|c| c.is_ascii_digit())
                && !post.is_empty()
                && post.chars().all(|c| c.is_ascii_digit())
            {
                p.kind = Kind::Error;
                return Ok(());
            }
            return Err(unknown());
        }
        if !valid_tail(head) {
            return Err(unknown());
        }
        p.kind = Kind::Out;
        return Ok(());
    }
    let Some((kind, tail)) = base_kind_word(head) else {
        return Err(unknown());
    };
    // Compound `K+CS2` / `W+SB3` / `K23+WP` / `K+E2/TH` / `IW+SB3` / `K+POCS2`.
    if matches!(kind, Kind::Strikeout | Kind::Walk | Kind::IntWalk | Kind::Hbp) && head.contains('+') {
        // Tail parts after the first `+`: pure-digit parts are fielder annotations;
        // runner-word parts (with optional base digit or H) are extra runner events; an
        // `E<digit>[/throw]` part means the batter reached on error (the PA is an error,
        // not a strikeout/walk).
        let mut reached_on_error = false;
        for part in tail.split('+') {
            if part.is_empty() {
                continue; // head like `IW+WP`: tail starts at the `+`
            }
            if part.chars().all(|c| c.is_ascii_digit()) && !part.is_empty() {
                continue; // fielder annotation on the out
            }
            let (part, _flags) = match part.split_once('/') {
                Some((w, flags)) => {
                    for f in flags.split('/') {
                        if f.is_empty() {
                            return Err(unknown());
                        }
                        if parse_adv_flag(f, raw).is_err() && !valid_modifier(f) {
                            return Err(unknown());
                        }
                    }
                    (w, ())
                }
                None => (part, ()),
            };
            // A RUNNER_WORDS word with a base tail (`SB2`, `SBH`, `POCS2`, `POH`…) —
            // with optional leading fielder digits consumed above? leading digits only
            // survive as their own `+` part, so require the part to start at the word.
            if let Some(runner) =
                RUNNER_WORDS.iter().find_map(|w| part.strip_prefix(w).and_then(|r| base_kind(w).is_some().then_some(r)))
            {
                let base_ok = runner.is_empty()
                    || runner == "H"
                    || (runner.len() == 1 && runner.chars().all(|c| c.is_ascii_digit()));
                if !base_ok {
                    return Err(unknown());
                }
                if let Some(suffix) = parse_suffix(part, raw)? {
                    p.suffix = Some(suffix);
                }
                continue;
            }
            if part.starts_with('E') && part[1..].chars().all(|c| c.is_ascii_digit()) {
                reached_on_error = true;
                continue;
            }
            return Err(unknown());
        }
        p.kind = if reached_on_error { Kind::Error } else { kind };
        return Ok(());
    }
    if matches!(kind, Kind::Steal(_) | Kind::CaughtStealing(_) | Kind::Pickoff(_) | Kind::PickoffCs(_)) {
        let base = if tail.is_empty() {
            0
        } else if tail == "H" {
            4 // steal/caught-stealing/pickoff of home
        } else {
            tail.parse().map_err(|_| unknown())?
        };
        let ok = if matches!(kind, Kind::Pickoff(_)) {
            (1..=3).contains(&base) // PO1: the runner on first is picked off
        } else {
            (2..=4).contains(&base) // SB2: second base is the destination
        };
        if !ok {
            return Err(unknown());
        }
        p.kind = match kind {
            Kind::Steal(_) => Kind::Steal(base),
            Kind::CaughtStealing(_) => Kind::CaughtStealing(base),
            Kind::Pickoff(_) => Kind::Pickoff(base),
            _ => Kind::PickoffCs(base),
        };
        return Ok(());
    }
    if !valid_tail(tail) {
        return Err(unknown());
    }
    p.kind = kind;
    Ok(())
}

/// Parse the extra runner events after the first `;` in the main section.
fn parse_extras(rest: &str, raw: &str) -> Result<Vec<Kind>, ParseError> {
    let mut out = Vec::new();
    if rest.is_empty() {
        return Ok(out);
    }
    for seg in rest.split(';') {
        let unknown = || ParseError::UnknownToken { token: seg.to_string(), raw: raw.to_string() };
        // Runner word + optional base + optional `(flags)` and `/annotations`
        // (`SB2/C`, `SB2/MREV`, `SBH(UR)`).
        let seg = match seg.split_once('(') {
            Some((w, flags)) => {
                for g in extract_parens(&format!("{w}({flags}"))?.1 {
                    if parse_adv_flag(&g, raw).is_err() && !valid_modifier(&g) {
                        return Err(unknown());
                    }
                }
                w
            }
            None => seg,
        };
        let seg = match seg.split_once('/') {
            Some((w, flags)) => {
                for f in flags.split('/') {
                    if f.is_empty() {
                        return Err(unknown());
                    }
                    if parse_adv_flag(f, raw).is_err() && !valid_modifier(f) {
                        return Err(unknown());
                    }
                }
                w
            }
            None => seg,
        };
        let Some((kind, tail)) = RUNNER_WORDS
            .iter()
            .find_map(|w| seg.strip_prefix(w).map(|r| (*w, r)))
            .and_then(|(w, r)| base_kind(w).map(|k| (k, r)))
        else {
            return Err(unknown());
        };
        let base = |tail: &str| -> Result<u8, ParseError> {
            if tail.is_empty() {
                return Ok(2);
            }
            if tail == "H" {
                return Ok(4); // steal/caught-stealing/pickoff of home
            }
            tail.parse::<u8>().map_err(|_| unknown())
        };
        let kind = match kind {
            Kind::Steal(_) => Kind::Steal(base(tail)?),
            Kind::CaughtStealing(_) => Kind::CaughtStealing(base(tail)?),
            Kind::Pickoff(_) => Kind::Pickoff(base(tail)?),
            Kind::PickoffCs(_) => Kind::PickoffCs(base(tail)?),
            k => k,
        };
        out.push(kind);
    }
    Ok(out)
}

fn parse_suffix(tail: &str, raw: &str) -> Result<Option<Kind>, ParseError> {
    if tail.is_empty() {
        return Ok(None);
    }
    let unknown = || ParseError::UnknownToken { token: tail.to_string(), raw: raw.to_string() };
    let Some((kind, rest)) = RUNNER_WORDS
        .iter()
        .find_map(|w| tail.strip_prefix(w).map(|r| (*w, r)))
        .and_then(|(w, r)| base_kind(w).map(|k| (k, r)))
    else {
        return Err(unknown());
    };
    let rest: u8 = match rest {
        "" => 2,
        "H" => 4,
        r => r.parse().map_err(|_| unknown())?,
    };
    let kind = match kind {
        Kind::Steal(_) => Kind::Steal(rest),
        Kind::CaughtStealing(_) => Kind::CaughtStealing(rest),
        k => k,
    };
    Ok(Some(kind))
}

fn default_dest(kind: Kind) -> u8 {
    match kind {
        Kind::Single
        | Kind::Walk
        | Kind::IntWalk
        | Kind::Hbp
        | Kind::Interference
        | Kind::Error
        | Kind::FieldersChoice
        | Kind::Obstruction => 1,
        Kind::Double | Kind::GroundRuleDouble => 2,
        Kind::Triple => 3,
        Kind::Homer => 4,
        _ => 0,
    }
}
