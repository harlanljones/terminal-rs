# 02 — Design arena (Phase 4)

Three decisions the grill marked most expensive to reverse, each designed twice as structurally distinct
candidates: modules and interfaces only (types and signatures; every body is `todo!()`). Scored on the agreed
rubric (2026-10-21): **effort** (5 = cheapest), **performance**, **reproducibility**, **portfolio signal**,
**maintenance**, **offline/ToS fit** — each 1–5, equal weight, max 30. After the scores, the winner is argued
against: three likeliest failure modes, each with the cheapest throwaway prototype that tests it. An adversarial
review by a second model family follows (section 5); the user's picks are recorded in section 6.

Inputs: decision records DR-3.1–3.36 (`01-grill.md`), the audit (`00-audit.md`), upstream types as of `b81f40b`.
Working names only; crate boundaries are decided by the pick, not assumed here.

---

## 1. Arena A — entity / series / window model and cache key

Governing records: DR-3.2 (entity = (player, role) on MLBAM id), DR-3.5 (SQLite re-keyed from `(symbol, tf, ts)`
to entity + window + sequence), DR-3.35 (windows carry a shrinkage label n/(n+x)), B3 (feature coverage intervals).
Upstream coupling being replaced: `tr_core::Bar {ts,o,h,l,c,v}`, `Timeframe`, `Security`; `BarCache` PK
`(symbol, tf, ts)`; chart x-axis is bar *index* (`LinePyramid::reduce` x = i + 0.5), `SeriesGpu::build(&[Bar])`,
`line_ref(&[f64])`.

Shared vocabulary (both candidates):

```rust
pub enum Role { Bat, Pit }
pub struct PlayerId { pub mlbam: u32, pub role: Role }             // DR-3.2
pub enum Entity { Player(PlayerId), Team { mlbam: u16 }, Game { pk: u32 }, Matchup { bat: PlayerId, pit: PlayerId } }
pub enum Unit { Pa, Bf, Pitch, Game }
pub enum Window { Events { unit: Unit, n: u32 }, Season(u16), Between { from: Date, to: Date } }
pub struct MetricId(&'static str);                                   // "xwoba", "bat_speed", …
/// Reliability label per DR-3.35: weight = n / (n + x); x from the Carleton table, None = "no threshold".
pub struct Reliability { pub n: u32, pub x: Option<u32> }
impl Reliability { pub fn weight(&self) -> Option<f64> { todo!() } }
/// Feature coverage per B3: a value is defined only inside one of its intervals; null outside, never zero.
pub struct Coverage { pub intervals: Vec<(Date, Option<Date>)> }
```

### A1 — Event log canonical, series derived on demand

The store holds one append-only event log per (entity, role, season); every metric series is a pure function of
events over a window, memoised by content hash. Nothing finance-shaped survives; the chart receives a derived
`Series`.

```rust
// crate `sabr-model` (working name), module `events`
pub struct Seq(pub u64);                      // dense per (entity, season): PA number or pitch number
pub struct Event {
    pub seq: Seq, pub game_pk: u32, pub date: Date, pub unit: Unit,
    pub fields: FieldMap,                     // name → f64 | str | null; names pinned by the Savant allowlist (B2)
}
pub struct EventLog { pub entity: Entity, pub season: u16, pub source_rev: SourceRev, pub events: Vec<Event> }
pub struct SourceRev { pub source: &'static str, pub fetched: DateTime, pub sha256: [u8; 32] }

// module `metric`
pub trait Metric: Send + Sync {
    fn id(&self) -> MetricId;
    fn unit(&self) -> Unit;                                           // which seq counts as n
    fn coverage(&self) -> Coverage;                                   // B3 intervals, static per metric
    fn threshold(&self) -> Option<u32>;                               // Carleton x, DR-3.35
    /// "undefined is not an error": None when the window has no defined inputs (saberkit rule).
    fn eval(&self, window: &[Event]) -> Option<f64>;
}
pub fn registry() -> &'static [&'static dyn Metric] { todo!() }

// module `series`
pub struct Point { pub seq: Seq, pub date: Date, pub value: Option<f64>, pub rel: Reliability }
pub struct Series { pub entity: Entity, pub metric: MetricId, pub window: Window, pub as_of: Date, pub points: Vec<Point> }
pub struct SeriesKey { pub entity: Entity, pub metric: MetricId, pub window: Window, pub as_of: Date, pub log_sha: [u8; 32] }
pub fn derive(log: &EventLog, metric: &dyn Metric, window: Window, as_of: Date) -> Series { todo!() }  // O(n) rolling
pub fn key(series: &Series, log: &EventLog) -> SeriesKey { todo!() }

// module `store` (SQLite, DR-3.5)
pub struct Store { conn: rusqlite::Connection }
impl Store {
    pub fn open(dir: &XdgDirs) -> Result<Self> { todo!() }
    // events(entity_key TEXT, role TEXT, season INT, seq INT, game_pk INT, date TEXT, fields BLOB,
    //        PRIMARY KEY(entity_key, role, season, seq)) WITHOUT ROWID; log_rev(entity_key, role, season, sha256, fetched)
    pub fn upsert_log(&self, log: &EventLog) -> Result<()> { todo!() }
    pub fn log(&self, entity: &Entity, season: u16) -> Result<Option<EventLog>> { todo!() }
    pub fn memo_get(&self, key: &SeriesKey) -> Result<Option<Series>> { todo!() }   // derived cache, evicted by log_sha
    pub fn memo_put(&self, key: &SeriesKey, s: &Series) -> Result<()> { todo!() }
}

// chart adapter (tr-chart kept; DR-3.5, audit H1)
pub fn to_line(series: &Series) -> Vec<f64> { todo!() }               // NaN where value is None → gap, not zero
pub fn axis_labels(series: &Series) -> Vec<(usize, String)> { todo!() } // seq index → "PA 120 · Jun 3" (E1 prototype P4)
```

