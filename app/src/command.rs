//! Bloomberg-style command line: `<SECURITY> [<FUNCTION> args] <GO>`.
//!
//! Examples: `AAPL`, `AAPL US EQUITY <GO>`, `5`, `1W`, `RSI`, `SMA 50`, `ICHIMOKU`, `IND MOMENTUM`,
//! `CMP MSFT`, `AAPL CONG`, `CONG`, `POL PELOSI`, `TRACK PELOSI`, `LAYOUT 4`, `FIB`, `HELP`.

use sabr_data::catalog::{CatRow, Catalog};
use sabr_model::metric::registry;
use sabr_model::window::{Unit, Window};
use tr_chart::drawings::Tool;
use tr_chart::scene::ChartType;
pub use tr_core::indicators::registry::Spec as IndicatorSpec;
use tr_core::{Security, Timeframe, YellowKey};

/// Indicators on a new chart: the 20 and 50 bar simple moving averages.
pub fn default_indicators() -> Vec<IndicatorSpec> {
    [20.0, 50.0].iter().filter_map(|n| IndicatorSpec::new("SMA", &[*n])).collect()
}

/// Views of the brokerage portfolio pane (`PORT`, `ACCT`, `POS`, `ORD`, `ACTV`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PortView {
    #[default]
    Summary,
    Accounts,
    Positions,
    Orders,
    Activity,
}

impl PortView {
    pub const ALL: [PortView; 5] =
        [PortView::Summary, PortView::Accounts, PortView::Positions, PortView::Orders, PortView::Activity];

    pub fn label(self) -> &'static str {
        match self {
            PortView::Summary => "Summary",
            PortView::Accounts => "Accounts",
            PortView::Positions => "Positions",
            PortView::Orders => "Orders",
            PortView::Activity => "Activity",
        }
    }

    /// The function code shown on the tab and typed on the command line.
    pub fn code(self) -> &'static str {
        match self {
            PortView::Summary => "PORT",
            PortView::Accounts => "ACCT",
            PortView::Positions => "POS",
            PortView::Orders => "ORD",
            PortView::Activity => "ACTV",
        }
    }
}

/// `BUY AAPL 10`, `SELL 5 MSFT LMT 410`, `BUY NVDA $500`, `TRADE AAPL`: fields for the order
/// ticket. A command only fills the ticket; orders are placed from the ticket after a preview
/// and a confirmation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TradeArgs {
    /// `Some(true)` buy, `Some(false)` sell, `None` the ticket's default.
    pub buy: Option<bool>,
    /// Market key (`AAPL US Equity`) of the security, when one was given.
    pub symbol: Option<String>,
    pub units: Option<f64>,
    /// Dollar amount (`$500`).
    pub notional: Option<f64>,
    pub limit: Option<f64>,
    pub stop: Option<f64>,
    /// `SELL ALL TSLA`: the whole position.
    pub all: bool,
}

impl TradeArgs {
    /// Parse the words after `BUY`/`SELL`/`TRADE`. Unknown words make the command invalid so
    /// a typo never fills the ticket with something unintended.
    fn parse(buy: Option<bool>, rest: &[&str]) -> Option<TradeArgs> {
        let mut t = TradeArgs { buy, ..Default::default() };
        let price =
            |s: &str| s.trim_start_matches(['@', '$']).replace(',', "").parse::<f64>().ok().filter(|x| *x > 0.0);
        let mut words = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            let w = rest[i];
            let up = w.to_ascii_uppercase();
            match up.as_str() {
                "LMT" | "LIMIT" | "@" => {
                    t.limit = Some(price(rest.get(i + 1)?)?);
                    i += 1;
                }
                "STP" | "STOP" => {
                    t.stop = Some(price(rest.get(i + 1)?)?);
                    i += 1;
                }
                "MKT" | "MARKET" | "SHARES" | "SHS" | "SH" => {}
                _ if up.starts_with('@') => t.limit = Some(price(&up)?),
                _ if up.starts_with('$') => t.notional = Some(price(&up)?),
                _ if up.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '.') => {
                    if t.units.is_some() {
                        return None;
                    }
                    t.units = Some(price(&up)?);
                }
                _ => words.push(w),
            }
            i += 1;
        }
        if t.units.is_some() && t.notional.is_some() {
            return None;
        }
        // `ALL` means the whole position when another word names the security (`ALL` alone
        // is Allstate's ticker).
        if words.len() > 1
            && let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case("ALL"))
        {
            if t.buy == Some(true) || t.units.is_some() || t.notional.is_some() {
                return None;
            }
            words.remove(i);
            t.all = true;
            t.buy = Some(false);
        }
        if !words.is_empty() {
            let (sec, used) = Security::resolve_tokens(&words)?;
            if used != words.len() {
                return None;
            }
            t.symbol = Some(sec.key());
        }
        Some(t)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A market key, e.g. `BTCUSD Curncy`.
    Symbol(String),
    SymbolFunction(String, Function),
    Function(Function),
}

/// Quick range: span of history plus the interval used to show it (mirrors common charting presets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    D1,
    D5,
    M1,
    M3,
    M6,
    Ytd,
    Y1,
    Y5,
    All,
}

impl Range {
    pub const ALL: [Range; 9] =
        [Range::D1, Range::D5, Range::M1, Range::M3, Range::M6, Range::Ytd, Range::Y1, Range::Y5, Range::All];

