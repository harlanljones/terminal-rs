//! Background worker. All network, SQLite, indicator and LOD-pyramid work runs on a
//! tokio runtime; the UI thread only sends [`Request`]s and drains [`Response`]s, so it
//! never blocks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use sabr_data::savant::{self, SavantMode};
use sabr_data::track::{AlertRow, TrackStore};
use sabr_model::entity::Entity;
use sabr_model::metric::registry;
use time::Date;
use tokio::sync::Mutex;
use tr_chart::gpu::{LineKind, LineRef};
use tr_chart::scene::{Overlay, SeriesGpu, line_ref, marker_ref};
use tr_congress::db::{Alert, PoliticianSummary};
use tr_congress::performance::{self, Performance, TradeReturn};
use tr_congress::refresh::{self, HttpSource, NewTradeAlert, RefreshConfig};
use tr_congress::{Db, Politician, Trade};
use tr_core::indicators::registry::{Input, Plot, Tone};
use tr_core::{Bar, Security, Series, Timeframe};
use tr_data::{BarCache, Provider, Quote, SearchHit, YieldCurves};

use crate::command::IndicatorSpec;
use crate::theme;

pub type PaneId = u64;

/// Minutes between Congress disclosure checks (`TR_CONGRESS_REFRESH_MIN`; 0 disables).
pub const CONGRESS_REFRESH_MIN: u64 = 30;

pub enum Request {
    /// Load the brokerage portfolio through SnapTrade (`None` = the simulated sample).
    Portfolio(Option<tr_broker::Credentials>),
    /// Register a SnapTrade user under the partner keys.
    BrokerRegister {
        creds: tr_broker::Credentials,
        user_id: String,
    },
    /// Generate a read-only Connection Portal link (optionally to reconnect a connection).
    BrokerPortal {
        creds: tr_broker::Credentials,
        dark: bool,
        reconnect: Option<String>,
    },
    /// Remove a brokerage connection at the user's explicit request.
    BrokerDisconnect {
        creds: tr_broker::Credentials,
        id: String,
    },
    /// Option chain for one expiration (`None` = nearest).
    Options {
        pane: PaneId,
        symbol: String,
        expiration: Option<i64>,
    },
    /// Headlines for a market key (`None` = general market news).
    News {
        pane: PaneId,
        symbol: Option<String>,
    },
    OrderBook {
        pane: PaneId,
        symbol: String,
        depth: usize,
    },
    History {
        pane: PaneId,
        generation: u64,
        symbol: String,
        tf: Timeframe,
        limit: usize,
    },
    /// Prepend older history to `existing`.
    MoreHistory {
        pane: PaneId,
        generation: u64,
        symbol: String,
        tf: Timeframe,
        existing: Arc<Vec<Bar>>,
        limit: usize,
    },
    Indicators {
        pane: PaneId,
        generation: u64,
        bars: Arc<Vec<Bar>>,
        tf: Timeframe,
        specs: Vec<IndicatorSpec>,
        /// Colours are baked into overlays, so the UI passes its current palette.
        palette: theme::Palette,
    },
    Compare {
        pane: PaneId,
        generation: u64,
        symbol: String,
        tf: Timeframe,
        base: Arc<Vec<Bar>>,
    },
    Markers {
        pane: PaneId,
        symbol: String,
    },
    Quotes(Vec<String>),
    Latest {
        limit: usize,
    },
    TickerTrades {
        pane: PaneId,
        symbol: String,
    },
    Profile {
        pane: PaneId,
        query: String,
    },
    SearchPoliticians(String),
    Track {
        query: String,
        on: bool,
    },
    Alerts,
    MarkAlertsSeen,
    RefreshCongress,
    /// A pane closed or changed: stop its live feed and drop its series.
    Forget(PaneId),
    YieldCurves,
    Search(String),
    // ---- sabr (wave 3): series / projection / backtest / watchlist ----------------------
    /// `<ENTITY> CAST <METRIC> [WINDOW] [<AS-OF YYYY-MM-DD>]`: derive a metric series from the
    /// bundled sample; `as_of` defaults to today.
    Series {
        entity_key: String,
        metric: String,
        /// `None` uses the metric's default window.
        window: Option<sabr_model::window::Window>,
        /// `None` = today's UTC date.
        as_of: Option<Date>,
        /// Load generation for the UI's newest-wins guard.
        generation: u64,
    },
    /// `<ENTITY> PROJ ROS|NEXT [MODEL]`.
    Projection {
        entity_key: String,
        horizon: crate::command::Horizon,
        model: Option<String>,
        /// Load generation for the UI's newest-wins guard.
        generation: u64,
    },
    /// `BTST <MODEL>`: the full v1 walk-forward suite for one model.
    Backtest {
        model: String,
    },
    /// `NYM ROST` — out of scope in this wave; answered as [`Response::OutOfScope`].
    Rost {
        what: String,
    },
    /// Track a resolved player entity (`TRACK OHTANI`).
    TrackEntity {
        entity_key: String,
        label: String,
    },
    /// Untrack (`UNTRACK OHTANI`).
    UntrackEntity {
        entity_key: String,
        label: String,
    },
    /// Refresh the sabr alerts list (track store).
    SabrAlerts,
    /// Order entry (see `app::trading`); only sent once trading is enabled.
    Trade(TradeRequest),
}

/// A trading call through SnapTrade. Previews never place anything; placing takes a preview
/// the user confirmed.
pub enum TradeRequest {
    Preview {
        creds: tr_broker::Credentials,
        req: tr_broker::trading::OrderRequest,
        ticket: u64,
    },
    Place {
        creds: tr_broker::Credentials,
        preview: Box<tr_broker::trading::OrderPreview>,
        ticket: u64,
    },
    Cancel {
        creds: tr_broker::Credentials,
        account: String,
        order: String,
    },
    /// Connection Portal link requesting trading access.
    Portal {
        creds: tr_broker::Credentials,
        dark: bool,
    },
    /// Which brokerage connections can trade (brokerage support and connection access): one
    /// `/authorizations` request, however many accounts there are.
    Capabilities {
        creds: tr_broker::Credentials,
    },
}

pub enum TradeResponse {
    Preview {
        ticket: u64,
        result: Result<Box<tr_broker::trading::OrderPreview>, String>,
    },
    Placed {
        ticket: u64,
        preview: Box<tr_broker::trading::OrderPreview>,
        result: Result<Option<Box<tr_broker::Order>>, tr_broker::trading::PlaceError>,
    },
    Cancelled {
        account: String,
        order: String,
        result: Result<(), String>,
    },
    Portal(Result<String, String>),
    /// Capabilities by connection (SnapTrade authorization) id.
    Capabilities(Result<std::collections::HashMap<String, tr_broker::trading::Capability>, CapsError>),
}

/// Why trading access could not be read. A rate limit is reported apart from real errors so
/// the app can back off quietly instead of showing an error.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsError {
    pub message: String,
    pub throttled: bool,
    pub retry_after: Option<Duration>,
}

pub struct BuiltIndicator {
    pub spec: IndicatorSpec,
    pub overlays: Vec<(String, Overlay)>,
}

pub struct Profile {
    pub politician: Politician,
    pub trades: Vec<Trade>,
    pub perf: Performance,
    pub returns: Vec<TradeReturn>,
    pub tracked: bool,
}