### A2 — Materialised metric series, bars-shaped

The store holds precomputed series rows per (entity, metric, window spec), ingestion materialises them; the app and
chart consume rows exactly as upstream consumes `Bar`s. Events are an ingestion input, not a queryable thing.

```rust
// crate `sabr-model`, module `row`
pub struct MetricRow { pub idx: u32, pub date: Date, pub value: Option<f64>, pub n: u32 }   // the Bar replacement
pub struct WindowSpec { pub unit: Unit, pub n: u32 }                                      // label "PA50", "G10", "SEASON"
impl WindowSpec { pub fn label(&self) -> String { todo!() } pub fn parse(s: &str) -> Option<Self> { todo!() } }

pub struct Series { pub entity: Entity, pub metric: MetricId, pub spec: WindowSpec, pub as_of: Date, pub rows: Vec<MetricRow> }

// module `materialize` (runs at ingest, not at view time)
pub struct Plan { pub metrics: Vec<MetricId>, pub specs: Vec<WindowSpec> }      // the shipped default set (DR-3.35 windows)
pub fn materialize(raw: &RawSeason, plan: &Plan) -> Vec<Series> { todo!() }
pub fn default_plan() -> Plan { todo!() }

// module `store` (SQLite, DR-3.5)
pub struct Store { conn: rusqlite::Connection }
impl Store {
    pub fn open(dir: &XdgDirs) -> Result<Self> { todo!() }
    // series(entity_key TEXT, metric TEXT, spec TEXT, idx INT, date TEXT, value REAL NULL, n INT,
    //        PRIMARY KEY(entity_key, metric, spec, idx)) WITHOUT ROWID; series_rev(entity_key, metric, spec, as_of, sha256)
    pub fn upsert(&self, s: &Series) -> Result<()> { todo!() }
    pub fn load(&self, entity: &Entity, metric: MetricId, spec: &WindowSpec, limit: usize) -> Result<Series> { todo!() }
    pub fn as_of(&self, entity: &Entity, metric: MetricId, spec: &WindowSpec) -> Result<Option<Date>> { todo!() }
}

// chart adapter: rows → tr_chart::line_ref, identical to the upstream worker path (build_gpu)
pub fn to_line(series: &Series) -> Vec<f64> { todo!() }
pub fn reliability(series: &Series, x: Option<u32>) -> Vec<Option<f64>> { todo!() }   // n/(n+x) per row, computed at view
```

### Scores — Arena A
| Criterion | A1 event log | A2 materialised | Notes |
|---|---|---|---|
| Effort (5 = cheapest) | 3 | 4 | A2 is a rename of the `Bar` path; A1 adds the memo layer and `Metric` trait before the first chart |
| Performance | 4 | 5 | A1 derives per view (O(n) rolling over ≤3K events/player-season; team-season ≤ 6.5K PA); A2 paints from rows |
| Reproducibility | 5 | 3 | A1: any window, any as-of, keyed by log hash; A2: only shipped specs, as-of baked at ingest |
| Portfolio signal | 5 | 2 | A1 is the point-in-time feature store the postings price; A2 reads as a skin over bars |
| Maintenance | 4 | 3 | A1: one derivation path; A2: every new window/metric re-materialises everything |
| Offline/ToS fit | 5 | 4 | both offline; A1's `source_rev` gives provenance per log (closes gotcha H6 mixed-provenance) |
| **Total** | **26** | **21** | |

**Winner: A1.** A2's only edge is Ticket 01 speed, and that edge is one `Metric` trait plus a memo table.

### Skeptic's case against A1

| # | Failure mode | Why it is likely | Cheapest throwaway prototype |
|---|---|---|---|
| A-F1 | Derive-per-view is too slow for team/season/league windows (league-wide rolling xwOBA = 180K PA; COMP over 30 teams) and the memo misses on every `as_of` change | memo key includes `as_of`; the in-season "as-of cursor" changes per keystroke | 60-line Rust bench: rolling window over 700K synthetic events with upstream `stats.rs` `windows`; target < 16 ms per derive at 180K, else memo must key on `(window, log_sha)` and slice by as-of after |
| A-F2 | Savant rows churn (pitch_type review, backfills like `miss_distance`) → `log_sha` changes on every refresh, invalidating every memo and shifting `seq` if rows are added/removed mid-season | B2/B3 facts: backfills happen, `sv_id` empty, classifications "may change" | Pull one player-season today and again in 7 days; diff row count, `seq` stability (by `game_pk`+`pitch_number`), field churn — decides whether `seq` keys on `(game_pk, at_bat_number, pitch_number)` instead of position (prototype P1 extension) |
| A-F3 | The index-based chart cannot show gaps/epochs: IL stints, season boundaries, coverage intervals render as a continuous line or as zero | **Partly refuted on read:** `lod.rs:128` "NaN values break the line"; `shader_line.wgsl:32,57` emit a degenerate segment when either endpoint is non-finite, so a level-0 `NaN` is a gap. Unverified: what `LinePyramid::reduce` does to a NaN neighbour at coarser LOD levels (does the gap survive zoom-out?) and how a gap reads on an index axis with no date tick | One `tests/render.rs` pixel test feeding `[1.0, NaN, 2.0]` through `line_ref` at level 0 and at a zoomed-out level; plus the P4 axis prototype for season dividers/as-of cursor |

---

## 2. Arena B — forecasting core: artifact and backtest contract