    pub fn label(self) -> &'static str {
        match self {
            Range::D1 => "1D",
            Range::D5 => "5D",
            Range::M1 => "1M",
            Range::M3 => "3M",
            Range::M6 => "6M",
            Range::Ytd => "YTD",
            Range::Y1 => "1Y",
            Range::Y5 => "5Y",
            Range::All => "ALL",
        }
    }

    pub fn timeframe(self) -> Timeframe {
        match self {
            Range::D1 => Timeframe::M1,
            Range::D5 => Timeframe::M5,
            Range::M1 => Timeframe::M30,
            Range::M3 => Timeframe::H1,
            Range::M6 => Timeframe::H4,
            Range::Ytd | Range::Y1 => Timeframe::D1,
            Range::Y5 => Timeframe::W1,
            Range::All => Timeframe::MN1,
        }
    }

    /// Approximate seconds of history to show (`None` = everything).
    pub fn span_secs(self, now: i64) -> Option<i64> {
        const D: i64 = 86_400;
        Some(match self {
            Range::D1 => D,
            Range::D5 => 5 * D,
            Range::M1 => 30 * D,
            Range::M3 => 91 * D,
            Range::M6 => 182 * D,
            Range::Ytd => {
                use chrono::Datelike;
                let y = chrono::DateTime::from_timestamp(now, 0).unwrap_or_default().year();
                now - chrono::NaiveDate::from_ymd_opt(y, 1, 1)?.and_hms_opt(0, 0, 0)?.and_utc().timestamp()
            }
            Range::Y1 => 365 * D,
            Range::Y5 => 5 * 365 * D,
            Range::All => return None,
        })
    }

    fn parse(s: &str) -> Option<Range> {
        Range::ALL.into_iter().find(|r| r.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Function {
    Timeframe(Timeframe),
    ChartType(ChartType),
    /// Add or remove an indicator (any code from the catalog: `RSI`, `BB 20 2.5`, `ICHIMOKU`).
    Indicator(IndicatorSpec),
    /// Open the indicator picker, pre-filled with a search (`IND`, `IND VOLUME`).
    IndicatorPicker(String),
    ClearIndicators,
    Compare(Option<String>),
    LogScale,
    Range(Range),
    Draw(Tool),
    ClearDrawings,
    Magnet,
    /// Congress trades: feed when used alone, per-ticker list after a symbol.
    Congress,
    Politician(String),
    Track(String),
    Untrack(String),
    Alerts,
    Refresh,
    Watchlist,
    WatchAdd(Vec<String>),
    WatchRemove(Vec<String>),
    /// `LAYOUT 1|2|2H|3|4|6` preset, `LAYOUT <name>` switch, or a layout subcommand
    /// (`SAVE`, `SAVEAS`, `NEW`, `RENAME`, `DUP`, `DEL`, `EXPORT`, `IMPORT`).
    Layout(String),
    /// List saved layouts.
    Layouts,
    /// Add a panel (`NEW NEWS`, `NEW BOOK BTCUSD`); empty adds a chart.
    NewPanel(String),
    /// Split the focused pane, adding a panel to the right (or below).
    Split {
        down: bool,
        panel: String,
    },
    PopOut,
    /// Put the focused pane in a colour link group (`None` = unlink).
    Link(Option<crate::layouts::Link>),
    /// Headlines (for a security when used after one).
    News,
    /// Order book depth (levels).
    OrderBook(Option<usize>),
    /// Option chain monitor (Bloomberg OMON).
    Options,
    /// `SIDEBAR LEFT|RIGHT|TOGGLE|SHOW|HIDE|SWAP`.
    Sidebar(String),
    Settings,
    /// Check for a newer release of the app.
    Update,
    /// `THEME LIGHT|DARK|SYSTEM|COMPACT|REGULAR|COMFORTABLE`.
    Theme(String),
    /// Rename the focused tab (empty resets).
    Title(String),
    ClosePane,
    Help,
    /// World equity indices / markets overview.
    Wei,
    /// US Treasury yield curve.
    Curve,
    /// Security search across asset classes.
    Search(String),
    /// Brokerage portfolio view, with an optional subcommand (`REFRESH`, `CONNECT`, `SETUP`).
    Portfolio(PortView, String),
    /// Open the order ticket (`BUY`, `SELL`, `TRADE`).
    Trade(TradeArgs),
    /// Switch between the demo account and live accounts (`DEMO`, `MODE LIVE`); `true` = live.
    AccountMode(bool),
    /// sabr series: `CAST` a metric over a window for the (resolved) entity. `entity` is
    /// `None` when no entity token preceded `CAST` (the focused pane's entity is not yet a
    /// sabr pane): the app reports it as unsupported rather than guessing.
    Cast {
        entity: Option<SabrEntity>,
        metric: Option<String>,
        window: Option<Window>,
    },
    /// sabr projection: `<ENTITY> PROJ ROS|NEXT [MODEL]`.
    Proj {
        entity: Option<SabrEntity>,
        horizon: Horizon,
        model: Option<String>,
    },
    /// sabr backtest of a named model: `BTST MARCEL`.
    Btst(String),
    /// sabr roster for a team abbreviation (`NYM ROST`); stub arm in this wave.
    Rost(String),
    /// sabr watchlist: track a resolved player entity (alerts fire on refresh).
    TrackPlayer(SabrEntity),
    /// sabr watchlist: stop tracking.
    UntrackPlayer(SabrEntity),
}

// ---------------------------------------------------------------------------
// sabr grammar (03-spec §4.7): `<ENTITY>[.BAT|.PIT] <FUNCTION> [ARGS] <GO>`
// ---------------------------------------------------------------------------

/// A resolved sabr entity: canonical key (`p:660271:bat`, `t:160`) plus a display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SabrEntity {
    pub key: String,
    pub label: String,
}

/// Projection horizon: rest-of-season or next season.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Horizon {
    RoS,
    Next,
}

/// One ranked picker row: a catalog hit for an ambiguous name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogHit {
    pub key_uuid: String,
    pub mlbam: Option<u32>,
    /// Display name (`First Last`).
    pub name: String,
    /// Role-neutral span weight (seasons), as the catalog ranks.
    pub weight: u32,
    /// Catalog role evidence (batting / pitching), for the picker's positions column.
    pub bat: bool,
    pub pit: bool,
}

fn hit_name(row: &CatRow) -> String {
    match &row.name_first {
        Some(f) => format!("{f} {}", row.name_last),
        None => row.name_last.clone(),
    }
}

/// A parse result that is not a plain command: an ambiguous entity opens the ranked picker
/// (`CommandOutcome::PickList`) instead of a command. The app shows the rows in an
/// `EntityPick` pane; accepting rewrites the ambiguous token to a direct entity key and
/// re-executes the command.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    Command(Command),
    PickList(Vec<CatalogHit>),
}

/// Parse one command line, keeping structured non-command outcomes (entity pickers).
pub fn parse_outcome(input: &str) -> Option<CommandOutcome> {
    let tokens: Vec<&str> = input.split_whitespace().filter(|t| !t.eq_ignore_ascii_case("<GO>")).collect();
    let (&head, rest) = tokens.split_first()?;
    let keyed = match rest {
        [k, ..] if YellowKey::parse(k).is_some() => true,
        [us, k, ..] if us.eq_ignore_ascii_case("US") && YellowKey::parse(k).is_some() => true,
        _ => false,
    };
    if !keyed {
        // TRACK/UNTRACK resolve against the player catalog first: a resolvable player (or a
        // picker) is a sabr watchlist command; unresolvable names fall back to politicians.
        if head.eq_ignore_ascii_case("TRACK") || head.eq_ignore_ascii_case("UNTRACK") {
            match resolve_entity_token(&rest.join(" "), true) {
                Resolution::Resolved(e) if head.eq_ignore_ascii_case("UNTRACK") => {
                    return Some(CommandOutcome::Command(Command::Function(Function::UntrackPlayer(e))));
                }
                Resolution::Resolved(e) => {
                    return Some(CommandOutcome::Command(Command::Function(Function::TrackPlayer(e))));
                }
                Resolution::Pick(hits) => return Some(CommandOutcome::PickList(hits)),
                Resolution::Unknown => {}
            }
        }
        if let Some(f) = parse_function(head, rest) {
            return Some(CommandOutcome::Command(Command::Function(f)));
        }
        // `NYM ROST`: a team abbreviation with the roster function is sabr team grammar.
        if rest.len() == 1 && rest[0].eq_ignore_ascii_case("ROST") && team_abbrev(&head.to_ascii_uppercase()) {
            return Some(CommandOutcome::Command(Command::Function(Function::Rost(head.to_ascii_uppercase()))));
        }
        // `<ENTITY> <SABR FUNCTION>`: resolve the entity token, then the function word. A
        // bare entity token resolves by name only with a role suffix (`OHTANI.PIT`), so
        // finance tickers keep their meaning; with a function word names resolve too.
        match resolve_entity_token(head, !rest.is_empty() || head.contains('.')) {
            Resolution::Resolved(e) => {
                return match rest.split_first() {
                    None => Some(CommandOutcome::Command(Command::Function(Function::Cast {
                        entity: Some(e),
                        metric: None,
                        window: None,
                    }))),
                    Some((&f, args)) => parse_sabr_function(f, args).map(|mut func| {
                        set_sabr_entity(&mut func, e);
                        CommandOutcome::Command(Command::Function(func))
                    }),
                };
            }
            Resolution::Pick(hits) => return Some(CommandOutcome::PickList(hits)),
            // Team grammar is recognised only where a sabr function follows (`NYM ROST`).
            Resolution::Unknown if rest.len() == 1 && rest[0].eq_ignore_ascii_case("ROST") => {
                let team = head.to_ascii_uppercase();
                if team_abbrev(&team) {
                    return Some(CommandOutcome::Command(Command::Function(Function::Rost(team))));
                }
            }
            Resolution::Unknown => {}
        }
    }
    // Plain finance security / function path.
    // `TSLA SELL ALL`: the security is already named, so `ALL` is the whole position.
    let (sec, used) = Security::resolve_tokens(&tokens)?;
    match tokens[used..].split_first() {
        None => Some(CommandOutcome::Command(Command::Symbol(sec.key()))),
        Some((&f, args))
            if ["SELL", "TRADE"].iter().any(|h| f.eq_ignore_ascii_case(h))
                && args.iter().any(|a| a.eq_ignore_ascii_case("ALL")) =>
        {
            let rest: Vec<&str> = args.iter().copied().filter(|a| !a.eq_ignore_ascii_case("ALL")).collect();
            match parse_function(f, &rest)? {
                Function::Trade(mut t) if t.symbol.is_none() && t.units.is_none() && t.notional.is_none() => {
                    t.all = true;
                    t.buy = Some(false);
                    Some(CommandOutcome::Command(Command::SymbolFunction(sec.key(), Function::Trade(t))))
                }
                _ => None,
            }
        }
        Some((&f, args)) => {
            parse_function(f, args).map(|func| CommandOutcome::Command(Command::SymbolFunction(sec.key(), func)))
        }
    }
}