pub enum Response {
    Portfolio(Result<Box<tr_broker::Portfolio>, String>),
    /// `(userId, userSecret)` for a newly registered SnapTrade user.
    BrokerRegistered(Result<(String, String), String>),
    BrokerPortal(Result<String, String>),
    /// The connection id removed, or why not.
    BrokerDisconnected(Result<String, String>),
    Options {
        pane: PaneId,
        symbol: String,
        chain: Result<Box<tr_data::OptionChain>, String>,
    },
    News {
        pane: PaneId,
        symbol: Option<String>,
        items: Result<Vec<tr_data::NewsItem>, String>,
    },
    OrderBook {
        pane: PaneId,
        symbol: String,
        book: Result<tr_data::OrderBook, String>,
    },
    History {
        pane: PaneId,
        generation: u64,
        symbol: String,
        tf: Timeframe,
        bars: Arc<Vec<Bar>>,
        gpu: SeriesGpu,
        /// Number of bars prepended (paging), 0 otherwise.
        prepended: usize,
        from_cache: bool,
        exhausted: bool,
        /// A live-feed update of the latest bar.
        live: bool,
    },
    HistoryError {
        pane: PaneId,
        generation: u64,
        error: String,
    },
    Indicators {
        pane: PaneId,
        generation: u64,
        built: Vec<BuiltIndicator>,
    },
    Compare {
        pane: PaneId,
        generation: u64,
        symbol: String,
        line: LineRef,
    },
    Markers {
        pane: PaneId,
        symbol: String,
        trades: Vec<Trade>,
    },
    Quotes(Vec<Quote>),
    Latest(Vec<Trade>),
    TickerTrades {
        pane: PaneId,
        symbol: String,
        trades: Vec<Trade>,
    },
    Profile {
        pane: PaneId,
        profile: Option<Box<Profile>>,
        query: String,
    },
    Politicians(Vec<PoliticianSummary>),
    Alerts(Vec<Alert>),
    NewAlerts(Vec<NewTradeAlert>),
    Status(String),
    CongressRefreshed(String),
    Curves(Box<YieldCurves>),
    SearchHits {
        query: String,
        hits: Vec<SearchHit>,
    },
    Trade(Box<TradeResponse>),
    // ---- sabr (wave 3) --------------------------------------------------------------
    /// A derived metric series for a bundled player.
    SabrSeries {
        generation: u64,
        entity_key: String,
        metric: String,
        points: Arc<Vec<sabr_model::series::Point>>,
    },
    /// A projection with its registry stamp (spec story 17: model id, version, training
    /// window, as-of date, last backtest score).
    SabrProjection {
        generation: u64,
        entity_key: String,
        stamp: Box<SabrModelStamp>,
        woba: Option<f64>,
        pa: Option<f64>,
        reliability: Option<f64>,
        /// Fan-chart bands from the latest artifact's interval model (DR-3.40).
        quantiles: Option<Box<SabrQuantiles>>,
    },
    /// A model's backtest: season-score rows plus the pooled MAE and gate verdict.
    SabrBacktest {
        model: String,
        seasons: Vec<SabrSeasonScore>,
        pooled_mae: f64,
        gate_pass: bool,
    },
    /// `TRACK` accepted; `was_new` distinguishes a fresh add.
    SabrTracked {
        entity_key: String,
        label: String,
        was_new: bool,
    },
    /// `UNTRACK` accepted; `was_tracked` distinguishes a real removal.
    SabrUntracked {
        entity_key: String,
        label: String,
        was_tracked: bool,
    },
    /// The current sabr alerts list (newest first).
    SabrAlerts(Vec<AlertRow>),
    /// The request was understood but has no implementation on this path.
    Unsupported {
        reason: String,
    },
    /// The request is a documented stub in this wave (e.g. `NYM ROST`).
    OutOfScope {
        what: String,
    },
}

/// Projection registry stamp (spec story 17).
#[derive(Debug, Clone, PartialEq)]
pub struct SabrModelStamp {
    /// Model id `marcel.batting/1.0.0`.
    pub id: String,
    pub version: String,
    pub train_window: String,
    pub as_of: time::Date,
    /// Pooled wOBA MAE of the model's latest backtest, when its score is cached.
    pub backtest_score: Option<f64>,
}

/// De-standardised forecast bands for the fan chart (DR-3.40): additive deltas on the
/// projection, p05..p95. `None` while the interval model is in warm-up (DR-3.21a).
#[derive(Debug, Clone, PartialEq)]
pub struct SabrQuantiles {
    pub q05: f64,
    pub q10: f64,
    pub q25: f64,
    pub q50: f64,
    pub q75: f64,
    pub q90: f64,
    pub q95: f64,
}

/// A projection with its registry stamp and bands, stored by the drain for the pane.
#[derive(Debug, Clone)]
pub struct SabrProjectionRow {
    pub entity_key: String,
    pub stamp: SabrModelStamp,
    pub woba: f64,
    /// Projected PA (season total); read by the fan pane's status line and future PA axes.
    #[allow(dead_code)]
    pub pa: f64,
    #[allow(dead_code)]
    pub reliability: f64,
    pub quantiles: Option<SabrQuantiles>,
}

/// One scored backtest season for the pane table.
#[derive(Debug, Clone, PartialEq)]
pub struct SabrSeasonScore {
    pub season: u16,
    pub mae: f64,
    pub pa_mae: f64,
    pub gate_pass: bool,
}

pub struct Worker {
    tx: tokio::sync::mpsc::UnboundedSender<Request>,
    pub rx: mpsc::Receiver<Response>,
    _rt: tokio::runtime::Runtime,
}

/// Bars per pane tagged with the load generation they belong to.
type SeriesMap = HashMap<PaneId, (u64, Arc<Vec<Bar>>)>;

struct Shared {
    provider: Provider,
    /// Authoritative bars per chart pane (history, paging and live ticks all merge here),
    /// tagged with the pane's load generation.
    series: std::sync::Mutex<SeriesMap>,
    cache: std::sync::Mutex<Option<BarCache>>,
    db: Mutex<Db>,
    /// Tracked sabr entities and their alerts (`crates/bdata/src/track.rs`).
    track: TrackStore,
    /// Pooled backtest MAE per model id, cached once a `BTST` run completed.
    btst_scores: std::sync::Mutex<HashMap<String, f64>>,
    /// The artifact registry index (DR-3.22): the projection arm reads the latest fitted
    /// interval model from here (DR-3.40 bands). `None` when the index cannot be opened.
    artifacts: Option<sabr_forecast::artifact::ArtifactIndex>,
    out: mpsc::Sender<Response>,
    ctx: egui::Context,
}

impl Shared {
    fn send(&self, r: Response) {
        let _ = self.out.send(r);
        self.ctx.request_repaint();
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("TR_DATA_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/terminal-rs")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("terminal-rs")
    }
}

