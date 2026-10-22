//! The Source layer (spec §4.6 "Sources", "Facade"): three typed, non-dyn source structs —
//! [`SavantSource`], [`StatsApiSource`], [`CotsSource`] — behind the typed [`Data`] facade. Nothing
//! here is an object-safe trait: the facade is a compile-time mapping from typed method to URL shape
//! and policy (C-obj2), and the source structs are plain values, never `dyn`.
//!
//! Fetch rule, shared verbatim by all three:
//! 1. [`RawStore::stale`] fresh → return the stored bytes as [`Freshness::Cached`], provenance read
//!    back from the store.
//! 2. Fan-out budget ([`Policy::max_per_command`]) is per user command, claimed by the [`Data`]
//!    facade and passed down per fetch: when exhausted, refuse — stored bytes as
//!    [`Freshness::StaleRefused`] when present, else [`FetchError::FanOutBudget`]. No silent retry
//!    loops, ever.
//! 3. Pace with [`Policy::min_interval_secs`]: sleep to the pacing shoulder. Every fetch runs inside
//!    a blocking worker (`spawn_blocking` callers), so a plain `thread::sleep` is safe and no lock
//!    is ever held across an `.await`.
//! 4. GET once: any 2xx is the payload; anything else is [`FetchError::Http { status }`]. Store the
//!    raw bytes via [`RawStore::put`] and return [`Freshness::Fetched`]. This layer never
//!    normalises — callers parse.
//!
//! Offline (`SABR_OFFLINE=1` or [`Data::open`] with `offline: true`): the source slots are `None`
//! and every network method returns [`FetchError::Offline`] before a URL is ever built.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use crate::key::{FetchKey, Role};
use crate::policy::Policy;
use crate::provenance::Provenance;
use crate::store::{Freshness, Loaded, RawStore, StoreError};

const SAVANT_CSV: &str = "https://baseballsavant.mlb.com/statcast_search/csv";
const STATSAPI: &str = "https://statsapi.mlb.com/api/v1";
const UA: &str = "sabr-rs/0.1 (+ https://github.com/sabr-rs; individual research, non-bulk)";

/// Blocking single-shot GET: `(status, body)` for any HTTP status, transport-level failure as
/// [`FetchError::Transport`]. One type-erased transport *field* keeps the three source structs
/// concrete: the C-obj2 ban is on dyn at the source/facade boundary, not on the single injected
/// transport seam that lets tests run with no network.
pub type HttpGet = Arc<dyn Fn(&str) -> FetchResult<(u16, Vec<u8>)> + Send + Sync>;