/// Fill the entity of an entity-headed sabr function.
fn set_sabr_entity(f: &mut Function, e: SabrEntity) {
    match f {
        Function::Cast { entity, .. } | Function::Proj { entity, .. } => *entity = Some(e),
        _ => {}
    }
}

/// sabr functions that may follow an entity token: `CAST`, `PROJ`, `ROST`.
fn parse_sabr_function(head: &str, rest: &[&str]) -> Option<Function> {
    if head.eq_ignore_ascii_case("CAST") {
        let mut f = parse_function("CAST", rest)?;
        if let Function::Cast { entity, .. } = &mut f {
            *entity = None;
        }
        return Some(f);
    }
    if head.eq_ignore_ascii_case("PROJ") {
        return parse_function("PROJ", rest);
    }
    None
}

enum Resolution {
    Resolved(SabrEntity),
    Pick(Vec<CatalogHit>),
    Unknown,
}

/// Bundled sample root (`sabr-data/sample`): `SABR_SAMPLE_DIR` overrides, then the cwd, then
/// the repo layout relative to this crate (so tests and `cargo run` both find it).
pub fn sample_root() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("SABR_SAMPLE_DIR") {
        return std::path::PathBuf::from(d);
    }
    if std::path::Path::new("sabr-data/sample/catalog.csv").is_file() {
        return std::path::PathBuf::from("sabr-data/sample");
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sabr-data/sample")
}

/// The player catalog, loaded (once, lazily) from the bundled sample. `None` when the bundle
/// is unavailable — sabr tokens then fail to resolve and finance parsing continues.
pub(crate) fn catalog() -> Option<&'static Catalog> {
    static CATALOG: std::sync::LazyLock<Option<Catalog>> = std::sync::LazyLock::new(|| {
        Catalog::open(sample_root()).map_err(|e| tracing::warn!("sabr catalog unavailable: {e}")).ok()
    });
    CATALOG.as_ref()
}

/// Parse an entity token: `<name|MLBAM id>[.BAT|.PIT]`. MLBAM ids resolve through the
/// catalog; names rank through it. A clearly dominant hit resolves; otherwise the ranked
/// picker opens.
fn resolve_entity_token(token: &str, allow_name: bool) -> Resolution {
    let token = token.trim();
    let (name, role) = match token.rsplit_once('.') {
        Some((head, suffix)) => match suffix.to_ascii_uppercase().as_str() {
            "BAT" => (head, Some(sabr_model::entity::Role::Bat)),
            "PIT" => (head, Some(sabr_model::entity::Role::Pit)),
            // A dot that is not a role suffix is a finance ticker; not ours.
            _ => return Resolution::Unknown,
        },
        None => (token, None),
    };
    let Some(cat) = catalog() else { return Resolution::Unknown };
    // An entity key produced by an earlier picker accept (`p:660271:bat`) resolves exactly
    // through the catalog, so a rewritten command can never open another picker.
    if let Some(e) = direct_key(cat, token) {
        return Resolution::Resolved(e);
    }
    // MLBAM id.
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        let Ok(id) = name.parse::<u32>() else { return Resolution::Unknown };
        let Some(row) = cat.by_mlbam(id) else { return Resolution::Unknown };
        return Resolution::Resolved(SabrEntity {
            key: format!("p:{}:{}", id, pick_role(cat, row, role).as_str()),
            label: hit_name(row),
        });
    }
    // Name search (accent/suffix folding is the catalog's job). Bare tokens that were not
    // introduced by a sabr function or role suffix never resolve by name — finance tickers
    // share the same namespace (`ACT`, `LINE`) and keep their upstream meaning.
    if !allow_name {
        return Resolution::Unknown;
    }
    let role_hint = role.map(|r| match r {
        sabr_model::entity::Role::Bat => sabr_data::Role::Batter,
        sabr_model::entity::Role::Pit => sabr_data::Role::Pitcher,
    });
    // Role filtering is evidence-based; a suffix on a row without role evidence in the
    // bundled sample is a fallback to the unfiltered search, not a dead end.
    let mut hits = cat.search_ranked(name, role_hint);
    if hits.is_empty() && role_hint.is_some() {
        hits = cat.search_ranked(name, None);
    }
    match hits.as_slice() {
        [] => Resolution::Unknown,
        // Exact name or full-name match only (rank ≤ 1): substring hits are ticker territory.
        [(row, rank)] if *rank <= 1 => Resolution::Resolved(entity_of(cat, row, role)),
        [(top, rank), (second, _), ..] if clearly_dominant(cat, top, second, *rank) => {
            Resolution::Resolved(entity_of(cat, top, role))
        }
        _ => Resolution::Pick(
            hits.iter()
                .map(|(row, _)| {
                    let (bat, pit) = cat.roles(row);
                    CatalogHit {
                        key_uuid: row.key_uuid.clone(),
                        mlbam: row.key_mlbam,
                        name: hit_name(row),
                        weight: cat.weight(row),
                        bat,
                        pit,
                    }
                })
                .collect(),
        ),
    }
}