impl Worker {
    pub fn spawn(ctx: egui::Context, provider: Provider) -> anyhow::Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir)?;
        let db = Db::open(dir.join("congress.sqlite3")).or_else(|e| {
            tracing::warn!("congress db unavailable ({e:#}); using in-memory store");
            Db::open_in_memory()
        })?;
        let cache =
            BarCache::open(dir.join("bars.sqlite3")).map_err(|e| tracing::warn!("bar cache unavailable: {e:#}")).ok();
        let track = TrackStore::open(dir.join("track.sqlite3")).or_else(|e| {
            tracing::warn!("track store unavailable ({e:#}); using in-memory store");
            TrackStore::open_in_memory()
        })?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .thread_name("tr-worker")
            .build()?;
        let (tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let (out, rx) = mpsc::channel();
        let artifacts = {
            let paths = crate::sabr::CliPaths::from_dirs(&sabr_data::xdg::XdgDirs::from_env());
            let _ = std::fs::create_dir_all(&paths.artifacts);
            sabr_forecast::artifact::ArtifactIndex::open(&paths.artifacts.join("index.sqlite3"))
                .map_err(|e| tracing::warn!("artifact index unavailable ({e}); projections show no bands"))
                .ok()
        };
        let shared = Arc::new(Shared {
            provider,
            series: Default::default(),
            cache: std::sync::Mutex::new(cache),
            db: Mutex::new(db),
            track,
            artifacts,
            btst_scores: std::sync::Mutex::new(HashMap::new()),
            out,
            ctx,
        });

        // Request dispatcher: every request runs as its own task.
        let s = shared.clone();
        rt.spawn(async move {
            while let Some(req) = req_rx.recv().await {
                let s = s.clone();
                tokio::spawn(async move { handle(&s, req).await });
            }
        });

        // Scheduled congressional disclosure refresh.
        let s = shared.clone();
        rt.spawn(async move {
            let minutes: u64 = std::env::var("TR_CONGRESS_REFRESH_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(CONGRESS_REFRESH_MIN);
            if minutes == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            let mut tick = tokio::time::interval(Duration::from_secs(minutes * 60));
            loop {
                tick.tick().await;
                congress_refresh(&s).await;
            }
        });
        Ok(Self { tx, rx, _rt: rt })
    }

    pub fn send(&self, req: Request) {
        let _ = self.tx.send(req);
    }

    /// A no-op worker for app tests: no dispatcher runs, requests pile into the returned
    /// channel for the test to inspect, and responses stay inside a channel nothing writes.
    #[cfg(test)]
    pub(crate) fn probe() -> (Worker, tokio::sync::mpsc::UnboundedReceiver<Request>) {
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("probe runtime");
        let (tx, req_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let (out, rx) = mpsc::channel::<Response>();
        std::mem::forget(out);
        (Worker { tx, rx, _rt: rt }, req_rx)
    }
}

async fn congress_refresh(s: &Shared) {
    s.send(Response::Status("Refreshing congressional disclosures (Senate eFD + House Clerk)…".into()));
    let source = match HttpSource::new() {
        Ok(src) => src,
        Err(e) => return s.send(Response::Status(format!("congress refresh failed: {e:#}"))),
    };
    let cfg = RefreshConfig::incremental(&*s.db.lock().await);
    match refresh::refresh(&source, &s.db, &cfg).await {
        Ok(report) => {
            if !report.alerts.is_empty() {
                s.send(Response::NewAlerts(report.alerts.clone()));
            }
            s.send(Response::CongressRefreshed(report.summary()));
        }
        Err(e) => s.send(Response::Status(format!("congress refresh failed: {e:#}"))),
    }
}

fn overlay(values: &[f64], color: egui::Color32, kind: LineKind) -> Overlay {
    let line = if kind == LineKind::Dots { marker_ref(values) } else { line_ref(values) };
    Overlay { line, color: theme::rgba(color), kind, scale: 1.0 }
}

/// Compute every indicator on `bars`. The `i`-th indicator takes the `i`-th series colour;
/// its signal lines, outer bands and up/down lines take theme roles.
pub fn build_indicators(
    bars: &[Bar],
    tf: Timeframe,
    specs: &[IndicatorSpec],
    palette: &theme::Palette,
) -> Vec<BuiltIndicator> {
    let input = Input::new(bars, tf);
    let colors = palette.series.len();
    specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let main = palette.series[i % colors];
            let mut outputs = spec.compute(&input);
            // Columns first, so lines and markers draw over them.
            outputs.sort_by_key(|o| o.plot != Plot::Hist);
            let overlays = outputs
                .into_iter()
                .map(|o| {
                    let color = match o.tone {
                        Tone::Main => main,
                        Tone::Signal if main == palette.amber => palette.cyan,
                        Tone::Signal => palette.amber,
                        Tone::Dim => main.gamma_multiply(0.6),
                        Tone::Up => palette.up,
                        Tone::Down => palette.down,
                        Tone::Series(k) => palette.series[(i + 1 + k as usize) % colors],
                    };
                    let kind = match o.plot {
                        Plot::Line => LineKind::Line,
                        Plot::Hist => LineKind::Hist,
                        Plot::Dots => LineKind::Dots,
                        Plot::Steps => LineKind::Steps,
                    };
                    (o.name.to_string(), overlay(&o.values, color, kind))
                })
                .collect();
            BuiltIndicator { spec: spec.clone(), overlays }
        })
        .collect()
}

/// Align `other`'s closes to `base` timestamps (last close at or before each bar). The
/// chart rebases the result to the first visible bar every frame.
pub fn align_compare(base: &[Bar], other: &[Bar]) -> Vec<f64> {
    let mut out = vec![f64::NAN; base.len()];
    let mut j = 0;
    for (i, b) in base.iter().enumerate() {
        while j + 1 < other.len() && other[j + 1].ts <= b.ts {
            j += 1;
        }
        if let Some(o) = other.get(j).filter(|o| o.ts <= b.ts) {
            out[i] = o.close;
        }
    }
    out
}

fn default_limit(tf: Timeframe) -> usize {
    match tf {
        Timeframe::M1 => 5_000,
        Timeframe::M5 | Timeframe::M15 | Timeframe::M30 => 5_000,
        Timeframe::H1 | Timeframe::H4 => 4_000,
        Timeframe::D1 => 6_000,
        Timeframe::W1 | Timeframe::MN1 => 2_000,
    }
}