Governing records: DR-3.4 (native + CLI artifacts), DR-3.7 (< 60 s in `cargo test`), DR-3.12 (MAE + PICP per
season per bucket), DR-3.19 (Marcel on season lines), DR-3.20/3.31 (as-of any date; 6 cuts/season), DR-3.21
(empirical residual quantiles), DR-3.22 (JSON artifact + SQLite index), DR-3.36 (PT model). Prior art:
bayes-horizon's purged walk-forward + PICP/MIS + JSON run tracking; sabr-jev's Jan-1 cutoff and oracle-only joins.

Shared vocabulary:

```rust
pub struct AsOf(pub Date);
pub struct SeasonLine { pub player: PlayerId, pub season: u16, pub age: u8, pub pa: u32, pub ab: u32, pub h: u32, pub bb: u32,
                        pub hbp: u32, pub x2b: u32, pub x3b: u32, pub hr: u32, pub sf: u32, pub so: u32, pub outs: u32 /* pitchers */ }
pub struct Corpus { pub lines: Vec<SeasonLine>, pub constants: Constants, pub rev: CorpusRev }      // DR-3.17/3.18/3.30
pub struct Constants { pub woba: BTreeMap<u16, WobaWeights>, pub park: BTreeMap<(u16, TeamKey), f64>, pub sha256: [u8; 32] }
pub struct CorpusRev { pub lahman_sha: [u8; 32], pub constants_sha: [u8; 32], pub open_season_fetched: Option<DateTime> }
pub enum Bucket { High, Med, Low, None }                                                            // Marcel reliability
pub struct Quantiles { pub p10: f64, pub p50: f64, pub p90: f64, pub lo80: f64, pub hi80: f64 }
```

### B1 — `Projector` trait + walk-forward harness (in-process, pure functions)

```rust
// crate `sabr-forecast` (working name), module `model`
pub struct ModelId { pub name: &'static str, pub version: semver::Version }
pub struct Projection { pub player: PlayerId, pub season: u16, pub as_of: AsOf, pub pa: f64, pub rate: BTreeMap<MetricId, f64>,
                        pub reliability: f64, pub bucket: Bucket }
pub trait Projector: Send + Sync {
    fn id(&self) -> ModelId;
    /// Pure: sees only lines with season < as_of.season, or == with date <= as_of (DR-3.20); the harness enforces it.
    fn project(&self, corpus: &CorpusView<'_>, player: PlayerId, as_of: AsOf) -> Option<Projection>;
}
pub struct CorpusView<'a> { /* corpus filtered to <= as_of; constructed only by the harness */ }
pub struct Marcel;   impl Projector for Marcel { /* DR-3.19, 3.34, 3.36 */ fn id(&self) -> ModelId { todo!() } fn project(&self, c: &CorpusView<'_>, p: PlayerId, a: AsOf) -> Option<Projection> { todo!() } }
pub struct PriorSeason; impl Projector for PriorSeason { fn id(&self) -> ModelId { todo!() } fn project(&self, c: &CorpusView<'_>, p: PlayerId, a: AsOf) -> Option<Projection> { todo!() } }

// module `harness`
pub struct Protocol { pub seasons: RangeInclusive<u16>, pub cuts: Vec<(u8, u8)> /* (month, day): DR-3.31 */, pub floor: Floor, pub metric: MetricId }
pub enum Floor { Fixed(f64), LeagueMinus(f64), LeagueAvgAtPa(u32) }   // the three published conventions, spec picks one
pub struct Residual { pub player: PlayerId, pub season: u16, pub as_of: AsOf, pub bucket: Bucket, pub pa_actual: u32, pub err: f64 }
pub struct FoldResult { pub season: u16, pub as_of: AsOf, pub residuals: Vec<Residual>, pub mae_pa_weighted: f64, pub pa_mae: f64 }
pub struct Scores { pub mae: f64, pub pa_mae: f64, pub picp80: BTreeMap<(u16, Bucket), f64>, pub mis80: f64, pub passes_gate: bool }
pub fn walk_forward(model: &dyn Projector, corpus: &Corpus, protocol: &Protocol) -> Vec<FoldResult> { todo!() }
pub fn score(folds: &[FoldResult], protocol: &Protocol) -> Scores { todo!() }

// module `interval` (DR-3.21): quantiles from residuals of folds strictly before the target season (nested walk-forward)
pub struct ResidualModel { pub by_bucket: BTreeMap<Bucket, Vec<f64>>, pub fitted_through: u16 }
pub fn fit_intervals(folds: &[FoldResult], through: u16) -> ResidualModel { todo!() }
pub fn interval(rm: &ResidualModel, p: &Projection) -> Quantiles { todo!() }

// module `artifact` (DR-3.22)
pub struct Artifact { pub model: ModelId, pub as_of: AsOf, pub corpus: CorpusRev, pub protocol_sha: [u8; 32],
                      pub scores: Scores, pub projections: Vec<(Projection, Quantiles)>, pub created: DateTime, pub tool: &'static str }
pub fn write(dir: &XdgDirs, a: &Artifact) -> Result<PathBuf> { todo!() }       // one JSON per fit, content-addressed name
pub fn index(store: &Store, path: &Path, a: &Artifact) -> Result<()> { todo!() } // SQLite index row
pub fn latest(store: &Store, model: &ModelId, as_of: AsOf) -> Result<Option<Artifact>> { todo!() }

// CLI (DR-3.4, 3.27, 3.32): `sabr fit --model marcel --as-of 2026-06-01`, `sabr backtest`, `sabr report <entity>`
```

### B2 — Declarative run manifest over content-addressed stages (mini-DVC)