/// A previously accepted entity key (`p:660271:bat`) resolves exactly: the catalog row for
/// `p:<MLBAM id>:bat|pit`, with the row's display label. Anything else is not a key and
/// feeds name/finance parsing.
fn direct_key(cat: &Catalog, token: &str) -> Option<SabrEntity> {
    let (id, role) = token.strip_prefix("p:")?.rsplit_once(':')?;
    if !(matches!(role, "bat" | "pit") && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let row = cat.by_mlbam(id.parse().ok()?)?;
    Some(SabrEntity { key: token.to_owned(), label: hit_name(row) })
}

/// The token an entity picker rewrites: the words after `TRACK`/`UNTRACK` (which can be two
/// words, `TRACK TATIS JR`), else the leading entity token (`DIAZ`, `WHEELER.PIT`).
pub fn ambiguous_token(input: &str) -> String {
    match input.split_whitespace().collect::<Vec<_>>().as_slice() {
        [head, rest @ ..] if matches!(head.to_ascii_uppercase().as_str(), "TRACK" | "UNTRACK") => rest.join(" "),
        [head, ..] => (*head).to_owned(),
        [] => String::new(),
    }
}

/// Is the top hit clearly dominant over the runner-up (same-name minors/system players)? A
/// name resolves only when it is an exact-namespace hit (rank ≤ 1) whose role-neutral weight
/// is not closely matched.
fn clearly_dominant(cat: &Catalog, top: &CatRow, second: &CatRow, rank: u8) -> bool {
    rank <= 1 && cat.weight(top) >= 4 * cat.weight(second).max(1)
}

fn entity_of(cat: &Catalog, row: &CatRow, role: Option<sabr_model::entity::Role>) -> SabrEntity {
    let role = pick_role(cat, row, role);
    SabrEntity { key: format!("p:{}:{}", row.key_mlbam.unwrap_or_default(), role.as_str()), label: hit_name(row) }
}

/// Role for an entity key: the explicit `.BAT`/`.PIT` suffix wins; otherwise the row's
/// catalog role evidence, defaulting to bat (a two-way player's bat identity).
fn pick_role(cat: &Catalog, row: &CatRow, hint: Option<sabr_model::entity::Role>) -> sabr_model::entity::Role {
    if let Some(r) = hint {
        return r;
    }
    let (bat, pit) = cat.roles(row);
    if pit && !bat { sabr_model::entity::Role::Pit } else { sabr_model::entity::Role::Bat }
}

/// The 30 MLB team abbreviations (`NYM ROST`).
fn team_abbrev(s: &str) -> bool {
    matches!(
        s,
        "ARI"
            | "AZ"
            | "ATL"
            | "BAL"
            | "BOS"
            | "CHC"
            | "CWS"
            | "CIN"
            | "CLE"
            | "COL"
            | "DET"
            | "HOU"
            | "KC"
            | "LAA"
            | "LAD"
            | "MIA"
            | "MIL"
            | "MIN"
            | "NYM"
            | "NYY"
            | "OAK"
            | "PHI"
            | "PIT"
            | "SD"
            | "SEA"
            | "SF"
            | "STL"
            | "TB"
            | "TEX"
            | "TOR"
            | "WSN"
    )
}

/// Window labels: `PA50`, `BF30`, `G10`, `AB100`, `BIP50`, `P100` trailing event counts, or
/// `SEASON` (season-to-date). A bare integer is a trailing-PA window (`CAST XWOBA 50`).
fn window_from_token(word: &str) -> Option<Window> {
    if word.eq_ignore_ascii_case("SEASON") {
        let year = chrono::Datelike::year(&chrono::Utc::now());
        return Some(Window::Season(u16::try_from(year).ok()?));
    }
    if word.bytes().all(|b| b.is_ascii_digit()) {
        return word.parse::<u32>().ok().filter(|n| *n > 0).map(|n| Window::Events { unit: Unit::Pa, n });
    }
    let split = word.char_indices().find(|(_, c)| c.is_ascii_digit())?;
    let (prefix, digits) = word.split_at(split.0);
    let n: u32 = digits.parse().ok()?;
    let unit = match prefix.to_ascii_uppercase().as_str() {
        "PA" => Unit::Pa,
        "BF" => Unit::Bf,
        "AB" => Unit::Ab,
        "BIP" => Unit::Bip,
        "G" => Unit::Game,
        "P" => Unit::Pitch,
        _ => return None,
    };
    (n > 0).then_some(Window::Events { unit, n })
}

/// Registry metric id for a typed code: `XWOBA` → `bat:xwoba`, `VELO` → `pit:velocity`. The
/// role is part of the id; a two-way player picks the entity's side at derive time.
fn metric_id(word: &str) -> Option<String> {
    let up = word.to_ascii_uppercase();
    let code = match up.as_str() {
        "VELO" => "VELOCITY",
        "LA" => "LAUNCH_ANGLE",
        "BA" | "AVERAGE" => "AVG",
        "WHIFF" => "WHIFF_RATE",
        "K%" | "KPCT" | "KRATE" => "K_RATE",
        "BB%" | "BBPCT" | "BBRATE" => "BB_RATE",
        "CHASERATE" => "CHASE_RATE",
        "HARDHIT" | "HHHR" | "HH" => "HARD_HIT_RATE",
        "BARRELRATE" => "BARREL_RATE",
        "BATSPEED" | "BATSPD" => "BAT_SPEED",
        "SPIN" => "SPIN_RATE",
        other => other,
    };
    registry()
        .iter()
        .find(|m| m.id.split_once(':').is_some_and(|(_, name)| name.eq_ignore_ascii_case(code)))
        .map(|m| m.id.to_owned())
}

fn num<T: std::str::FromStr>(t: Option<&&str>) -> Option<T> {
    t.and_then(|s| s.parse().ok())
}

fn parse_function(head: &str, rest: &[&str]) -> Option<Function> {
    let up = head.to_ascii_uppercase();
    let words = || rest.iter().map(|s| s.to_ascii_uppercase()).collect::<Vec<_>>();
    let joined = || rest.join(" ");
    Some(match up.as_str() {
        "CANDLE" | "CANDLES" => Function::ChartType(ChartType::Candles),
        "BAR" | "BARS" | "OHLC" => Function::ChartType(ChartType::Bars),
        "LINE" => Function::ChartType(ChartType::Line),
        "AREA" | "MOUNTAIN" => Function::ChartType(ChartType::Area),
        // `IND RSI 21` adds directly; anything else searches the picker.
        "IND" | "INDICATOR" | "INDICATORS" | "STUDY" | "STUDIES" => {
            match rest.split_first().and_then(|(code, args)| IndicatorSpec::parse(code, args)) {
                Some(spec) => Function::Indicator(spec),
                None => Function::IndicatorPicker(joined()),
            }
        }
        "CLEAR" | "CLR" => match words().first().map(String::as_str) {
            Some("DRAW") | Some("DRAWINGS") => Function::ClearDrawings,
            _ => Function::ClearIndicators,
        },
        "CMP" | "COMPARE" | "COMP" => Function::Compare(rest.first().map(|s| s.to_ascii_uppercase())),
        "LOG" => Function::LogScale,
        "TL" | "TREND" | "TRENDLINE" => Function::Draw(Tool::Trendline),
        "HL" | "HLINE" => Function::Draw(Tool::Horizontal),
        "VL" | "VLINE" => Function::Draw(Tool::Vertical),
        "FIB" => Function::Draw(Tool::Fib),
        "MAGNET" | "MAG" => Function::Magnet,
        "CONG" | "CONGRESS" => Function::Congress,
        "POL" | "PROFILE" => Function::Politician(joined()),
        "TRACK" => Function::Track(joined()),
        "UNTRACK" => Function::Untrack(joined()),
        "ALRT" | "ALERTS" | "ALERT" => Function::Alerts,
        "REFRESH" | "SYNC" => Function::Refresh,
        "W" | "WL" | "WATCH" | "WATCHLIST" | "MON" => match words().first().map(String::as_str) {
            Some("ADD") => Function::WatchAdd(words()[1..].to_vec()),
            Some("DEL") | Some("RM") | Some("REMOVE") => Function::WatchRemove(words()[1..].to_vec()),
            _ => Function::Watchlist,
        },
        "LAYOUT" | "GRID" | "LP" => Function::Layout(joined()),
        "LAYOUTS" => Function::Layouts,
        "NEW" | "ADD" => Function::NewPanel(joined()),
        "SPLIT" | "SPLITV" | "SPLITH" => {
            let w = words();
            let down = up == "SPLITH" || matches!(w.first().map(String::as_str), Some("DOWN" | "H" | "BELOW"));
            let skip =
                usize::from(matches!(w.first().map(String::as_str), Some("DOWN" | "H" | "BELOW" | "RIGHT" | "V")));
            Function::Split { down, panel: rest[skip.min(rest.len())..].join(" ") }
        }
        "POP" | "POPOUT" | "UNDOCK" => Function::PopOut,
        "LINK" => Function::Link(match words().first().map(String::as_str) {
            None | Some("OFF" | "NONE" | "0") => None,
            Some(c) => Some(crate::layouts::Link::ALL.into_iter().find(|l| l.label().eq_ignore_ascii_case(c))?),
        }),
        "N" | "CN" | "NEWS" | "TOP" => Function::News,
        "OB" | "BOOK" | "DEPTH" | "DOM" => Function::OrderBook(num(rest.first())),
        "OMON" | "OMST" | "OPT" | "OPTIONS" | "CHAIN" => Function::Options,
        "SIDEBAR" | "SB" => Function::Sidebar(words().join(" ")),
        "PREFS" | "SETTINGS" | "PDF" | "CONFIG" => Function::Settings,
        "UPDATE" | "UPDATES" => Function::Update,
        "THEME" => Function::Theme(words().join(" ")),
        "TITLE" | "RENAME" => Function::Title(joined()),
        "CLOSE" => Function::ClosePane,
        "HELP" | "?" | "H" => Function::Help,
        "WEI" | "MARKETS" | "WORLD" => Function::Wei,
        "GC" | "YCRV" | "CURVE" | "YC" => Function::Curve,
        "SECF" | "SEARCH" | "FIND" => Function::Search(joined()),
        "PORT" | "PORTFOLIO" | "PRTU" => Function::Portfolio(PortView::Summary, words().join(" ")),
        "ACCT" | "ACCOUNTS" | "ACCTS" | "BAL" | "BALANCES" => {
            Function::Portfolio(PortView::Accounts, words().join(" "))
        }
        "POS" | "POSITIONS" | "HOLDINGS" => Function::Portfolio(PortView::Positions, words().join(" ")),
        "ORD" | "ORDERS" | "BLTR" => Function::Portfolio(PortView::Orders, words().join(" ")),
        "ACTV" | "ACTIVITY" | "TXNS" | "TRANSACTIONS" => Function::Portfolio(PortView::Activity, words().join(" ")),
        "BROKER" | "BROKERAGE" | "SNAPTRADE" => Function::Portfolio(PortView::Summary, "SETUP".into()),
        "BUY" => Function::Trade(TradeArgs::parse(Some(true), rest)?),
        "CAST" => {
            // `CAST [METRIC] [WINDOW]`, index-free: a window label, a registry metric code,
            // or a bare trailing-PA count (`CAST XWOBA 50`).
            let mut metric: Option<String> = None;
            let mut window: Option<Window> = None;
            for w in words() {
                if window.is_none()
                    && let Some(win) = window_from_token(&w)
                {
                    window = Some(win);
                    continue;
                }
                if metric.is_none()
                    && let Some(id) = metric_id(&w)
                {
                    metric = Some(id);
                    continue;
                }
                return None; // an unknown argument invalidates the command
            }
            Function::Cast { entity: None, metric, window }
        }
        "PROJ" | "PROJECTION" => {
            // `PROJ ROS|NEXT [MODEL]`; `PROJ [MODEL]` defaults to rest-of-season.
            let mut words = words();
            let horizon = match words.first().map(String::as_str) {
                Some("ROS" | "SEASON") => {
                    words.remove(0);
                    Horizon::RoS
                }
                Some("NEXT") => {
                    words.remove(0);
                    Horizon::Next
                }
                _ => Horizon::RoS,
            };
            Function::Proj { entity: None, horizon, model: (!words.is_empty()).then(|| words.join(" ")) }
        }
        "BTST" | "BACKTEST" => Function::Btst(words().join(" ")),
        "ROST" | "ROSTER" => Function::Rost(String::new()),
        "SELL" => Function::Trade(TradeArgs::parse(Some(false), rest)?),
        "TRADE" | "TICKET" | "EMSX" => Function::Trade(TradeArgs::parse(None, rest)?),
        "DEMO" | "PAPER" if rest.is_empty() => Function::AccountMode(false),
        "MODE" => match words().join(" ").as_str() {
            "DEMO" | "PAPER" => Function::AccountMode(false),
            "LIVE" | "REAL" => Function::AccountMode(true),
            _ => return None,
        },
        "RANGE" => Function::Range(Range::parse(rest.first()?)?),
        // Timeframes win where spellings overlap (`1D`, `1M`); otherwise try a range (`5D`, `YTD`),
        // then the indicator catalog (`RSI`, `BB 20 2.5`), then a sabr window label
        // (`PA50`, `SEASON`), then a bare sabr metric code (`XWOBA`).
        _ => {
            return head
                .parse::<Timeframe>()
                .ok()
                .map(Function::Timeframe)
                .or_else(|| Range::parse(&up).map(Function::Range))
                .or_else(|| IndicatorSpec::parse(head, rest).map(Function::Indicator))
                .or_else(|| {
                    // Bare heads only: `660271 CAST …` is a sabr entity, not a PA window.
                    (rest.is_empty())
                        .then(|| {
                            window_from_token(&up).map(|window| Function::Cast {
                                entity: None,
                                metric: None,
                                window: Some(window),
                            })
                        })
                        .flatten()
                })
                .or_else(|| {
                    if rest.is_empty() {
                        metric_id(head).map(|metric| Function::Cast {
                            entity: None,
                            metric: Some(metric),
                            window: None,
                        })
                    } else {
                        None
                    }
                });
        }
    })
}

/// Parse one command line. Returns `None` for empty or unrecognised input; ambiguous sabr
/// entities (a picker, not a command) also read as `None` here — use
/// [`parse_outcome`] for those.
pub fn parse(input: &str) -> Option<Command> {
    match parse_outcome(input) {
        Some(CommandOutcome::Command(c)) => Some(c),
        _ => None,
    }
}

/// One-line live interpretation of partially typed input, shown under the command line.
pub fn hint(input: &str) -> String {
    match parse_outcome(input) {
        None if input.trim().is_empty() => "Type a ticker and press <GO> (Enter).  HELP for functions.".into(),
        None => "Unrecognised command".into(),
        Some(CommandOutcome::PickList(hits)) => {
            let names: Vec<String> =
                hits.iter().take(3).map(|h| format!("{} ({})", h.name, h.mlbam.unwrap_or(0))).collect();
            format!("Ambiguous name — pick from {} matches: {}…", hits.len(), names.join(", "))
        }
        Some(CommandOutcome::Command(Command::Symbol(s))) => match Security::parse(&s).and_then(|x| x.name()) {
            Some(name) => format!("Load {s} — {name}"),
            None => format!("Load {s}"),
        },
        Some(CommandOutcome::Command(Command::SymbolFunction(s, f))) => format!("{s}: {}", describe(&f)),
        Some(CommandOutcome::Command(Command::Function(f))) => describe(&f),
    }
}

fn describe(f: &Function) -> String {
    match f {
        Function::Timeframe(tf) => format!(
            "Interval {}",
            match tf {
                Timeframe::M1 => "1 minute",
                Timeframe::M5 => "5 minutes",
                Timeframe::M15 => "15 minutes",
                Timeframe::M30 => "30 minutes",
                Timeframe::H1 => "1 hour",
                Timeframe::H4 => "4 hours",
                Timeframe::D1 => "1 day",
                Timeframe::W1 => "1 week",
                Timeframe::MN1 => "1 month",
            }
        ),
        Function::ChartType(c) => format!("Chart type {}", c.label()),
        Function::Indicator(i) => format!("Toggle {} — {}", i.label(), i.name()),
        Function::IndicatorPicker(q) if q.is_empty() => "Browse indicators".into(),
        Function::IndicatorPicker(q) => format!("Search indicators for \"{q}\""),
        Function::ClearIndicators => "Remove all indicators".into(),
        Function::Compare(Some(s)) => format!("Compare with {s}"),
        Function::Compare(None) => "Remove comparison".into(),
        Function::LogScale => "Toggle log scale".into(),
        Function::Range(r) => format!("Show {} at {}", r.label(), r.timeframe()),
        Function::Draw(t) => format!("Draw {}", t.label()),
        Function::ClearDrawings => "Remove all drawings".into(),
        Function::Magnet => "Toggle magnet (snap to OHLC)".into(),
        Function::Congress => "Congressional trades".into(),
        Function::Politician(n) => format!("Politician profile: {n}"),
        Function::Track(n) => format!("Track {n} (alert on new filings)"),
        Function::Untrack(n) => format!("Stop tracking {n}"),
        Function::Alerts => "Alerts".into(),
        Function::Refresh => "Refresh congressional disclosures now".into(),
        Function::Watchlist => "Watchlist".into(),
        Function::WatchAdd(s) => format!("Add {} to watchlist", s.join(", ")),
        Function::WatchRemove(s) => format!("Remove {} from watchlist", s.join(", ")),
        Function::Layout(l) if l.trim().is_empty() => "Saved layouts".into(),
        Function::Layout(l) => format!("Layout {l}"),
        Function::Layouts => "Saved layouts".into(),
        Function::NewPanel(p) if p.trim().is_empty() => "Add a chart pane".into(),
        Function::NewPanel(p) => format!("Add panel: {p}"),
        Function::Split { down, panel } => {
            format!(
                "Split {} with {}",
                if *down { "down" } else { "right" },
                if panel.is_empty() { "a chart" } else { panel }
            )
        }
        Function::PopOut => "Pop the focused pane out into its own window".into(),
        Function::Link(Some(l)) => format!("Link focused pane to the {} group", l.label()),
        Function::Link(None) => "Unlink focused pane".into(),
        Function::News => "News headlines".into(),
        Function::OrderBook(_) => "Order book (depth)".into(),
        Function::Options => "Option chain: calls and puts by strike, IV and greeks".into(),
        Function::Sidebar(s) if s.is_empty() => "Toggle the sidebar".into(),
        Function::Sidebar(s) => format!("Sidebar {}", s.to_lowercase()),
        Function::Settings => "Settings: theme, fonts, density, hotkeys, panels, layouts".into(),
        Function::Update => "Check for updates".into(),
        Function::Theme(t) => format!("Theme {t}"),
        Function::Title(t) => format!("Rename tab to {t:?}"),
        Function::ClosePane => "Close focused pane".into(),
        Function::Help => "Help".into(),
        Function::Wei => "World markets overview".into(),
        Function::Curve => "US Treasury yield curve".into(),
        Function::Search(q) => format!("Search securities: {q}"),
        Function::Portfolio(v, a) if a.is_empty() => match v {
            PortView::Summary => "Brokerage portfolio: value, cash, P&L and holdings (SnapTrade)".into(),
            PortView::Accounts => "Brokerage accounts and balances".into(),
            PortView::Positions => "Brokerage positions with unrealised P&L".into(),
            PortView::Orders => "Brokerage orders (last 30 days)".into(),
            PortView::Activity => "Brokerage activity: trades, dividends, transfers".into(),
        },
        Function::Portfolio(_, a) => match a.as_str() {
            "REFRESH" | "SYNC" => "Reload brokerage accounts now".into(),
            "CONNECT" | "LINK" => "Connect a brokerage in your browser (SnapTrade, read-only)".into(),
            _ => "Brokerage connection settings".into(),
        },
        Function::Trade(t) => {
            let side = match t.buy {
                Some(true) => "Buy",
                Some(false) => "Sell",
                None => "Trade",
            };
            let mut s = format!("Order ticket: {side}");
            if let Some(u) = t.units {
                s += &format!(" {}", tr_broker::trading::fmt_units(u));
            }
            if let Some(n) = t.notional {
                s += &format!(" ${n:.2} of");
            }
            if let Some(k) = &t.symbol {
                s += &format!(" {}", Security::parse(k).map_or(k.clone(), |x| x.ticker));
            }
            if let Some(p) = t.stop {
                s += &format!(" stop {p}");
            }
            if let Some(p) = t.limit {
                s += &format!(" limit {p}");
            }
            if t.all {
                s += " (whole position)";
            }
            s
        }
        Function::AccountMode(false) => "Switch to the DEMO account (paper trading)".into(),
        Function::AccountMode(true) => "Switch to LIVE brokerage accounts (asks first)".into(),
        Function::Cast { entity, metric, window } => {
            let what = match (metric, window) {
                (Some(m), Some(w)) => Some(format!("{m} over {}", sabr_window_label(w))),
                (Some(m), None) => Some(m.clone()),
                (None, Some(w)) => Some(format!("series over {}", sabr_window_label(w))),
                (None, None) => None,
            };
            match (entity, what) {
                (None, None) => "Cast a metric series".into(),
                (None, Some(w)) => format!("Cast {w}"),
                (Some(e), Some(w)) => format!("{e}: cast {w}"),
                (Some(e), None) => format!("{e}: cast series"),
            }
        }
        Function::Proj { entity, horizon, model } => match (entity, model) {
            (Some(e), Some(m)) => format!("{e}: project {} with {m}", horizon.label()),
            (Some(e), None) => format!("{e}: project {}", horizon.label()),
            (None, Some(m)) => format!("Project {} with {m}", horizon.label()),
            (None, None) => format!("Project {}", horizon.label()),
        },
        Function::Btst(m) if m.is_empty() => "Backtest a model".into(),
        Function::Btst(m) => format!("Backtest {m}: MAE table, coverage grid"),
        Function::Rost(team) if team.is_empty() => "Roster".into(),
        Function::Rost(team) => format!("Roster: {team}"),
        Function::TrackPlayer(e) => format!("Track {e} (alerts on refresh)"),
        Function::UntrackPlayer(e) => format!("Stop tracking {e}"),
    }
}

/// Short window label for hints (`PA50`, `SEASON`).
fn sabr_window_label(window: &Window) -> String {
    match *window {
        Window::Events { unit, n } => match unit {
            Unit::Pa => format!("PA{n}"),
            Unit::Bf => format!("BF{n}"),
            Unit::Ab => format!("AB{n}"),
            Unit::Bip => format!("BIP{n}"),
            Unit::Game => format!("G{n}"),
            Unit::Pitch => format!("P{n}"),
        },
        Window::Season(_) => "SEASON".into(),
        Window::Between { .. } => "span".into(),
    }
}

impl Horizon {
    pub fn label(self) -> &'static str {
        match self {
            Horizon::RoS => "rest of season",
            Horizon::Next => "next season",
        }
    }
}