/// Bars requested on first load; `TR_HISTORY_LIMIT` overrides (e.g. 1000000 with sample data).
pub fn initial_limit(tf: Timeframe) -> usize {
    std::env::var("TR_HISTORY_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| default_limit(tf))
}

async fn handle(s: &Arc<Shared>, req: Request) {
    match req {
        Request::History { pane, generation, symbol, tf, limit } => {
            if !claim_series(s, pane, generation) {
                return; // a newer load for this pane is already running
            }
            // Paint from the disk cache immediately, then refresh from the network.
            let cached = s.cache.lock().ok().and_then(|c| c.as_ref().and_then(|c| c.load(&symbol, tf, limit).ok()));
            if let Some(bars) = cached.filter(|b| !b.is_empty()) {
                let bars = Arc::new(bars);
                let gpu = build_gpu(bars.clone()).await;
                s.send(Response::History {
                    pane,
                    generation,
                    symbol: symbol.clone(),
                    tf,
                    bars,
                    gpu,
                    prepended: 0,
                    from_cache: true,
                    exhausted: false,
                    live: false,
                });
            }
            match s.provider.history(&symbol, tf, None, limit).await {
                Ok(bars) if !bars.is_empty() => {
                    if let Ok(mut c) = s.cache.lock()
                        && let Some(c) = c.as_mut()
                    {
                        let _ = c.store(&symbol, tf, &bars);
                    }
                    let exhausted = bars.len() < limit / 2;
                    let bars = Arc::new(bars);
                    if !store_series(s, pane, generation, bars.clone()) {
                        return; // superseded while loading
                    }
                    let gpu = build_gpu(bars.clone()).await;
                    s.send(Response::History {
                        pane,
                        generation,
                        symbol: symbol.clone(),
                        tf,
                        bars,
                        gpu,
                        prepended: 0,
                        from_cache: false,
                        exhausted,
                        live: false,
                    });
                    start_live(s.clone(), pane, generation, symbol, tf);
                }
                Ok(_) => s.send(Response::HistoryError { pane, generation, error: format!("no data for {symbol}") }),
                Err(e) => s.send(Response::HistoryError { pane, generation, error: format!("{e:#}") }),
            }
        }
        Request::MoreHistory { pane, generation, symbol, tf, existing, limit } => {
            // Page from the worker's copy (it may hold newer live bars than the UI's).
            let base = current_series(s, pane, generation).filter(|b| !b.is_empty()).unwrap_or(existing);
            let Some(first) = base.first().copied() else { return };
            match s.provider.history(&symbol, tf, Some(first.ts), limit).await {
                Ok(older) => {
                    let older: Vec<Bar> = older.into_iter().filter(|b| b.ts < first.ts).collect();
                    let prepended = older.len();
                    let bars = {
                        let mut map = s.series.lock().expect("series lock");
                        let Some((g, cur)) = map.get_mut(&pane).filter(|(g, _)| *g == generation) else { return };
                        let _ = g;
                        let mut merged = older;
                        merged.extend_from_slice(cur);
                        *cur = Arc::new(merged);
                        cur.clone()
                    };
                    let gpu = build_gpu(bars.clone()).await;
                    s.send(Response::History {
                        pane,
                        generation,
                        symbol,
                        tf,
                        bars,
                        gpu,
                        prepended,
                        from_cache: false,
                        exhausted: prepended == 0,
                        live: false,
                    });
                }
                Err(e) => s.send(Response::HistoryError { pane, generation, error: format!("{e:#}") }),
            }
        }
        Request::Indicators { pane, generation, bars, tf, specs, palette } => {
            let built = tokio::task::spawn_blocking(move || build_indicators(&bars, tf, &specs, &palette))
                .await
                .unwrap_or_default();
            s.send(Response::Indicators { pane, generation, built });
        }
        Request::Compare { pane, generation, symbol, tf, base } => {
            let limit = base.len().max(100);
            match s.provider.history(&symbol, tf, None, limit).await {
                Ok(other) => {
                    let line = tokio::task::spawn_blocking(move || line_ref(&align_compare(&base, &other))).await;
                    if let Ok(line) = line {
                        s.send(Response::Compare { pane, generation, symbol, line });
                    }
                }
                Err(e) => s.send(Response::Status(format!("compare {symbol}: {e:#}"))),
            }
        }
        Request::Markers { pane, symbol } => {
            let sec = tr_data::security(&symbol);
            let trades = if sec.has_congress_trades() {
                s.db.lock().await.trades_for_ticker(&sec.ticker, 2_000).unwrap_or_default()
            } else {
                Vec::new()
            };
            s.send(Response::Markers { pane, symbol, trades });
        }
        Request::Quotes(symbols) => match s.provider.quotes(&symbols).await {
            Ok(q) => s.send(Response::Quotes(q)),
            Err(e) => s.send(Response::Status(format!("quotes: {e:#}"))),
        },
        Request::Latest { limit } => {
            let t = s.db.lock().await.latest_trades(limit).unwrap_or_default();
            s.send(Response::Latest(t));
        }
        Request::TickerTrades { pane, symbol } => {
            let ticker = Security::parse(&symbol).map(|x| x.ticker).unwrap_or_else(|| symbol.clone());
            let trades = s.db.lock().await.trades_for_ticker(&ticker, 1_000).unwrap_or_default();
            s.send(Response::TickerTrades { pane, symbol, trades });
        }
        Request::Profile { pane, query } => {
            let profile = profile(s, &query).await;
            s.send(Response::Profile { pane, profile: profile.map(Box::new), query });
        }
        Request::SearchPoliticians(q) => {
            let r = s.db.lock().await.search_politicians(&q, 500).unwrap_or_default();
            s.send(Response::Politicians(r));
        }
        Request::Track { query, on } => {
            let db = s.db.lock().await;
            let hit = resolve_politician(&db, &query);
            match hit {
                Some(p) => {
                    let _ = db.set_tracked(p.id, on);
                    let verb = if on { "Tracking" } else { "Stopped tracking" };
                    s.send(Response::Status(format!("{verb} {} — alerts fire when they file a new PTR", p.name)));
                }
                None => s.send(Response::Status(format!("No politician matches '{query}' yet (run REFRESH)"))),
            }
            let r = db.search_politicians("", 500).unwrap_or_default();
            s.send(Response::Politicians(r));
        }
        Request::Alerts => {
            let a = s.db.lock().await.alerts(false, 200).unwrap_or_default();
            s.send(Response::Alerts(a));
        }
        Request::MarkAlertsSeen => {
            let db = s.db.lock().await;
            let _ = db.mark_alerts_seen();
            s.send(Response::Alerts(db.alerts(false, 200).unwrap_or_default()));
        }
        Request::RefreshCongress => congress_refresh(s).await,
        Request::Forget(pane) => {
            s.series.lock().expect("series lock").remove(&pane);
        }
        Request::YieldCurves => match s.provider.yield_curves().await {
            Ok(c) => s.send(Response::Curves(Box::new(c))),
            Err(e) => s.send(Response::Status(format!("yield curve: {e:#}"))),
        },
        Request::Portfolio(creds) => {
            let result = match creds {
                None => Ok(tr_broker::sample::portfolio()),
                Some(c) => match tr_broker::SnapTrade::new(c) {
                    Ok(client) => client.portfolio().await,
                    Err(e) => Err(e),
                },
            };
            s.send(Response::Portfolio(result.map(Box::new).map_err(|e| format!("{e:#}"))));
        }
        Request::BrokerRegister { creds, user_id } => {
            let result = match tr_broker::SnapTrade::new(creds) {
                Ok(client) => client.register_user(&user_id).await,
                Err(e) => Err(e),
            };
            s.send(Response::BrokerRegistered(result.map_err(|e| format!("{e:#}"))));
        }
        Request::BrokerPortal { creds, dark, reconnect } => {
            let result = match tr_broker::SnapTrade::new(creds) {
                Ok(client) => client.connection_portal_url(dark, reconnect.as_deref()).await,
                Err(e) => Err(e),
            };
            s.send(Response::BrokerPortal(result.map_err(|e| format!("{e:#}"))));
        }
        Request::BrokerDisconnect { creds, id } => {
            let result = match tr_broker::SnapTrade::new(creds) {
                Ok(client) => client.delete_connection(&id).await,
                Err(e) => Err(e),
            };
            s.send(Response::BrokerDisconnected(result.map(|()| id).map_err(|e| format!("{e:#}"))));
        }
        Request::Options { pane, symbol, expiration } => {
            let chain = s.provider.options(&symbol, expiration).await.map(Box::new).map_err(|e| format!("{e:#}"));
            s.send(Response::Options { pane, symbol, chain });
        }
        Request::News { pane, symbol } => {
            let items = s.provider.news(symbol.as_deref()).await.map_err(|e| format!("{e:#}"));
            s.send(Response::News { pane, symbol, items });
        }
        Request::OrderBook { pane, symbol, depth } => {
            let book = s.provider.order_book(&symbol, depth).await.map_err(|e| format!("{e:#}"));
            s.send(Response::OrderBook { pane, symbol, book });
        }
        Request::Search(query) => {
            let hits = s.provider.search(&query).await.unwrap_or_default();
            s.send(Response::SearchHits { query, hits });
        }
        Request::Trade(t) => s.send(Response::Trade(Box::new(trade(t).await))),
        // ---- sabr (wave 3) ----------------------------------------------------------
        Request::Series { entity_key, metric, window, as_of, generation } => {
            handle_series(s, generation, &entity_key, &metric, window, as_of).await
        }
        Request::Projection { entity_key, horizon, model, generation } => {
            handle_projection(s, &entity_key, horizon, model.as_deref(), generation).await
        }
        Request::Backtest { model } => handle_backtest(s, &model).await,
        Request::Rost { what } => s.send(Response::OutOfScope { what: format!("roster ({what})") }),
        Request::TrackEntity { entity_key, label } => {
            let was_new = match s.track.is_tracked(&entity_key) {
                Ok(false) => {
                    let _ = s.track.track(&entity_key, chrono::Utc::now().timestamp());
                    true
                }
                Ok(true) => false,
                Err(e) => {
                    s.send(Response::Unsupported { reason: format!("track store: {e:#}") });
                    return;
                }
            };
            s.send(Response::SabrTracked { entity_key, label, was_new });
            s.send(Response::SabrAlerts(s.track.alerts(false, 200).unwrap_or_default()));
        }
        Request::UntrackEntity { entity_key, label } => {
            let was_tracked = s.track.untrack(&entity_key).unwrap_or(false);
            s.send(Response::SabrUntracked { entity_key, label, was_tracked });
            s.send(Response::SabrAlerts(s.track.alerts(false, 200).unwrap_or_default()));
        }
        Request::SabrAlerts => s.send(Response::SabrAlerts(s.track.alerts(false, 200).unwrap_or_default())),
    }
}

// ---- sabr arms ---------------------------------------------------------------------------

/// Bundled per-role season log for the sample player (`p:660271:bat` / `p:660271:pit`).
fn bundled_log(entity_key: &str) -> Result<sabr_model::event::EventLog, String> {
    let (file, mode) = match entity_key {
        "p:660271:bat" => ("ohtani-2025-bat.csv", SavantMode::Bat),
        "p:660271:pit" => ("ohtani-2025-pit.csv", SavantMode::Pitch),
        _ => return Err(format!("no bundled data for {entity_key}")),
    };
    let raw = match std::fs::read_to_string(crate::command::sample_root().join(file)) {
        Ok(raw) => raw,
        Err(e) => return Err(format!("bundled sample unavailable: {e}")),
    };
    let rev = sabr_model::event::SourceRev {
        source: "savant",
        url: "bundled sample (committed fixture)".into(),
        fetched: 0,
        sha256: [0; 32],
    };
    savant::parse_log(&raw, mode, 660_271, rev).map_err(|e| format!("bundled sample: {e:?}"))
}

/// The metric requested by code, rejecting role mismatches and unsupported entities.
fn sought_metric(entity_key: &str, metric: &str) -> Result<&'static sabr_model::metric::Metric, String> {
    let Some(m) = registry::by_id(metric) else {
        return Err(format!("unknown metric '{metric}' (see the sabr registry)"));
    };
    let player = match entity_key.parse::<Entity>() {
        Ok(Entity::Player(p)) => p,
        _ => return Err(format!("'{entity_key}' is not a player entity")),
    };
    if player.role != m.role {
        return Err(format!("metric '{metric}' is a {} metric; {entity_key} is a {} entity", m.role, player.role));
    }
    Ok(m)
}