```rust
// crate `sabr-forecast`, module `stage`
pub struct Hash([u8; 32]);
pub trait Stage: Send + Sync {
    fn name(&self) -> &'static str;
    fn inputs(&self) -> Vec<Hash>;                       // declared before run; the run key = hash(name, inputs, params)
    fn run(&self, ctx: &Ctx) -> Result<Output>;          // writes outputs into the content store
}
pub enum Output { Table(PathBuf), Json(PathBuf), Scores(PathBuf) }
pub struct Ctx<'a> { pub store: &'a ContentStore, pub params: &'a toml::Table }

// module `manifest` — a TOML DAG: [stage.features] deps=["corpus"] … [stage.score] deps=["project","actuals"]
pub struct Manifest { pub stages: Vec<(String, Box<dyn Stage>)>, pub edges: Vec<(String, String)> }
pub fn load(path: &Path) -> Result<Manifest> { todo!() }
pub fn plan(m: &Manifest, store: &ContentStore) -> Vec<String> { todo!() }   // stages whose run key is missing → to run
pub fn execute(m: &Manifest, store: &ContentStore) -> Result<RunReport> { todo!() }

// module `content` — the registry *is* the content store
pub struct ContentStore { root: PathBuf }
impl ContentStore {
    pub fn put(&self, bytes: &[u8]) -> Result<Hash> { todo!() }
    pub fn get(&self, h: &Hash) -> Result<Vec<u8>> { todo!() }
    pub fn runs(&self) -> Result<Vec<RunRecord>> { todo!() }        // run key → outputs, timestamps, tool version
}
pub struct RunRecord { pub key: Hash, pub stage: String, pub outputs: Vec<(String, Hash)>, pub created: DateTime }

// the forecasting stages are concrete Stage impls: FeaturesStage (season lines ≤ as_of), FitStage(Marcel),
// ProjectStage, ActualsStage (oracle join, eval-only — sabr-jev rule), ScoreStage (MAE/PICP/MIS), IntervalStage.
// A projection panel resolves (model, as_of) → run key → ProjectStage output; "version" = stage code hash + params.
```

### Scores — Arena B

| Criterion | B1 trait + harness | B2 manifest + content store | Notes |
|---|---|---|---|
| Effort | 4 | 2 | B2 builds a DAG runner and content store before Marcel exists |
| Performance | 5 | 4 | both trivial for Marcel; B2 adds hashing + file I/O per stage |
| Reproducibility | 4 | 5 | B1 pins corpus/constants/protocol hashes in the artifact; B2 is content-addressed end to end |
| Portfolio signal | 4 | 4 | B1 shows a clean model/harness/registry contract; B2 shows pipeline engineering — different audiences, equal weight |
| Maintenance | 5 | 2 | B2 is a second build system inside the repo; stage boundaries ossify |
| Offline/ToS fit | 5 | 5 | both run from the bundled corpus |
| **Total** | **27** | **22** | |

**Winner: B1** — with B2's one real idea stolen: the artifact's name is the content hash of (model id, corpus rev,
protocol sha, as-of), so re-running a fit is idempotent.

### Skeptic's case against B1

| # | Failure mode | Why it is likely | Cheapest throwaway prototype |
|---|---|---|---|
| B-F1 | `Projection` is Marcel-shaped (point + reliability); the v1.1 quantile/Bayesian model has no point estimate and no Marcel bucket, so `interval()` and the bucketed gate cannot score it without a rewrite | the trait was designed around one model | Write the trait signature twice on paper for a toy quantile model (`project` returns `Distribution` with `.quantile(q)`, `.mean()`); if Marcel fits that shape too, ship the distribution-valued trait from day one |
| B-F2 | Residual quantiles pooled across *all* folds leak the target season into its own interval (DR-3.21 says strictly-before folds); the first 3 seasons then have no residuals → no interval → gate undefined | the convenient implementation pools | 40-line script on Marcel residuals: PICP computed nested vs pooled; if they differ by > 1 point the nested rule is a test, and seasons 2001–2003 are declared warm-up in the protocol |
| B-F3 | The Steamer/ZiPS fixture cannot be joined: their files key on FanGraphs ids (register `key_fangraphs` is empty for new debuts) or names; missing joins silently bias the aggregate comparison | C2 facts | Download one Steamer preseason file (free registration), list its id columns, join to the register subset, count unmatched — fixes the join key and the "unmatched" accounting before the fixture is built |

---

## 3. Arena C — data layer: source / fetch / cache shape