/// Fetch-layer errors. Policy refusals that still have bytes are `Loaded` values instead; these are
/// the hard failures.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("offline: the {0} source slot is empty (SABR_OFFLINE=1)")]
    Offline(&'static str),
    #[error("fan-out budget exhausted for {family} (max {max} per command) and no stored bytes")]
    FanOutBudget { family: &'static str, max: u32 },
    #[error("source {want} will not serve key {key}")]
    WrongSource { want: &'static str, key: String },
    #[error("http {status} from {url}")]
    Http { status: u16, url: String },
    #[error("transport: {0}")]
    Transport(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub type FetchResult<T> = std::result::Result<T, FetchError>;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The real transport: `ureq` 2 (blocking, rustls), `sabr-rs` UA, 60 s timeout, one attempt.
pub struct RealTransport;
impl RealTransport {
    pub fn build() -> HttpGet {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).user_agent(UA).build();
        Arc::new(move |url: &str| match agent.get(url).set("Accept", "*/*").call() {
            // ureq 2: 2xx → Ok(response); 4xx/5xx → Err(Error::Status(_, response)). Both paths
            // surface a (status, body) pair; the fetch rule turns non-2xx into FetchError::Http.
            Ok(resp) => {
                let mut bytes = Vec::new();
                resp.into_reader().read_to_end(&mut bytes).map_err(|e| FetchError::Transport(e.to_string()))?;
                Ok((200, bytes))
            }
            Err(ureq::Error::Status(_, resp)) => {
                let status = resp.status();
                let mut bytes = Vec::new();
                let _ = resp.into_reader().read_to_end(&mut bytes);
                Ok((status, bytes))
            }
            Err(e) => Err(FetchError::Transport(e.to_string())),
        })
    }
}

fn empty_prov() -> Provenance {
    Provenance::new("missing", "", 0, [0; 32], "")
}

/// The URL per key; each variant compiles to exactly one shape for its source family.
fn url_for(key: &FetchKey) -> String {
    match key {
        FetchKey::StatcastPlayerSeason { mlbam, role, season } => {
            let (lookup, player_type) = match role {
                Role::Batter => ("batters_lookup", "batter"),
                Role::Pitcher => ("pitchers_lookup", "pitcher"),
            };
            format!(
                "{SAVANT_CSV}?all=true&type=details&hfSea={season}%7C&hfGT=R%7C&{lookup}%5B%5D={mlbam}&player_type={player_type}"
            )
        }
        FetchKey::SeasonLines { season } => format!("{STATSAPI}/schedule?sportId=1&season={season}"),
        FetchKey::TransactionsDay { day } => {
            format!("{STATSAPI}/transactions?date={}", day.format("%Y-%m-%d"))
        }
        FetchKey::Roster { team, season } => format!("{STATSAPI}/teams/{team}/roster?season={season}"),
        FetchKey::MilbSeasonLines { sport_id, season } => format!(
            "{STATSAPI}/stats?stats=season&group=hitting,pitching&statsByScenario=true&sportId={sport_id}&season={season}"
        ),
        FetchKey::Draft { year } => format!("{STATSAPI}/draft/{year}"),
        FetchKey::Payroll { season, sheet_id } => {
            format!("https://docs.google.com/spreadsheets/d/{sheet_id}/gviz/tq?tqx=out:csv&sheet={season}.xls")
        }
    }
}

/// State a source owns while fetching: UA string, transport seam, pacing timestamp.
pub struct HttpState {
    pub ua: String,
    http: HttpGet,
    last: Mutex<Option<Instant>>,
    /// Test seam: replaces the policy's min_interval_secs (zero = no sleep) so budget tests
    /// exercise refusal logic without paying the real 2 s pacing.
    pace_override: Option<Duration>,
}

impl HttpState {
    pub fn new(ua: &str, http: HttpGet) -> Self {
        Self { ua: ua.to_string(), http, last: Mutex::new(None), pace_override: None }
    }

    #[cfg(test)]
    pub fn with_pace(ua: &str, http: HttpGet, pace: Option<Duration>) -> Self {
        Self { ua: ua.to_string(), http, last: Mutex::new(None), pace_override: pace }
    }

    /// Sleep so successive fetches are at least `min` apart. Called inside blocking workers only
    /// (spawn_blocking callers); no lock is held across the sleep.
    fn pace(&self, min: Duration) -> FetchResult<()> {
        let min = self.pace_override.unwrap_or(min);
        let wait = match self.last.lock() {
            Ok(mut g) => {
                let wait = g.map(|t| min.saturating_sub(t.elapsed())).unwrap_or(Duration::ZERO);
                *g = Some(Instant::now());
                wait
            }
            // A poisoned pacing lock means a panicked fetch; keep going without pacing.
            Err(_) => Duration::ZERO,
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        Ok(())
    }
}

/// The shared fetch body. `budget` is the count the [`Data`] facade already spent on this source
/// family within the current command; the source enforces the refusal itself.
fn fetch_shared(
    state: &HttpState,
    source: &'static str,
    key: &FetchKey,
    store: &RawStore,
    policy: &Policy,
    budget: u32,
) -> FetchResult<Loaded<Vec<u8>>> {
    if key.source_family() != source {
        return Err(FetchError::WrongSource { want: source, key: key.as_str() });
    }
    let now = now_secs();
    if !store.stale(key, policy, now)? {
        return match store.get(key)? {
            Some(sp) => {
                let age_secs = now.saturating_sub(sp.prov.fetched);
                Ok(Loaded::new(sp.bytes, sp.prov, Freshness::Cached { age_secs }))
            }
            // Fresh row but the payload read raced a purge: nothing to serve, nothing to spend.
            None => Ok(Loaded::new(Vec::new(), empty_prov(), Freshness::StaleRefused)),
        };
    }
    if budget >= policy.max_per_command {
        return match store.get(key)? {
            Some(sp) => Ok(Loaded::new(sp.bytes, sp.prov, Freshness::StaleRefused)),
            None => Err(FetchError::FanOutBudget { family: source, max: policy.max_per_command }),
        };
    }
    state.pace(Duration::from_secs(policy.min_interval_secs))?;
    let url = url_for(key);
    let (status, bytes) = (state.http)(&url)?;
    if !(200..300).contains(&status) {
        return Err(FetchError::Http { status, url });
    }
    let prov = Provenance::new(
        source,
        url.as_str(),
        now_secs(),
        {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher.finalize().into()
        },
        policy.attribution,
    );
    store.put(key, &prov, &bytes)?;
    Ok(Loaded::new(bytes, prov, Freshness::Fetched))
}

/// One bundle of the parts every source shares; instantiates a typed source struct per family.
macro_rules! define_source {
    ($name:ident, $family:literal) => {
        pub struct $name {
            state: HttpState,
        }

        impl $name {
            /// The live, network-backed source on the `ureq`/rustls transport.
            pub fn live() -> Self {
                $name::with_transport(RealTransport::build())
            }

            /// Test seam: inject a transport closure (recording, canned) — no network in tests.
            pub fn with_transport(http: HttpGet) -> Self {
                $name { state: HttpState::new(UA, http) }
            }

            #[cfg(test)]
            pub fn with_pace(http: HttpGet, pace: Option<Duration>) -> Self {
                $name { state: HttpState::with_pace(UA, http, pace) }
            }

            pub fn url(&self, key: &FetchKey) -> String {
                url_for(key)
            }

            /// The shared fetch rule at this source's pace and policy.
            pub fn fetch(
                &self,
                key: &FetchKey,
                store: &RawStore,
                policy: &Policy,
                budget: u32,
            ) -> FetchResult<Loaded<Vec<u8>>> {
                fetch_shared(&self.state, $family, key, store, policy, budget)
            }
        }
    };
}

define_source!(SavantSource, "savant");
define_source!(StatsApiSource, "statsapi");
define_source!(CotsSource, "cots");

/// Per-key refresh outcome.
#[derive(Debug)]
pub struct RefreshEntry {
    pub key: FetchKey,
    pub outcome: Result<Freshness, String>,
}

/// What [`Data::refresh_stale`] did: one entry per requested key.
#[derive(Debug, Default)]
pub struct RefreshReport {
    pub entries: Vec<RefreshEntry>,
}

/// The typed data facade: the raw store, one optional slot per source (present iff network is
/// enabled) and the per-command fan-out budgets. `Sync` — the budget map is a mutex, so one `Data`
/// can be shared across blocking worker threads the way the CLI's `spawn_blocking` calls do.
pub struct Data {
    pub raw: RawStore,
    pub savant: Option<SavantSource>,
    pub statsapi: Option<StatsApiSource>,
    pub cots: Option<CotsSource>,
    budgets: Mutex<BTreeMap<&'static str, u32>>,
}

impl Data {
    /// `SABR_OFFLINE=1` forces every source slot to `None`; otherwise all three live sources are
    /// present, each with its family policy ([`Policy::savant`], [`Policy::statsapi`],
    /// [`Policy::cots`]).
    pub fn from_env(dirs: &crate::xdg::XdgDirs) -> Result<Data, StoreError> {
        Self::open(dirs, std::env::var("SABR_OFFLINE").as_deref() == Ok("1"))
    }

    /// Explicit constructor: also the seam tests need to build either mode unambiguously.
    pub fn open(dirs: &crate::xdg::XdgDirs, offline: bool) -> Result<Data, StoreError> {
        Ok(Data {
            raw: RawStore::open(&dirs.cache)?,
            savant: (!offline).then(SavantSource::live),
            statsapi: (!offline).then(StatsApiSource::live),
            cots: (!offline).then(CotsSource::live),
            budgets: Mutex::new(BTreeMap::new()),
        })
    }

    /// Test/CI constructor: store present, sources injected explicitly (closures run in-process).
    pub fn with_transports(
        dirs: &crate::xdg::XdgDirs,
        savant: HttpGet,
        statsapi: HttpGet,
        cots: HttpGet,
    ) -> Result<Data, StoreError> {
        Ok(Data {
            raw: RawStore::open(&dirs.cache)?,
            savant: Some(SavantSource::with_transport(savant)),
            statsapi: Some(StatsApiSource::with_transport(statsapi)),
            cots: Some(CotsSource::with_transport(cots)),
            budgets: Mutex::new(BTreeMap::new()),
        })
    }

    #[cfg(test)]
    pub fn with_paced_transports(
        dirs: &crate::xdg::XdgDirs,
        savant: HttpGet,
        statsapi: HttpGet,
        cots: HttpGet,
        pace: Option<Duration>,
    ) -> Result<Data, StoreError> {
        Ok(Data {
            raw: RawStore::open(&dirs.cache)?,
            savant: Some(SavantSource::with_pace(savant, pace)),
            statsapi: Some(StatsApiSource::with_pace(statsapi, pace)),
            cots: Some(CotsSource::with_pace(cots, pace)),
            budgets: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn offline(dirs: &crate::xdg::XdgDirs) -> Result<Data, StoreError> {
        Self::open(dirs, true)
    }

    pub fn is_offline(&self) -> bool {
        self.savant.is_none() && self.statsapi.is_none() && self.cots.is_none()
    }

    /// Reset the per-command fan-out budgets; call at the top of every user command.
    pub fn begin_command(&self) {
        if let Ok(mut map) = self.budgets.lock() {
            map.clear();
        } // a poisoned map means a panicked fetch; begin_command must not panic on it.
    }

    /// Claim (read) the family's spent count, then remember to [`Data::spend`] after a fetch.
    fn claim(&self, family: &'static str) -> u32 {
        let mut map = self.budgets.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry(family).or_insert(0)
    }

    fn spend(&self, family: &'static str) {
        let mut map = self.budgets.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry(family).or_insert(0) += 1;
    }

    fn fetch_key(&self, key: &FetchKey) -> FetchResult<Loaded<Vec<u8>>> {
        let family = key.source_family();
        let (slot, policy): (Option<&HttpState>, Policy) = match family {
            "savant" => (self.savant.as_ref().map(|s| &s.state), Policy::savant()),
            "statsapi" => (self.statsapi.as_ref().map(|s| &s.state), Policy::statsapi()),
            "cots" => (self.cots.as_ref().map(|s| &s.state), Policy::cots()),
            other => unreachable!("no fetch path for source family {other:?}"),
        };
        let Some(state) = slot else { return Err(FetchError::Offline(family)) };
        let budget = self.claim(family);
        let loaded = fetch_shared(state, family, key, &self.raw, &policy, budget)?;
        if loaded.freshness == Freshness::Fetched {
            self.spend(family);
        }
        Ok(loaded)
    }

    // ---- Typed slots (spec §4.6 "Facade"): each method is a compile-time choice of source ---

    /// One player's Statcast season CSV, per role (Savant).
    pub fn player_statcast(&self, mlbam: u32, role: Role, season: u16) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::StatcastPlayerSeason { mlbam, role, season })
    }

    /// One season's game lines feed (Stats API schedule for the MLB sport level).
    pub fn season_lines(&self, season: u16) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::SeasonLines { season })
    }

    /// One day's transaction feed (Stats API).
    pub fn transactions_day(&self, day: NaiveDate) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::TransactionsDay { day })
    }

    /// One team's MLB roster for a season (Stats API).
    pub fn roster(&self, team: &str, season: u16) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::Roster { team: team.to_owned(), season })
    }

    /// One minor-league sport level's season lines (Stats API).
    pub fn milb_lines(&self, sport_id: u32, season: u16) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::MilbSeasonLines { sport_id, season })
    }

    /// One draft year's picks feed (Stats API).
    pub fn draft(&self, year: u16) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::Draft { year })
    }

    /// One season's Opening-Day salary sheet (Cot's).
    pub fn payroll(&self, season: u16, sheet_id: &str) -> FetchResult<Loaded<Vec<u8>>> {
        self.fetch_key(&FetchKey::Payroll { season, sheet_id: sheet_id.to_owned() })
    }

    /// Refresh with a scope (spec §4.6 "Refresh"): for each requested key, fetch what is stale
    /// within policy. Budgets reset first — a refresh *is* one command. Keys whose slot is offline
    /// (or any per-key error) are reported, never retried. The CLI wires tracked/recently-viewed
    /// keys into this in a later wave.
    pub fn refresh_stale(&self, keys: &[FetchKey]) -> FetchResult<RefreshReport> {
        self.begin_command();
        let mut report = RefreshReport::default();
        for key in keys {
            let family = key.source_family();
            let entry = match self.fetch_key(key) {
                Ok(loaded) => Ok(loaded.freshness),
                Err(FetchError::Offline(_)) => Err(format!("offline: {family} disabled")),
                Err(e) => Err(e.to_string()),
            };
            report.entries.push(RefreshEntry { key: key.clone(), outcome: entry });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xdg::XdgDirs;

    fn dirs(tag: &str) -> XdgDirs {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join(tag);
        std::mem::forget(tmp); // the dirs outlive the handle; temp litter is acceptable in CI
        XdgDirs { config: base.clone(), cache: base.clone(), state: base.clone(), data: base.join("data") }
    }

    /// A transport that records URLs and returns canned bodies.
    #[derive(Clone)]
    struct Recorder {
        hits: Arc<Mutex<Vec<String>>>,
        body: Vec<u8>,
        status: u16,
    }

    impl Recorder {
        fn new(body: &[u8]) -> Self {
            Recorder { hits: Arc::new(Mutex::new(Vec::new())), body: body.to_vec(), status: 200 }
        }
        fn transport(self) -> HttpGet {
            let hits = self.hits.clone();
            let body = self.body;
            let status = self.status;
            Arc::new(move |url: &str| {
                hits.lock().unwrap().push(url.to_string());
                Ok((status, body.clone()))
            })
        }
        fn count(&self) -> usize {
            self.hits.lock().unwrap().len()
        }
    }

    #[test]
    fn offline_data_returns_offline_for_every_slot_and_never_fetches() {
        let data = Data::offline(&dirs("offline")).unwrap();
        assert!(data.is_offline());
        let errs = [
            data.player_statcast(660271, Role::Batter, 2025).unwrap_err(),
            data.season_lines(2025).unwrap_err(),
            data.transactions_day(NaiveDate::from_ymd_opt(2026, 3, 20).unwrap()).unwrap_err(),
            data.roster("NYY", 2025).unwrap_err(),
            data.milb_lines(11, 2025).unwrap_err(),
            data.draft(2025).unwrap_err(),
            data.payroll(2026, "12XSXOQpjDJDCJKsA4xC1e_9FlS11aeioZy_p1nqpclg").unwrap_err(),
        ];
        for e in &errs {
            assert!(matches!(e, FetchError::Offline(_)), "expected Offline, got {e:?}");
        }
        // refresh_stale is a report, not an error: offline keys are reported per entry.
        let report = data.refresh_stale(&[FetchKey::Draft { year: 2025 }]).unwrap();
        assert!(matches!(&report.entries[0].outcome, Err(msg) if msg.starts_with("offline")));
    }

    #[test]
    fn first_fetch_stores_and_second_serves_cached_without_transport() {
        let rec = Recorder::new(b"pitch_type,x\nfastball,99");
        let transport = rec.clone().transport();
        let data = Data::with_transports(&dirs("cached"), transport.clone(), transport.clone(), transport).unwrap();
        let k = FetchKey::StatcastPlayerSeason { mlbam: 660271, role: Role::Batter, season: 2025 };
        let first = data.fetch_key(&k).unwrap();
        assert_eq!(first.freshness, Freshness::Fetched);
        assert_eq!(first.value, b"pitch_type,x\nfastball,99");
        assert_eq!(first.prov.source, "savant");
        assert!(!first.prov.attribution.is_empty());
        let second = data.fetch_key(&k).unwrap();
        assert!(matches!(second.freshness, Freshness::Cached { .. }), "second must be cached");
        assert_eq!(second.value, first.value);
        assert_eq!(rec.count(), 1, "the cached hit must not touch the transport");
    }

    #[test]
    fn fan_out_budget_refuses_without_transport_and_keeps_stored_bytes() {
        // Drive the savant family's 300-key budget with zero pacing so the test measures the
        // refusal logic, not the real 2 s pace (policy values live in policy::tests).
        let rec = Recorder::new(b"player,salary\n");
        let transport = rec.clone().transport();
        let data = Data::with_paced_transports(
            &dirs("budget"),
            transport.clone(),
            transport.clone(),
            transport.clone(),
            Some(Duration::ZERO),
        )
        .unwrap();
        let max = Policy::savant().max_per_command;
        for season in 1990..1990 + max {
            let k = FetchKey::StatcastPlayerSeason { mlbam: season, role: Role::Batter, season: season as u16 };
            assert!(data.fetch_key(&k).unwrap().freshness == Freshness::Fetched, "season {season}");
        }
        assert_eq!(rec.count(), max as usize);
        // Budget exhausted: a NEW key with no stored bytes is a hard refusal.
        let over = FetchKey::StatcastPlayerSeason { mlbam: 999999, role: Role::Batter, season: 2300 };
        let err = data.fetch_key(&over).unwrap_err();
        assert!(matches!(err, FetchError::FanOutBudget { family: "savant", .. }), "got {err:?}");
        // A key with stored bytes but an expired TTL serves them as StaleRefused instead of
        // erroring: age the stored payload's fetch time past the TTL directly in the index.
        let known = FetchKey::StatcastPlayerSeason { mlbam: 1990, role: Role::Batter, season: 1990 };
        let aged = now_secs() - Policy::savant().ttl_open_secs - 10;
        data.raw.set_fetched_for_test(&known, aged).unwrap();
        let served = data.fetch_key(&known).unwrap();
        assert_eq!(served.freshness, Freshness::StaleRefused);
        assert_eq!(served.value, b"player,salary\n");
        assert_eq!(rec.count(), max as usize, "refusal must not fetch");
        // begin_command resets the budget; the over-budget key now fetches.
        data.begin_command();
        assert!(data.fetch_key(&over).unwrap().freshness == Freshness::Fetched);
    }

    #[test]
    fn wrong_family_key_is_a_compile_time_shape_error() {
        let rec = Recorder::new(b"{}");
        let transport = rec.clone().transport();
        let data =
            Data::with_transports(&dirs("wrong"), transport.clone(), transport.clone(), transport.clone()).unwrap();
        // Force the savant source to serve a statsapi key through the per-source fetch (the facade
        // routes by family, so this only misfires via the source method directly).
        let savant = SavantSource::with_transport(transport);
        let err = savant.fetch(&FetchKey::Draft { year: 2025 }, &data.raw, &Policy::savant(), 0).unwrap_err();
        assert!(matches!(err, FetchError::WrongSource { want: "savant", .. }), "got {err:?}");
        assert_eq!(rec.count(), 0);
    }

    #[test]
    fn http_status_maps_to_error_and_stores_nothing() {
        let rec = Recorder { hits: Arc::new(Mutex::new(Vec::new())), body: b"nope".to_vec(), status: 500 };
        let transport = rec.clone().transport();
        let data = Data::with_transports(&dirs("http500"), transport.clone(), transport.clone(), transport).unwrap();
        let k = FetchKey::Draft { year: 2024 };
        let err = data.fetch_key(&k).unwrap_err();
        assert!(matches!(err, FetchError::Http { status: 500, .. }), "got {err:?}");
        assert!(data.raw.get(&k).unwrap().is_none(), "a failed fetch must not store");
    }

    #[test]
    fn urls_match_the_probe_verified_shapes() {
        let savant = SavantSource::with_transport(Arc::new(|_| unreachable!("no fetch in this test")));
        assert_eq!(
            savant.url(&FetchKey::StatcastPlayerSeason { mlbam: 660271, role: Role::Pitcher, season: 2025 }),
            "https://baseballsavant.mlb.com/statcast_search/csv?all=true&type=details&hfSea=2025%7C&hfGT=R%7C&pitchers_lookup%5B%5D=660271&player_type=pitcher"
        );
        let api = StatsApiSource::with_transport(Arc::new(|_| unreachable!("no fetch in this test")));
        assert_eq!(
            api.url(&FetchKey::TransactionsDay { day: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap() }),
            "https://statsapi.mlb.com/api/v1/transactions?date=2026-07-01"
        );
        let cots = CotsSource::with_transport(Arc::new(|_| unreachable!("no fetch in this test")));
        assert!(
            cots.url(&FetchKey::Payroll { season: 2026, sheet_id: "12XSXO".to_string() })
                .starts_with("https://docs.google.com/spreadsheets/d/12XSXO/gviz/tq?tqx=out:csv&sheet=2026.xls")
        );
    }

    // ---- live probes: `cargo test -p sabr-data -- --ignored network` (one per source) ----

    #[test]
    #[ignore = "network"]
    fn live_savant_player_season_fetches_and_normalizes() {
        let data = Data::open(&dirs("live-savant"), false).unwrap();
        let loaded = data.player_statcast(660271, Role::Batter, 2025).unwrap();
        assert_eq!(loaded.freshness, Freshness::Fetched);
        let text = String::from_utf8(loaded.value.clone()).unwrap();
        let rev = sabr_model::event::SourceRev {
            source: "savant",
            url: loaded.prov.url.clone(),
            fetched: loaded.prov.fetched,
            sha256: loaded.prov.sha256,
        };
        let log = crate::savant::parse_log(&text, crate::savant::SavantMode::Bat, 660271, rev).unwrap();
        assert_eq!(log.events.len(), 2940, "Ohtani 2025 bat rows");
    }

    #[test]
    #[ignore = "network"]
    fn live_statsapi_transactions_day() {
        let data = Data::open(&dirs("live-api"), false).unwrap();
        let loaded = data.transactions_day(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()).unwrap();
        assert!(loaded.value.starts_with(b"{"), "expected JSON");
    }

    #[test]
    #[ignore = "network"]
    fn live_cots_sheet_header() {
        let data = Data::open(&dirs("live-cots"), false).unwrap();
        let loaded = data.payroll(2026, "12XSXOQpjDJDCJKsA4xC1e_9FlS11aeioZy_p1nqpclg").unwrap();
        assert!(loaded.value.starts_with(b"\""), "expected quoted CSV header");
    }
}