/// Default trailing window per unit when none was typed: 50 PA / 30 pitches / 10 games-ish
/// sensible starting points (spec 4.7 leaves the default open).
fn default_window(metric: &sabr_model::metric::Metric) -> sabr_model::window::Window {
    use sabr_model::window::Unit;
    let (unit, n) = match metric.unit {
        Unit::Pa => (Unit::Pa, 50),
        Unit::Bf => (Unit::Bf, 50),
        Unit::Ab => (Unit::Ab, 50),
        Unit::Bip => (Unit::Bip, 50),
        Unit::Pitch => (Unit::Pitch, 50),
        Unit::Game => (Unit::Game, 10),
    };
    sabr_model::window::Window::Events { unit, n }
}

async fn handle_series(
    s: &Arc<Shared>,
    generation: u64,
    entity_key: &str,
    metric: &str,
    window: Option<sabr_model::window::Window>,
    as_of: Option<Date>,
) {
    let unsupported = |reason: String| Response::Unsupported { reason };
    // A bare `CAST` (no typed metric) defaults to the role's headline rate.
    let metric: String = if metric.is_empty() {
        match entity_key.rsplit(':').next() {
            Some("pit") => "pit:velocity".into(),
            _ => "bat:woba".into(),
        }
    } else {
        metric.to_owned()
    };
    let m = match sought_metric(entity_key, &metric) {
        Ok(m) => m,
        Err(reason) => return s.send(unsupported(reason)),
    };
    let window = window.unwrap_or_else(|| default_window(m));
    let log = match bundled_log(entity_key) {
        Ok(log) => log,
        Err(reason) => {
            return s.send(unsupported(reason));
        }
    };
    let as_of = as_of.unwrap_or_else(chrono_today);
    let points = tokio::task::spawn_blocking(move || sabr_model::series::derive(&[log], m, window, as_of)).await;
    match points {
        Ok(series) => {
            if series.points.is_empty() {
                s.send(unsupported(format!("series is empty for {entity_key} {metric} over {window:?}")));
            } else {
                s.send(Response::SabrSeries {
                    generation,
                    entity_key: entity_key.to_owned(),
                    metric: metric.to_owned(),
                    points: Arc::new(series.points),
                });
            }
        }
        Err(e) => s.send(unsupported(format!("derive failed: {e}"))),
    }
}