Governing records: DR-3.6 (per-entity on-demand, cached, never bulk; risk = tolerance), DR-3.17 (Lahman pinned +
Stats API open season), DR-3.25 (Cot's sheets, cited), DR-3.27 (one refresh path), DR-3.29 (register subset),
DR-3.1/3.16 (committed slice). Six sources: Savant CSV, Stats API JSON, Lahman CSV (pinned), Retrosheet (pinned,
constants tool), Chadwick register (pinned, trimmed), Cot's sheets (CSV export). Upstream shape being replaced:
`#[async_trait] trait MarketData` (8 methods, `Arc<dyn>`), `Markets` router by `AssetClass`, `Fallback<P>`,
`BarCache`, `from_env`; gotcha H6: synthetic/real pages merged without provenance.

### C1 — One `Source` trait, raw-payload store with provenance, normalisers

Every source implements the same two-method trait; every fetched payload is stored raw (bytes + provenance) and
normalised into the domain model by a pure function; the bundled dataset is the same store pre-populated, so
offline is "no network source registered", not a different code path.

```rust
// crate `sabr-data`, module `source`
pub enum Fetch {                                                   // one request vocabulary across six sources
    StatcastPlayerSeason { player: PlayerId, season: u16 },        // Savant CSV, per entity (DR-3.6)
    SeasonLines { season: u16 },                                   // Stats API, open season only (DR-3.17)
    Transactions { from: Date, to: Date },                         // Stats API (DR-3.13)
    Roster { team: u16, season: u16 },                             // Stats API (DR-3.25 team join)
    MilbSeasonLines { sport_id: u8, season: u16 },                 // Stats API (DR-3.24)
    Draft { year: u16 },                                           // Stats API (DR-3.24)
    Payroll { season: u16 },                                       // Cot's sheet export (DR-3.25)
}
pub struct Policy { pub min_interval: Duration, pub ttl_closed: Option<Duration>, pub ttl_open: Duration,
                    pub attribution: &'static str, pub bulk: bool /* always false: DR-3.6 */ }
pub struct Provenance { pub source: &'static str, pub url: String, pub fetched: DateTime, pub sha256: [u8; 32], pub attribution: &'static str }
pub struct Payload { pub bytes: Vec<u8>, pub prov: Provenance }
#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> &'static str;
    fn policy(&self) -> Policy;
    fn accepts(&self, req: &Fetch) -> bool;
    async fn fetch(&self, req: &Fetch) -> Result<Payload, FetchError>;
}
pub struct Savant; pub struct StatsApi; pub struct Cots;            // impl Source for each: todo!()

// module `store` — raw payloads, keyed by request
pub struct RawStore { conn: rusqlite::Connection, blobs: PathBuf }  // payloads as files (CSV/JSON, zstd), index in SQLite
impl RawStore {
    pub fn get(&self, req: &Fetch) -> Result<Option<Payload>> { todo!() }
    pub fn put(&self, req: &Fetch, p: &Payload) -> Result<()> { todo!() }
    pub fn stale(&self, req: &Fetch, policy: &Policy, now: DateTime) -> Result<bool> { todo!() }   // treasury.rs TTL pattern
}

// module `normalize` — pure, versioned, re-runnable over the raw store
pub trait Normalize<T> { const VERSION: u32; fn normalize(p: &Payload) -> Result<T, SchemaError>; }
pub struct SavantAllowlist;  impl Normalize<EventLog> for SavantAllowlist { const VERSION: u32 = 1; fn normalize(p: &Payload) -> Result<EventLog, SchemaError> { todo!() } } // header-name parse, BOM strip (B2)
pub struct StatsSeason;      impl Normalize<Vec<SeasonLine>> for StatsSeason { const VERSION: u32 = 1; fn normalize(p: &Payload) -> Result<Vec<SeasonLine>, SchemaError> { todo!() } }
pub struct CotsSheet;        impl Normalize<Vec<Contract>> for CotsSheet { const VERSION: u32 = 1; fn normalize(p: &Payload) -> Result<Vec<Contract>, SchemaError> { todo!() } }
pub struct TransactionsDedup; impl Normalize<Vec<Transaction>> for TransactionsDedup { const VERSION: u32 = 1; fn normalize(p: &Payload) -> Result<Vec<Transaction>, SchemaError> { todo!() } } // (person, date, typeCode, description)

// module `data` — what the app calls; the only async surface the worker sees
pub struct Data { sources: Vec<Arc<dyn Source>>, raw: RawStore, model: sabr_model::Store, bundled: BundledSet }
impl Data {
    pub fn from_env(dirs: &XdgDirs) -> Result<Self> { todo!() }   // SABR_OFFLINE=1 → no network sources; sample always loaded
    pub async fn events(&self, p: PlayerId, season: u16) -> Result<Loaded<EventLog>> { todo!() }   // bundled → raw store → source
    pub async fn season_lines(&self, season: u16) -> Result<Loaded<Vec<SeasonLine>>> { todo!() }
    pub async fn transactions(&self, since: Date) -> Result<Loaded<Vec<Transaction>>> { todo!() }
    pub async fn payroll(&self, season: u16) -> Result<Loaded<Vec<Contract>>> { todo!() }
    pub fn catalog(&self) -> &Catalog { todo!() }                 // register subset, in memory (DR-3.29)
    pub async fn refresh(&self, scope: RefreshScope) -> Result<RefreshReport> { todo!() }  // the one path (DR-3.27)
}
pub struct Loaded<T> { pub value: T, pub prov: Provenance, pub freshness: Freshness }   // every value carries its provenance
pub enum Freshness { Bundled, Cached { age: Duration }, Fetched }
pub struct BundledSet { /* include_bytes! / data dir: slice, Lahman 1998–2025, constants, register subset */ }
```

### C2 — Typed per-domain repositories, composed

Each domain gets its own trait with its own cache table and its own bundled/live implementations; a `Data` struct
composes them, mirroring upstream's `Markets` router but by domain instead of asset class; `Fallback` is per
repository.

```rust
// crate `sabr-data`
#[async_trait] pub trait StatcastRepo: Send + Sync {
    async fn pitches(&self, p: PlayerId, season: u16) -> Result<EventLog>;
}
#[async_trait] pub trait SeasonRepo: Send + Sync {
    async fn lines(&self, season: u16) -> Result<Vec<SeasonLine>>;
    fn closed_seasons(&self) -> RangeInclusive<u16>;               // Lahman-backed
}
#[async_trait] pub trait EventsRepo: Send + Sync {                 // tr-congress skeleton re-parameterised (audit H8/H12)
    async fn transactions(&self, since: Date) -> Result<Vec<Transaction>>;
    async fn injuries(&self, since: Date) -> Result<Vec<Transaction>>;
}
#[async_trait] pub trait PayrollRepo: Send + Sync { async fn contracts(&self, season: u16) -> Result<Vec<Contract>>; }
#[async_trait] pub trait FarmRepo: Send + Sync { async fn lines(&self, sport_id: u8, season: u16) -> Result<Vec<SeasonLine>>; async fn draft(&self, year: u16) -> Result<Vec<Pick>>; }
pub trait CatalogRepo: Send + Sync { fn search(&self, q: &str) -> Vec<CatalogHit>; fn by_mlbam(&self, id: u32) -> Option<Person>; }

pub struct SavantLive; pub struct SavantCached<R: StatcastRepo> { inner: R, cache: PitchCache }   // one cache type per repo
pub struct LahmanBundled; pub struct StatsApiSeason; pub struct SeasonFallback<A: SeasonRepo, B: SeasonRepo> { closed: A, open: B }
pub struct CotsLive; pub struct PayrollCached<R: PayrollRepo> { inner: R, cache: ContractCache }
pub struct Bundled<R> { sample: R }                                 // offline impls per repo

pub struct Data {
    pub statcast: Arc<dyn StatcastRepo>, pub seasons: Arc<dyn SeasonRepo>, pub events: Arc<dyn EventsRepo>,
    pub payroll: Arc<dyn PayrollRepo>, pub farm: Arc<dyn FarmRepo>, pub catalog: Arc<dyn CatalogRepo>,
}
impl Data { pub fn from_env(dirs: &XdgDirs) -> Result<Self> { todo!() } pub async fn refresh(&self) -> Result<RefreshReport> { todo!() } }
// caches: pitches(entity_key, role, season, seq, …), season_lines(season, player, …), contracts(season, player, …) — each its own table and TTL
```

### Scores — Arena C

| Criterion | C1 raw store + normalisers | C2 typed repos | Notes |
|---|---|---|---|
| Effort | 3 | 4 | C2's first repo is upstream's trait pattern verbatim; C1 fronts the raw store and `Normalize` before the first panel |
| Performance | 4 | 5 | C1 re-normalises on cache hit unless the model store memoises (it does: A1 `upsert_log`); C2 reads typed rows |
| Reproducibility | 5 | 3 | C1 keeps bytes + hash + URL + time for every value; a normaliser bug is fixed by re-running over the raw store; C2 caches the parse, so a bug means re-fetching (ToS cost) |
| Portfolio signal | 4 | 3 | C1 is the lakehouse "raw → curated" shape (statcast-lakehouse prior art); C2 is a repository pattern |
| Maintenance | 4 | 2 | six sources: C1 = six `Source` impls + six normalisers, one store, one policy type; C2 = six traits × (live, cached, bundled, fallback) impls and six cache schemas |
| Offline/ToS fit | 5 | 4 | C1's `Policy { bulk: false }` and attribution live on the source; `Loaded.prov` carries the citation to the panel (DR-3.25); C2 must repeat it per repo |
| **Total** | **25** | **21** | |

**Winner: C1.**

### Skeptic's case against C1

| # | Failure mode | Why it is likely | Cheapest throwaway prototype |
|---|---|---|---|
| C-F1 | The `Fetch` enum becomes a god-type: every new source adds a variant, every `accepts` grows, and a request's cache key is whatever `Debug` prints | enums across six sources always do | 30-line spike: implement `RawStore::get/put` with a `Fetch::key() -> String` that is explicit per variant and tested for stability; if the key needs source-specific fields (Cot's sheet id), the variant carries them and the test pins the format |
| C-F2 | Open-season TTL is wrong in both directions: too long and the in-season as-of cursor shows yesterday's rows; too short and a `COMP` across 30 teams fires 30 Savant requests — a de-facto bulk pull DR-3.6 forbids | per-entity fetch × many entities = bulk | Fetch one player-season twice across a game day and diff (also answers A-F2); then count requests a 30-team `COMP` would issue and set the policy to refuse > N fetches per minute with a visible "N stale" badge instead of fetching |
| C-F3 | Cot's sheets are hand-maintained: multi-row headers, `*` deferral markers, "FA"/"cond opt" cells, per-year sheet ids scraped from an Anubis-gated page; `CotsSheet::normalize` breaks every spring | DR-3.25 risk record | Parse 2016, 2021 and 2026 sheets with one normaliser; count anomalies and unparsed cells; if > 1% the panel degrades to "unparsed" rows rather than the fetch failing (and the manual-import path is exercised by the same normaliser) |