impl std::fmt::Display for SabrEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        let sym = |s: &str| Some(Command::Symbol(s.into()));
        assert_eq!(parse("aapl"), sym("AAPL US Equity"));
        assert_eq!(parse("AAPL US EQUITY <GO>"), sym("AAPL US Equity"));
        assert_eq!(parse("brk/b"), sym("BRK.B US Equity"));
        assert_eq!(parse("BTCUSD <CRNCY>"), sym("BTCUSD Curncy"));
        assert_eq!(parse("EURUSD <CRNCY>"), sym("EURUSD Curncy"));
        assert_eq!(parse("US10Y <GOVT>"), sym("US10Y Govt"));
        assert_eq!(parse("CL1 <CMDTY>"), sym("CL1 Comdty"));
        assert_eq!(parse("SPX <INDEX>"), sym("SPX Index"));
        assert_eq!(parse("AAPL <EQUITY>"), sym("AAPL US Equity"));
        assert_eq!(parse("btcusd"), sym("BTCUSD Curncy"));
        // A yellow key makes a function-like word a ticker.
        assert_eq!(parse("LINE <EQUITY>"), sym("LINE US Equity"));
        assert_eq!(parse("LINE"), Some(Command::Function(Function::ChartType(ChartType::Line))));
        assert_eq!(parse("   "), None);
        assert_eq!(parse("123abc"), None);
    }

    #[test]
    fn timeframes_and_ranges() {
        assert_eq!(parse("5"), Some(Command::Function(Function::Timeframe(Timeframe::M5))));
        assert_eq!(parse("1W"), Some(Command::Function(Function::Timeframe(Timeframe::W1))));
        assert_eq!(parse("1M"), Some(Command::Function(Function::Timeframe(Timeframe::MN1))));
        assert_eq!(parse("1m"), Some(Command::Function(Function::Timeframe(Timeframe::M1))));
        assert_eq!(parse("YTD"), Some(Command::Function(Function::Range(Range::Ytd))));
        assert_eq!(parse("RANGE 5D"), Some(Command::Function(Function::Range(Range::D5))));
        assert_eq!(Range::Y5.timeframe(), Timeframe::W1);
    }

    #[test]
    fn functions() {
        let ind = |code: &str, params: &[f64]| {
            Some(Command::Function(Function::Indicator(IndicatorSpec::new(code, params).unwrap())))
        };
        assert_eq!(parse("rsi"), ind("RSI", &[14.0]));
        assert_eq!(parse("SMA 50"), ind("SMA", &[50.0]));
        assert_eq!(parse("bb 20 2.5"), ind("BB", &[20.0, 2.5]));
        assert_eq!(parse("MACD"), ind("MACD", &[12.0, 26.0, 9.0]));
        assert_eq!(parse("ichimoku"), ind("ICHIMOKU", &[]));
        assert_eq!(parse("IND STOCH 5 3 3"), ind("STOCH", &[5.0, 3.0, 3.0]));
        assert_eq!(parse("IND"), Some(Command::Function(Function::IndicatorPicker(String::new()))));
        assert_eq!(parse("study volume"), Some(Command::Function(Function::IndicatorPicker("volume".into()))));
        let Some(Command::SymbolFunction(_, Function::Indicator(adx))) = parse("AAPL ADX") else { panic!("AAPL ADX") };
        assert_eq!(adx.label(), "ADX 14 14");
        assert_eq!(parse("CMP msft"), Some(Command::Function(Function::Compare(Some("MSFT".into())))));
        assert_eq!(parse("CMP"), Some(Command::Function(Function::Compare(None))));
        assert_eq!(parse("FIB"), Some(Command::Function(Function::Draw(Tool::Fib))));
        assert_eq!(parse("CLEAR DRAW"), Some(Command::Function(Function::ClearDrawings)));
        assert_eq!(parse("POL Nancy Pelosi"), Some(Command::Function(Function::Politician("Nancy Pelosi".into()))));
        assert_eq!(
            parse("WL ADD msft nvda"),
            Some(Command::Function(Function::WatchAdd(vec!["MSFT".into(), "NVDA".into()])))
        );
        assert_eq!(parse("LAYOUT 4"), Some(Command::Function(Function::Layout("4".into()))));
        assert_eq!(
            parse("LAYOUT save Morning Desk"),
            Some(Command::Function(Function::Layout("save Morning Desk".into())))
        );
        assert_eq!(parse("NEW BOOK"), Some(Command::Function(Function::NewPanel("BOOK".into()))));
        assert_eq!(
            parse("SPLIT DOWN NEWS"),
            Some(Command::Function(Function::Split { down: true, panel: "NEWS".into() }))
        );
        assert_eq!(parse("SPLIT"), Some(Command::Function(Function::Split { down: false, panel: String::new() })));
        assert_eq!(parse("LINK BLUE"), Some(Command::Function(Function::Link(Some(crate::layouts::Link::Blue)))));
        assert_eq!(parse("LINK OFF"), Some(Command::Function(Function::Link(None))));
        assert_eq!(parse("LINK PINK"), None);
        assert_eq!(parse("AAPL CN"), Some(Command::SymbolFunction("AAPL US Equity".into(), Function::News)));
        assert_eq!(
            parse("BTCUSD CRNCY OB 50"),
            Some(Command::SymbolFunction("BTCUSD Curncy".into(), Function::OrderBook(Some(50))))
        );
        assert_eq!(parse("PREFS"), Some(Command::Function(Function::Settings)));
        assert_eq!(parse("update"), Some(Command::Function(Function::Update)));
        assert_eq!(parse("AAPL OMON"), Some(Command::SymbolFunction("AAPL US Equity".into(), Function::Options)));
        assert_eq!(parse("SPX INDEX OMON"), Some(Command::SymbolFunction("SPX Index".into(), Function::Options)));
        assert_eq!(parse("OMON"), Some(Command::Function(Function::Options)));
        assert_eq!(parse("SIDEBAR RIGHT"), Some(Command::Function(Function::Sidebar("RIGHT".into()))));
    }

    #[test]
    fn portfolio_commands() {
        let f = |v, a: &str| Some(Command::Function(Function::Portfolio(v, a.into())));
        assert_eq!(parse("PORT"), f(PortView::Summary, ""));
        assert_eq!(parse("port refresh"), f(PortView::Summary, "REFRESH"));
        assert_eq!(parse("ACCT"), f(PortView::Accounts, ""));
        assert_eq!(parse("POS"), f(PortView::Positions, ""));
        assert_eq!(parse("ORD"), f(PortView::Orders, ""));
        assert_eq!(parse("ACTV"), f(PortView::Activity, ""));
        assert_eq!(parse("PORT CONNECT"), f(PortView::Summary, "CONNECT"));
        assert_eq!(parse("BROKER"), f(PortView::Summary, "SETUP"));
        // After a security: positions (or orders) in that security.
        assert_eq!(
            parse("AAPL POS"),
            Some(Command::SymbolFunction(
                "AAPL US Equity".into(),
                Function::Portfolio(PortView::Positions, String::new())
            ))
        );
        // `ACT` stays a ticker (Enact Holdings), not the activity view.
        assert_eq!(parse("ACT"), Some(Command::Symbol("ACT US Equity".into())));
        for v in PortView::ALL {
            assert_eq!(parse(v.code()), f(v, ""), "{} round-trips", v.code());
        }
    }

    #[test]
    fn trade_commands() {
        let t = |s: &str| match parse(s) {
            Some(Command::Function(Function::Trade(t))) => Some(t),
            Some(Command::SymbolFunction(k, Function::Trade(mut t))) => {
                t.symbol = Some(k);
                Some(t)
            }
            _ => None,
        };
        let aapl = Some("AAPL US Equity".to_string());
        assert_eq!(
            t("BUY AAPL 10"),
            Some(TradeArgs { buy: Some(true), symbol: aapl.clone(), units: Some(10.0), ..Default::default() })
        );
        assert_eq!(t("buy 10 aapl"), t("BUY AAPL 10"), "quantity and symbol in either order");
        assert_eq!(t("AAPL BUY 10"), t("BUY AAPL 10"), "after a security");
        assert_eq!(t("AAPL US EQUITY BUY 10"), t("BUY AAPL 10"));
        let s = t("SELL MSFT 5 LMT 410.50").unwrap();
        assert_eq!((s.buy, s.units, s.limit), (Some(false), Some(5.0), Some(410.5)));
        assert_eq!(t("SELL MSFT 5 @410.5").unwrap().limit, Some(410.5));
        let st = t("SELL TSLA 3 STP 250 LMT 245").unwrap();
        assert_eq!((st.stop, st.limit), (Some(250.0), Some(245.0)));
        let n = t("BUY NVDA $1,500").unwrap();
        assert_eq!((n.notional, n.units), (Some(1500.0), None));
        assert_eq!(t("BUY 0.5 BRK/B").unwrap().symbol.as_deref(), Some("BRK.B US Equity"));
        let open = t("TRADE").unwrap();
        assert_eq!((open.buy, open.symbol), (None, None));
        assert_eq!(t("TRADE AAPL").unwrap().symbol, aapl);
        assert_eq!(t("BUY").unwrap().buy, Some(true));
        // Ambiguous or malformed orders are rejected rather than guessed.
        assert_eq!(t("BUY AAPL 10 20"), None);
        assert_eq!(t("BUY AAPL 10 $500"), None);
        assert_eq!(t("BUY AAPL LMT"), None);
        assert_eq!(t("BUY AAPL LMT -5"), None);
        assert!(hint("BUY AAPL 10 LMT 150").contains("Buy 10 AAPL limit 150"));
        let all = t("SELL ALL TSLA").unwrap();
        assert_eq!((all.buy, all.all, all.symbol.as_deref()), (Some(false), true, Some("TSLA US Equity")));
        assert_eq!(t("sell tsla all"), t("SELL ALL TSLA"));
        assert_eq!(t("TSLA SELL ALL").map(|a| a.all), Some(true));
        assert!(!t("SELL ALL").unwrap().all, "ALL alone is Allstate");
        assert_eq!(t("BUY ALL TSLA"), None);
        assert_eq!(t("SELL ALL TSLA 5"), None);
        assert_eq!(parse("DEMO"), Some(Command::Function(Function::AccountMode(false))));
        assert_eq!(parse("MODE LIVE"), Some(Command::Function(Function::AccountMode(true))));
    }

    #[test]
    fn symbol_with_function() {
        assert_eq!(parse("NVDA CONG <GO>"), Some(Command::SymbolFunction("NVDA US Equity".into(), Function::Congress)));
        assert_eq!(
            parse("BTCUSD CRNCY 5"),
            Some(Command::SymbolFunction("BTCUSD Curncy".into(), Function::Timeframe(Timeframe::M5)))
        );
        assert_eq!(parse("WEI"), Some(Command::Function(Function::Wei)));
        assert_eq!(parse("GC"), Some(Command::Function(Function::Curve)));
        assert_eq!(parse("SECF gold"), Some(Command::Function(Function::Search("gold".into()))));
        assert_eq!(
            parse("AAPL US EQUITY 1W"),
            Some(Command::SymbolFunction("AAPL US Equity".into(), Function::Timeframe(Timeframe::W1)))
        );
        assert_eq!(parse("AAPL WHATEVER"), None);
    }

    #[test]
    fn hints() {
        assert_eq!(hint("5"), "Interval 5 minutes");
        assert_eq!(hint("msft"), "Load MSFT US Equity — Microsoft Corp");
        assert_eq!(hint("US10Y GOVT"), "Load US10Y Govt — US Treasury 10 Year");
        assert!(hint("").contains("ticker"));
    }

    /// sabr grammar table (03-spec §4.9): `OHTANI.PIT PROJ ROS`, `660271 CAST XWOBA 50`,
    /// `NYM ROST`, `DIAZ CAST` (picker), `BTST MARCEL`, `TRACK OHTANI`, `UNTRACK OHTANI`.
    #[test]
    fn sabr_grammar() {
        assert_eq!(
            parse("OHTANI.PIT PROJ ROS"),
            Some(Command::Function(Function::Proj {
                entity: Some(SabrEntity { key: "p:660271:pit".into(), label: "Shohei Ohtani".into() }),
                horizon: Horizon::RoS,
                model: None,
            }))
        );
        assert_eq!(
            parse("660271 CAST XWOBA 50"),
            Some(Command::Function(Function::Cast {
                entity: Some(SabrEntity { key: "p:660271:bat".into(), label: "Shohei Ohtani".into() }),
                metric: Some("bat:xwoba".into()),
                window: Some(sabr_model::window::Window::Events { unit: sabr_model::window::Unit::Pa, n: 50 }),
            }))
        );
        assert_eq!(parse("NYM ROST"), Some(Command::Function(Function::Rost("NYM".into()))));
        // `DIAZ` is ambiguous: the ranked picker opens instead of a command.
        let Some(CommandOutcome::PickList(hits)) = parse_outcome("DIAZ CAST") else { panic!("DIAZ CAST") };
        assert!(hits.len() > 2);
        assert!(hits.iter().any(|h| h.name.contains("Díaz")));
        // A dominant name resolves directly; `BTST` needs its model.
        assert_eq!(parse("BTST MARCEL"), Some(Command::Function(Function::Btst("MARCEL".into()))));
        assert_eq!(
            parse("TRACK OHTANI"),
            Some(Command::Function(Function::TrackPlayer(SabrEntity {
                key: "p:660271:bat".into(),
                label: "Shohei Ohtani".into(),
            })))
        );
        assert!(parse("UNTRACK OHTANI").is_some());
        // Roles and horizons round-trip either way.
        assert!(
            matches!(parse("660271 CAST"), Some(Command::Function(Function::Cast { metric: None, .. }))),
            "bare CAST resolves to the default metric"
        );
        assert!(parse("TRACK PELOSI").is_some(), "unresolvable names stay on the politician path");
        // `ALRT` opens the alerts pane without an entity.
        assert_eq!(parse("ALRT"), Some(Command::Function(Function::Alerts)));
    }

    #[test]
    fn indicator_identity() {
        let spec = |code: &str, params: &[f64]| IndicatorSpec::new(code, params).unwrap();
        assert!(spec("RSI", &[14.0]).same_kind(&spec("RSI", &[21.0])));
        assert!(!spec("SMA", &[20.0]).same_kind(&spec("SMA", &[50.0])));
        assert!(spec("MACD", &[]).is_pane());
    }

    /// Every indicator code and alias reaches the indicator: no function, timeframe or range
    /// spelling shadows it.
    #[test]
    fn every_indicator_code_parses_to_its_indicator() {
        for def in tr_core::indicators::registry::all() {
            for code in std::iter::once(&def.code).chain(def.aliases) {
                match parse(code) {
                    Some(Command::Function(Function::Indicator(s))) => assert_eq!(s.code, def.code, "{code}"),
                    other => panic!("{code} parses as {other:?}"),
                }
                assert_eq!(parse(&format!("IND {code}")), parse(code), "IND {code}");
            }
        }
    }
}