async fn handle_projection(
    s: &Arc<Shared>,
    entity_key: &str,
    horizon: crate::command::Horizon,
    model: Option<&str>,
    generation: u64,
) {
    let unsupported = |reason: String| Response::Unsupported { reason };
    let Some(mlbam) = entity_id_of(entity_key) else {
        return s.send(unsupported(format!("'{entity_key}' is not a player entity")));
    };
    let entity_key = entity_key.to_owned();
    let entity_key_for_reason = entity_key.clone();
    let model_name = model.unwrap_or("MARCEL").to_ascii_uppercase();
    // Only the bundled batter has a projection path in this wave; the Marcel pitching model is
    // a separate ticket.
    let job = tokio::task::spawn_blocking(
        move || -> Result<(String, sabr_forecast::marcel::Forecast, SabrModelStamp), String> {
            let root = crate::command::sample_root().join("lahman");
            let corpus = sabr_forecast::Corpus::from_lahman(&root).map_err(|e| format!("bundled corpus: {e}"))?;
            let player_id = match crate::command::catalog() {
                Some(cat) => cat.by_mlbam(mlbam).and_then(|r| r.key_bbref.clone()),
                None => None,
            };
            let player_id = match player_id {
                Some(id) if corpus.batting.iter().any(|l| l.player_id == id) => id,
                _ => return Err(format!("no bundled corpus season line for {entity_key_for_reason}")),
            };
            if model_name != "MARCEL" {
                return Err(format!("unknown model '{model_name}' (want MARCEL)"));
            }
            let constants = sabr_forecast::constants::Constants::prototype();
            let year: i32 = chrono::Datelike::year(&chrono::Utc::now());
            let last = corpus.batting.iter().map(|l| l.season).max().unwrap_or(0);
            let (target, as_of_season) = match horizon {
                // Rest-of-season: target the open (calendar) season; behind the corpus this
                // degrades to the same fold shape as next-season.
                crate::command::Horizon::RoS => {
                    let now = u16::try_from(year).unwrap_or(0);
                    (now.max(last), now.max(last) - 1)
                }
                crate::command::Horizon::Next => (last + 1, last),
            };
            let partial = (u16::try_from(year).is_ok_and(|y| y == target))
                .then(|| corpus.batting.iter().find(|l| l.player_id == player_id && l.season == target).cloned())
                .flatten();
            let forecast =
                sabr_forecast::marcel::project_marcel(&corpus, &constants, target, as_of_season, partial.as_ref())
                    .remove(&player_id)
                    .ok_or_else(|| format!("no projection for {player_id} in {target}"))?;
            let stamp = SabrModelStamp {
                id: "marcel.batting".into(),
                version: "1.0.0".into(),
                train_window: "3 prior seasons, weights 5/4/3".into(),
                as_of: chrono_today(),
                backtest_score: None,
            };
            Ok((player_id, forecast, stamp))
        },
    )
    .await;
    match job {
        Err(e) => s.send(unsupported(format!("projection: {e}"))),
        Ok(Err(e)) => s.send(unsupported(e)),
        Ok(Ok((_player_id, forecast, mut stamp))) => {
            stamp.backtest_score = s.btst_scores.lock().expect("btst lock").get("marcel.batting").copied();
            let quantiles = artifact_bands(s.artifacts.as_ref(), &forecast);
            s.send(Response::SabrProjection {
                generation,
                entity_key: entity_key.to_owned(),
                stamp: Box::new(stamp),
                woba: Some(forecast.woba),
                pa: Some(forecast.pa),
                reliability: Some(forecast.reliability),
                quantiles: quantiles.map(Box::new),
            });
        }
    }
}

async fn handle_backtest(s: &Arc<Shared>, model: &str) {
    // Only Marcel (batting) is fit-able in this wave; the baseline shares the harness but not
    // the scoring entry point used here.
    let model = model.trim().to_ascii_uppercase();
    if model != "MARCEL" {
        return s.send(Response::Unsupported { reason: format!("unknown backtest model '{model}' (want MARCEL)") });
    }
    let result = tokio::task::spawn_blocking(move || {
        let root = crate::command::sample_root().join("lahman");
        let corpus = sabr_forecast::Corpus::from_lahman(&root).map_err(|e| format!("bundled corpus: {e}"))?;
        let (folds, report) = sabr_forecast::harness::walk_forward(
            &sabr_forecast::marcel::Marcel,
            &corpus,
            &sabr_forecast::Protocol::v1(),
            &sabr_forecast::constants::Constants::prototype(),
        );
        let seasons = folds
            .iter()
            .map(|f| SabrSeasonScore {
                season: f.season,
                mae: f.score.mae,
                pa_mae: f.score.pa_mae,
                gate_pass: f.score.gate_pass,
            })
            .collect();
        Ok::<_, String>((seasons, report.pooled_mae, report.gate.pass))
    })
    .await;
    match result {
        Err(e) => s.send(Response::Unsupported { reason: format!("backtest task: {e}") }),
        Ok(Err(e)) => s.send(Response::Unsupported { reason: format!("backtest: {e}") }),
        Ok(Ok((seasons, pooled_mae, gate_pass))) => {
            s.btst_scores.lock().expect("btst lock").insert("marcel.batting".into(), pooled_mae);
            s.send(Response::SabrBacktest { model: "MARCEL".into(), seasons, pooled_mae, gate_pass });
        }
    }
}

/// De-standardise the latest artifact's interval model for one forecast (DR-3.40 bands).
/// `None` when no artifact exists yet or the model is in warm-up (DR-3.21a).
fn artifact_bands(
    index: Option<&sabr_forecast::artifact::ArtifactIndex>,
    forecast: &sabr_forecast::marcel::Forecast,
) -> Option<SabrQuantiles> {
    let row = index?.latest("marcel.batting").ok()??;
    let artifact = std::fs::read_to_string(&row.path).ok()?;
    let artifact: sabr_forecast::artifact::Artifact = serde_json::from_str(&artifact).ok()?;
    let model = artifact.intervals?;
    let q = model.quantiles(forecast.bucket)?;
    let d = sabr_forecast::intervals::IntervalModel::destandardise(q, forecast.pa.max(1.0));
    Some(SabrQuantiles {
        q05: forecast.woba + d.p05,
        q10: forecast.woba + d.p10,
        q25: forecast.woba + d.p25,
        q50: forecast.woba + d.p50,
        q75: forecast.woba + d.p75,
        q90: forecast.woba + d.p90,
        q95: forecast.woba + d.p95,
    })
}

/// MLBAM id of a canonical player key (`p:660271:bat`).
fn entity_id_of(entity_key: &str) -> Option<u32> {
    match entity_key.parse::<Entity>() {
        Ok(Entity::Player(p)) => Some(p.mlbam),
        _ => None,
    }
}

/// Today's UTC date (a live clock always yields a genuine calendar date).
fn chrono_today() -> time::Date {
    use chrono::Datelike;
    let now = chrono::Utc::now().date_naive();
    time::Date::from_calendar_date(now.year(), time::Month::try_from(now.month() as u8).unwrap(), now.day() as u8)
        .expect("today is a genuine calendar date")
}

async fn trade(t: TradeRequest) -> TradeResponse {
    let err = |e: anyhow::Error| format!("{e:#}");
    let client = |creds| tr_broker::SnapTrade::new(creds).map_err(err);
    match t {
        TradeRequest::Preview { creds, req, ticket } => {
            let result = match client(creds) {
                Ok(c) => c.order_impact(&req).await.map(Box::new).map_err(err),
                Err(e) => Err(e),
            };
            TradeResponse::Preview { ticket, result }
        }
        TradeRequest::Place { creds, preview, ticket } => {
            let result = match client(creds) {
                Ok(c) => c.place_previewed(&preview).await.map(|o| o.map(Box::new)),
                Err(e) => Err(tr_broker::trading::PlaceError::NotSent(e)),
            };
            TradeResponse::Placed { ticket, preview, result }
        }
        TradeRequest::Cancel { creds, account, order } => {
            let result = match client(creds) {
                Ok(c) => c.cancel_order(&account, &order).await.map_err(err),
                Err(e) => Err(e),
            };
            TradeResponse::Cancelled { account, order, result }
        }
        TradeRequest::Portal { creds, dark } => TradeResponse::Portal(match client(creds) {
            Ok(c) => c.trading_portal_url(dark).await.map_err(err),
            Err(e) => Err(e),
        }),
        TradeRequest::Capabilities { creds } => TradeResponse::Capabilities(match client(creds) {
            Ok(c) => c.connection_capabilities().await.map_err(|e| {
                let throttle = tr_broker::snaptrade::throttle(&e);
                CapsError { message: err(e), throttled: throttle.is_some(), retry_after: throttle.flatten() }
            }),
            Err(message) => Err(CapsError { message, throttled: false, retry_after: None }),
        }),
    }
}

/// Register a load for `generation`; generations only move forward, since request
/// tasks run concurrently and may start out of order.
fn claim_series(s: &Shared, pane: PaneId, generation: u64) -> bool {
    let mut map = s.series.lock().expect("series lock");
    match map.get(&pane) {
        Some((g, _)) if *g > generation => false,
        Some((g, _)) if *g == generation => true,
        _ => {
            map.insert(pane, (generation, Arc::new(Vec::new())));
            true
        }
    }
}