---

## 4. Cross-arena consequences (facts, not decisions)

- A1 + C1 compose: `Data::events` returns `Loaded<EventLog>` whose `prov` is the `SourceRev` A1 keys its memo on.
- B1's `Corpus` is built from C1's `season_lines` (open season) plus the bundled Lahman files; `CorpusRev` is the
  pair of hashes the artifact pins.
- All three keep `tr-chart`, `dock`, themes, command parser and keyboard model untouched (audit H1–H5); the
  adapters are `to_line`/`axis_labels` (A) and the worker's `Request`/`Response` re-enumeration (C).
- Prototype queue additions from the skeptic sections: A-F1 bench, A-F2/C-F2 churn diff, A-F3 NaN pixel test,
  B-F1 trait-shape paper test, B-F2 nested-vs-pooled PICP, B-F3 Steamer id join, C-F1 key spike, C-F3 Cot's
  three-season parse. All single-file, fake data where possible, discarded afterwards (Phase 5 rule).

## 5. Adversarial review (second model family: `openai-codex/gpt-6-astra` via the read-only scout; full text at
`agent://ArenaAdversary`, session-scoped)

Objections verified by the author against the source before being accepted; each ends with its disposition.

### Arena A
- **A-obj1 — "tr-chart untouched" was an overclaim.** `price_pane` is typed on `&SeriesGpu` and sizes the viewport
  from `series.bars.pyramid` (scene.rs:100-233); `line_ref` output is only an overlay/sub-pane input; the app's
  x-axis, crosshair, range and anchors read `bars[i].ts` + `Timeframe` (chart_view.rs:249-250, 890-892, 958, 1178).
  Both candidates pay the main-pane rewrite the audit already lists (§4.1, "rewriting these four"); A1 hid it.
  **Accepted:** adapter becomes `to_bars`/a line-primary pane; `Anchor.ts` on a PA axis = packed (season, seq).