/// Store freshly loaded bars for a pane if `generation` is still the newest.
fn store_series(s: &Shared, pane: PaneId, generation: u64, bars: Arc<Vec<Bar>>) -> bool {
    let mut map = s.series.lock().expect("series lock");
    match map.get(&pane) {
        Some((g, _)) if *g > generation => false,
        _ => {
            map.insert(pane, (generation, bars));
            true
        }
    }
}

fn current_series(s: &Shared, pane: PaneId, generation: u64) -> Option<Arc<Vec<Bar>>> {
    s.series.lock().expect("series lock").get(&pane).filter(|(g, _)| *g == generation).map(|(_, b)| b.clone())
}

/// Merge the provider's live feed (if any) into the pane's series, publishing at most
/// four updates a second. Stops when the pane reloads, closes or the feed ends.
fn start_live(s: Arc<Shared>, pane: PaneId, generation: u64, symbol: String, tf: Timeframe) {
    let Some(mut feed) = s.provider.live(&symbol, tf) else { return };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut dirty = false;
        loop {
            tokio::select! {
                bar = feed.recv() => {
                    let Some(bar) = bar else { break };
                    let mut map = s.series.lock().expect("series lock");
                    match map.get_mut(&pane) {
                        Some((g, bars)) if *g == generation => dirty |= Series::apply_live(Arc::make_mut(bars), bar),
                        _ => break,
                    }
                }
                _ = tick.tick() => {
                    if !dirty {
                        if current_series(&s, pane, generation).is_none() {
                            break;
                        }
                        continue;
                    }
                    dirty = false;
                    let Some(bars) = current_series(&s, pane, generation) else { break };
                    let gpu = build_gpu(bars.clone()).await;
                    s.send(Response::History {
                        pane,
                        generation,
                        symbol: symbol.clone(),
                        tf,
                        bars,
                        gpu,
                        prepended: 0,
                        from_cache: false,
                        exhausted: false,
                        live: true,
                    });
                }
            }
        }
        tracing::debug!("live feed for pane {pane} ({symbol}) stopped");
    });
}

async fn build_gpu(bars: Arc<Vec<Bar>>) -> SeriesGpu {
    tokio::task::spawn_blocking(move || SeriesGpu::build(&bars)).await.expect("pyramid build task")
}

fn resolve_politician(db: &Db, query: &str) -> Option<Politician> {
    if let Ok(id) = query.trim().parse::<i64>() {
        return db.politician(id).ok().flatten();
    }
    // Prefer an exact last-name hit, then substring matches by activity.
    let all = db.search_politicians(query.trim(), 50).ok()?;
    let q = query.trim().to_ascii_lowercase();
    all.iter()
        .find(|p| p.politician.name.to_ascii_lowercase().split_whitespace().last() == Some(q.as_str()))
        .or(all.first())
        .map(|p| p.politician.clone())
}

async fn profile(s: &Shared, query: &str) -> Option<Profile> {
    let (politician, trades, tracked) = {
        let db = s.db.lock().await;
        let p = resolve_politician(&db, query)?;
        let trades = db.trades_for_politician(p.id).ok()?;
        let tracked = db.is_tracked(p.id).unwrap_or(false);
        (p, trades, tracked)
    };
    // Daily closes for each traded ticker (bounded) to estimate performance.
    let mut tickers: Vec<String> = trades.iter().filter_map(|t| t.ticker.clone()).collect();
    tickers.sort();
    tickers.dedup();
    tickers.truncate(40);
    let oldest = trades.iter().map(|t| t.tx_date).min();
    let days = oldest.map(|d| (chrono::Utc::now().date_naive() - d).num_days().max(30) as usize + 10).unwrap_or(400);
    let mut prices: std::collections::HashMap<String, Vec<Bar>> = std::collections::HashMap::new();
    let provider = s.provider.clone();
    let fetches = tickers.into_iter().map(|t| {
        let provider = provider.clone();
        async move {
            let r = provider.history(&format!("{t} US Equity"), Timeframe::D1, None, days.min(4000)).await;
            (t, r)
        }
    });
    for (t, r) in futures_join(fetches).await {
        if let Ok(b) = r {
            prices.insert(t, b);
        }
    }
    let (perf, returns) = performance::estimate(
        &trades,
        |tk, date| {
            let bars = prices.get(tk)?;
            let ts = date.and_hms_opt(23, 59, 59)?.and_utc().timestamp();
            let i = bars.partition_point(|b| b.ts <= ts).checked_sub(1)?;
            Some(bars[i].close)
        },
        |tk| prices.get(tk).and_then(|b| b.last()).map(|b| b.close),
    );
    Some(Profile { politician, trades, perf, returns, tracked })
}

/// Run futures concurrently, at most 6 in flight (keeps data providers happy).
async fn futures_join<F, T>(futs: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T>,
{
    use futures::StreamExt;
    futures::stream::iter(futs).buffered(6).collect().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_alignment_carries_last_close() {
        let base: Vec<Bar> = (0..5).map(|i| Bar::new(i * 10, 100.0, 100.0, 100.0, 100.0 + i as f64, 1.0)).collect();
        let other: Vec<Bar> = vec![Bar::new(5, 0.0, 0.0, 0.0, 50.0, 1.0), Bar::new(25, 0.0, 0.0, 0.0, 55.0, 1.0)];
        let a = align_compare(&base, &other);
        assert!(a[0].is_nan());
        assert_eq!(&a[1..], &[50.0, 50.0, 55.0, 55.0]);
    }

    #[test]
    fn indicators_build_expected_overlays() {
        let bars = tr_core::sample::generate("X", Timeframe::D1, 300, 1_790_000_000);
        let palette = crate::themes::terminal_light().palette(theme::DEFAULT_ACCENT);
        let specs: Vec<IndicatorSpec> =
            ["BB", "MACD", "VWAP", "PSAR", "PIVOT"].iter().map(|c| IndicatorSpec::new(c, &[]).unwrap()).collect();
        let built = build_indicators(&bars, Timeframe::D1, &specs, &palette);
        assert_eq!(built[0].overlays.len(), 3);
        // The histogram draws first, under the MACD and signal lines.
        assert_eq!(built[1].overlays[0].1.kind, LineKind::Hist);
        assert_ne!(built[1].overlays[2].1.color, built[1].overlays[1].1.color, "signal stands out from MACD");
        assert_eq!(built[2].overlays.len(), 1);
        assert!(built[3].overlays.iter().all(|(_, o)| o.kind == LineKind::Dots));
        assert_eq!(built[3].overlays[0].1.color, theme::rgba(palette.up));
        // Every pair of bars holding a marker keeps one when zoomed out.
        let up = &built[3].overlays[0].1.line.pyramid;
        let pairs = up.data[..up.offsets[1]].chunks(2).filter(|c| c.iter().any(|p| p[1].is_finite())).count();
        assert_eq!(up.data[up.offsets[1]..up.offsets[2]].iter().filter(|p| p[1].is_finite()).count(), pairs);
        assert!(built[4].overlays.iter().all(|(_, o)| o.kind == LineKind::Steps));
    }

    #[test]
    fn series_generations_only_move_forward() {
        let (out, _rx) = mpsc::channel();
        let s = Shared {
            provider: std::sync::Arc::new(tr_data::sample::SampleProvider::at(0, 10)),
            series: Default::default(),
            cache: std::sync::Mutex::new(None),
            db: Mutex::new(Db::open_in_memory().unwrap()),
            track: TrackStore::open_in_memory().unwrap(),
            artifacts: None,
            btst_scores: std::sync::Mutex::new(HashMap::new()),
            out,
            ctx: egui::Context::default(),
        };
        // The newer load's task starts first; the older one must not take over.
        assert!(claim_series(&s, 1, 2));
        assert!(!claim_series(&s, 1, 1));
        assert!(!store_series(&s, 1, 1, Arc::new(vec![])));
        assert!(store_series(&s, 1, 2, Arc::new(vec![Bar::new(0, 1.0, 1.0, 1.0, 1.0, 1.0)])));
        assert_eq!(current_series(&s, 1, 2).unwrap().len(), 1);
        assert!(current_series(&s, 1, 1).is_none());
    }

    #[test]
    fn join_all_runs_everything() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let out = rt.block_on(futures_join((0..20).map(|i| async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            i * 2
        })));
        assert_eq!(out, (0..20).map(|i| i * 2).collect::<Vec<_>>());
    }

    fn test_shared() -> (Arc<Shared>, mpsc::Receiver<Response>) {
        let (out, rx) = mpsc::channel();
        let s = Shared {
            provider: std::sync::Arc::new(tr_data::sample::SampleProvider::at(0, 10)),
            series: Default::default(),
            cache: std::sync::Mutex::new(None),
            db: Mutex::new(Db::open_in_memory().unwrap()),
            track: TrackStore::open_in_memory().unwrap(),
            artifacts: None,
            btst_scores: std::sync::Mutex::new(HashMap::new()),
            out,
            ctx: egui::Context::default(),
        };
        (Arc::new(s), rx)
    }

    fn drain(rx: &mpsc::Receiver<Response>) -> Vec<Response> {
        let mut out = Vec::new();
        while let Ok(r) = rx.try_recv() {
            out.push(r);
        }
        out
    }

    /// The bundled Ohtani yields a real derived series end to end; any other entity is an
    /// explicit `Unsupported` (ticketed, not guessed).
    #[test]
    fn series_dispatch_bundled_vs_unsupported() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (s, rx) = test_shared();
            handle(
                &s,
                Request::Series {
                    entity_key: "p:660271:bat".into(),
                    metric: "bat:woba".into(),
                    window: None,
                    as_of: None,
                    generation: 1,
                },
            )
            .await;
            let msgs = drain(&rx);
            let Some(Response::SabrSeries { points, entity_key, metric, .. }) = msgs.first() else {
                panic!("expected a Series response, got something else")
            };
            assert_eq!(entity_key, "p:660271:bat");
            assert_eq!(metric, "bat:woba");
            assert!(!points.is_empty());
            assert!(points.iter().any(|p| p.value.is_some()), "at least one defined value");
            // A bare `CAST` defaults to the role's headline metric.
            handle(
                &s,
                Request::Series {
                    entity_key: "p:660271:bat".into(),
                    metric: String::new(),
                    window: None,
                    as_of: None,
                    generation: 2,
                },
            )
            .await;
            assert!(
                matches!(drain(&rx).first(), Some(Response::SabrSeries { metric, points, .. }) if metric == "bat:woba" && !points.is_empty()),
                "bare CAST defaults to bat:woba"
            );
            // A non-bundled player and a role mismatch both answer `Unsupported`.
            handle(
                &s,
                Request::Series {
                    entity_key: "p:123456:bat".into(),
                    metric: "bat:woba".into(),
                    window: None,
                    as_of: None,
                    generation: 1,
                },
            )
            .await;
            handle(
                &s,
                Request::Series {
                    entity_key: "p:660271:pit".into(),
                    metric: "bat:woba".into(),
                    window: None,
                    as_of: None,
                    generation: 1,
                },
            )
            .await;
            let unsupported: Vec<String> = drain(&rx)
                .into_iter()
                .filter_map(|r| match r {
                    Response::Unsupported { reason } => Some(reason),
                    _ => None,
                })
                .collect();
            assert_eq!(unsupported.len(), 2, "{unsupported:?}");
        });
    }

    /// The bundled pitcher series works too, and the Ohtani projection runs the real Marcel
    /// over the bundled corpus with a registry stamp.
    #[test]
    fn projection_dispatch_marcel_with_stamp() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (s, rx) = test_shared();
            handle(
                &s,
                Request::Series {
                    entity_key: "p:660271:pit".into(),
                    metric: "pit:velocity".into(),
                    window: None,
                    as_of: None,
                    generation: 1,
                },
            )
            .await;
            assert!(matches!(drain(&rx).first(), Some(Response::SabrSeries { .. })), "pitcher series");
            handle(
                &s,
                Request::Projection {
                    entity_key: "p:660271:bat".into(),
                    horizon: crate::command::Horizon::Next,
                    model: Some("MARCEL".into()),
                    generation: 1,
                },
            )
            .await;
            let msgs = drain(&rx);
            let Some(Response::SabrProjection { stamp, woba, pa, reliability, .. }) = msgs.first() else {
                panic!("expected a Projection response, got something else")
            };
            assert_eq!(stamp.id, "marcel.batting");
            assert_eq!(stamp.version, "1.0.0");
            assert!(!stamp.train_window.is_empty());
            assert_eq!(stamp.as_of, chrono_today());
            let (woba, pa, reliability) =
                (woba.unwrap_or_default(), pa.unwrap_or_default(), reliability.unwrap_or(-1.0));
            assert!(woba > 0.2 && woba < 0.6, "marcel wOBA {woba}");
            assert!(pa > 50.0, "marcel PA {pa}");
            assert!((0.0..=1.0).contains(&reliability), "reliability {reliability}");
        });
    }

    /// `NYM ROST` answers the documented OutOfScope stub.
    #[test]
    fn rost_dispatch_is_out_of_scope() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (s, rx) = test_shared();
            handle(&s, Request::Rost { what: "NYM".into() }).await;
            assert!(matches!(drain(&rx).first(), Some(Response::OutOfScope { .. })));
        });
    }

    /// Track/untrack round trip with alerts on the worker's track store.
    #[test]
    fn track_dispatch_round_trip() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (s, rx) = test_shared();
            handle(&s, Request::TrackEntity { entity_key: "p:660271:bat".into(), label: "Shohei Ohtani".into() }).await;
            let msgs = drain(&rx);
            assert!(
                matches!(msgs.first(), Some(Response::SabrTracked { was_new: true, label, .. }) if label == "Shohei Ohtani"),
                "first track is new"
            );
            assert!(matches!(msgs.get(1), Some(Response::SabrAlerts(_))), "alerts refreshed after track");
            handle(&s, Request::TrackEntity { entity_key: "p:660271:bat".into(), label: "Shohei Ohtani".into() }).await;
            assert!(matches!(drain(&rx).first(), Some(Response::SabrTracked { was_new: false, .. })), "re-track idempotent");
            assert!(s.track.is_tracked("p:660271:bat").unwrap());
            assert_eq!(s.track.tracked().unwrap().len(), 1);
            handle(&s, Request::UntrackEntity { entity_key: "p:660271:bat".into(), label: "Shohei Ohtani".into() }).await;
            assert!(matches!(drain(&rx).first(), Some(Response::SabrUntracked { was_tracked: true, .. })));
            assert!(!s.track.is_tracked("p:660271:bat").unwrap());
        });
    }
}