- **A-obj2 — NaN gaps are LOD-unstable.** `LinePyramid::build` merges `|_, later| later` (lod.rs:138, verified), so
  a 1–3-point coverage gap (B3) vanishes or erases a finite neighbour when zoomed out; only level 0 is safe
  (shader_line.wgsl:32,57). **Accepted:** `reduce` is closure-parameterised (lod.rs:147) — add a gap-preserving
  policy (`any NaN → NaN`) and run the A-F3 pixel test at level ≥ 2.
- **A-obj3 — the memo layer is dead weight and mis-keyed.** `SeriesKey` lacks a metric-code version (a fixed xwOBA
  formula would return stale memos); backward windows have the prefix property so `as_of` in the key is over-keying;
  the open-season `log_sha` changes nightly so in-season hit rate ≈ 0; per-player derive is ≤ 3K events — microseconds.
  Also `stats::windows` is `pub(super)` (stats.rs:12) and NaN-poisons any window containing a non-finite input
  (stats.rs:45-47) — the opposite of "sum defined / count defined"; `Metric::eval(&[Event])` is slice evaluation,
  O(n·w) not O(n). **Accepted:** drop `memo_get/put` from v1; metrics get a `VERSION`; the rolling engine is a
  sabermetric one (defined-count windows), not `stats.rs`; A-F1's bench targets that engine.
- **A-obj4 — single-season `derive` and a `Unit` that cannot express DR-3.35.** `derive(log: &EventLog)` cannot
  build a 50-PA window on April 5 or a multi-season chart; `Seq` dense per season collides across seasons; `Unit`
  lacks `Ab`/`Bip` while the Carleton x-points are in AB and BIP. **Accepted:** `derive(logs: &[EventLog], …)`,
  `Point { season, seq }`, `Unit += Ab, Bip`, `Metric::threshold() -> Option<(Unit, u32)>`.
- Score dispute: portfolio 5/2 "is taste"; with C1's raw store A2's `materialize` is an equally pure point-in-time
  function (repro 3→4). **Partly accepted** — see re-score.
- Verdict: not overturned, but "A1 minus the memo plus fixes 1–4 is A2 with derive-at-view"; pick on whether
  arbitrary windows/as-of at view time are wanted in v1.

### Arena B
- **B-obj1 — BLOCKING: no source exists for the in-season cuts.** `Corpus { lines: Vec<SeasonLine> }` is season
  totals with no date; DR-3.31's May–Sep cuts for 2001–2025 need per-date player lines, and no decision names a
  source (Lahman is season-level; Stats API per-player game logs for 1,400 players × 25 seasons is the bulk pull
  DR-3.6 forbids). As written the 150 fits are 25, the ≈125 in-season evaluations feeding DR-3.21 do not exist,
  and ROS calibration is untestable. **Accepted; two real options verified 2026-10-21:**
  (a) **Retrosheet event files** on the Chadwick mirror (`seasons/<year>/<TEAM>.EVA|EVN`, every season through
  2026, ≈420 KB/team/season; 2024ARI.EVN = 8,501 plays whose first token is one of ≈15 outcome codes —
  fielded-out, NP, K, S, W, D, HR, SB, HP, WP, T, E, IW, FC — covering the batting line; runs/earned runs are
  explicit advancement markers) → an in-repo play parser derives cumulative player lines at the 5 cut dates ≈ 190K
  rows, fully redistributable under the Retrosheet notice; the parser is validated against Lahman season totals
  (which is prototype P2). Effort: a parser ticket (M) before the ROS backtest.
  (b) **Stats API `stats=byDateRange&playerPool=All`** (verified: 2015-03-01→05-01 returns 718 hitters incl. 0-PA
  and 457 pitchers with `battersFaced/outs/earnedRuns/…`): 25 seasons × 5 cuts × 2 groups = 250 league-level
  requests ≈ 300K MLB-sourced rows that must be committed for offline backtests — the DR-3.1 exposure class, ×80.
  (c) Preseason-only backtests for closed seasons; ROS calibrated prospectively on open seasons only (thin).
  → **Verdict question V-B1 in section 6.**
- **B-obj2 — `CorpusView` is privacy, not enforcement.** Any `Projector` in the crate can hold its own `Arc<Corpus>`.
  **Accepted:** keep the view as the API; the real guard is a harness test per fold — perturb every season > as_of
  and assert byte-identical projections (fits DR-3.7).
- **B-obj3 — the bucketed PICP gate is flaky at these cell sizes.** `[INFERENCE, adversary]` ≈ 150 effective
  residuals in the `None` bucket → binomial SE at p = 0.8 ≈ 0.033; the ±0.05 band is ≈ 1.5 SE, so a *correctly
  calibrated* model fails a 12-cell gate on luck in a large share of runs. **Accepted as a defect in DR-3.12's
  wording** — needs the user: gate = "0.80 inside the Wilson 95% interval of observed PICP per cell, cells below
  n_min merged upward" (DR-3.21's own merge clause), with `(picp, n)` carried per cell. → **V-B2.**
- **B-obj4 — signatures invite the B-F2 leak and will not compile as written.** `score(folds)` receiving flat folds
  makes pooled interval fitting the obvious implementation → **Accepted:** `walk_forward` attaches `lo80/hi80` to
  each `Residual` from a `ResidualModel` fitted on strictly-earlier folds; `score` only counts. `&'static str` ids
  cannot `Deserialize` → `String`. `SeasonLine` is batting-shaped → separate `BattingLine`/`PitchingLine`.
- Verdict: not overturned; B2 is a build system for a 150-fit problem; keep only its content-hash artifact name.

### Arena C
- **C-obj1 — a request-keyed raw cache is wrong for range-shaped requests.** `Transactions { from, to }` keys on a
  moving range (never hits) and every consumer needs a queryable row table anyway (dedup, team join, event log) —
  so C1 = C2's typed row tables *plus* raw retention; the maintenance 4/2 is inflated (three of six sources are
  bundled-only with no live/cached/fallback impl). **Accepted:** maintenance 3/3; C1's real edge is reproducibility.
- **C-obj2 — `accepts` is runtime routing for a compile-time mapping.** Each variant belongs to exactly one source;
  the worker never sees `Fetch` (it calls typed methods the way it calls `provider.history` per `Request` today,
  worker.rs:508, 541). **Accepted:** `Fetch` becomes `pub(crate) FetchKey` (the raw-store key); typed source slots
  on `Data`; `accepts` deleted.
- **C-obj3 — `rusqlite::Connection` is `!Sync`.** `RawStore { conn }` inside `async fn(&self)` cannot be
  `Arc`-shared across tokio tasks; upstream wraps (`std::sync::Mutex<Option<BarCache>>`, `tokio::sync::Mutex<Db>`,
  worker.rs:296-297) and never holds a guard across an await. **Accepted:** `Mutex<Connection>`; lock → get → drop
  → `fetch.await` → lock → put; `Provenance.source: String`.
- **C-obj4 — `bulk: false` is a comment, not a control.** Nothing bounds fan-out (COMP × 30 teams = 30 Savant
  requests). **Accepted:** `Policy { max_per_command }` + `Freshness::Stale` surfaced in the panel — a type, not a
  prototype.
- Verdict: not overturned (raw-byte retention is what makes a normaliser bug cheap under DR-3.6), but the honest
  shape is **C2's typed repos + a raw payload side table**, "also the cheapest thing to build first".

### Re-scores after the review

| Arena | Candidate | Effort | Perf | Repro | Portfolio | Maint | Offline/ToS | Total |
|---|---|---|---|---|---|---|---|---|
| A | A1′ event log, derive-at-view, no memo, fixes 1–4 | 3 | 4 | 5 | 4 | 4 | 5 | **25** |
| A | A2 materialised rows (with raw store) | 4 | 5 | 4 | 3 | 3 | 4 | 23 |
| B | B1′ trait + harness, residual-attached intervals, hashed artifact name, daily corpus per V-B1 | 3 | 5 | 5 | 4 | 5 | 5 | **27** |
| B | B2 manifest + content store (same corpus hole) | 2 | 4 | 5 | 4 | 2 | 5 | 22 |
| C | C1′ typed `Data` methods + per-domain row tables + raw payload side table, `FetchKey`, `Mutex<Connection>`, fan-out budget | 3 | 5 | 5 | 4 | 3 | 5 | **25** |
| C | C2 typed repos, no raw retention | 4 | 5 | 3 | 3 | 3 | 4 | 22 |

Margins after honest scoring: A 2 points (was 5), B 5 (was 5, but B1 is unbuildable without V-B1), C 3 (was 4).

## 6. Verdicts (user, 2026-10-21)

| Verdict | Pick | Consequence |
|---|---|---|
| V-A | **A1′** — events canonical, derive at view (no memo; metric `VERSION`; multi-season `derive`; `Unit += Ab, Bip`; gap-preserving LOD policy; main-pane rewrite per audit §4.1) | Phase 8 scaffolds `Entity/Window/Metric/Series` as in §1 with the A-obj fixes; the sabermetric rolling engine (defined-count windows) replaces `stats::windows` for metric evaluation |
| V-B1 | **Retrosheet event files → in-repo play parser → bundled cumulative lines at the 5 cuts** | New decision record **DR-3.37** in `01-grill.md`; a parser ticket (M) precedes the ROS backtest; the parser is validated against Lahman season totals (prototype P2 becomes its acceptance test); `Corpus` gains `cuts: Vec<CutLine>` |
| V-B2 | **Wilson-interval gate**: 0.80 inside the 95% Wilson interval of observed PICP per (season, bucket); cells below `n_min` merge upward; `(picp, n)` carried per cell; [0.75, 0.85] stays the reporting band | Amends DR-3.12's wording → **DR-3.12a** in `01-grill.md` |
| V-C | **C1′** — typed `Data` methods, per-domain row tables, raw payload side table with provenance, `pub(crate) FetchKey`, `Mutex<Connection>`, `Policy { max_per_command }` + `Freshness::Stale` | Phase 8 scaffolds `sabr-data` as in §3 with the C-obj fixes; `Loaded<T>.prov` carries attribution to panels |

Prototype queue carried to Phase 5 (throwaway, single-file): A-F1 bench (defined-count rolling engine at 180K
events), A-F2/C-F2 Savant churn diff across a game day and across 7 days, A-F3 NaN pixel test at LOD ≥ 2, B-F1
distribution-valued `Projector` paper test, B-F2 nested-vs-pooled PICP, B-F3 Steamer id-join, C-F1 `FetchKey`
stability spike, C-F3 Cot's three-season parse, plus a Retrosheet play-token coverage count over one full season
(V-B1 sizing), P1–P7 from the Phase 2 record.
