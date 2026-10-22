//! Top-level terminal: command line, docked panes, saved layouts, hotkeys and status bar.
//!
//! Submodules: `chrome` (toolbar, sidebar, status bar), `workspace` (dock rendering,
//! drag and drop, pop-out windows, link groups, saved layouts), `panels` (per-kind pane
//! content) and `settings_view` (preferences UI).

mod broker;
mod chrome;
mod demo;
mod hotkeys_view;
mod options_view;
mod palette_view;
mod panels;
mod portfolio_view;
mod settings_view;
mod theming;
mod trading;
mod workspace;

pub use broker::PosSort;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{Color32, Event, Key, Modifiers, Rect, Ui, UiBuilder, pos2, vec2};
use serde::{Deserialize, Serialize};
use tr_chart::drawings::Tool;
use tr_congress::Trade;
use tr_congress::db::{Alert, PoliticianSummary};
use tr_data::{NewsItem, OrderBook, Quote, SearchHit, YieldCurves};

use crate::anim;
use crate::chart_view::ChartState;
use crate::command::{self, Command, CommandOutcome, Function};
use crate::congress_view::{Action, FeedFilter};
use crate::dock::{Node, PaneId};
use crate::layout::Layout;
use crate::layouts::{LayoutDoc, Link, PaneSettings, Store, WindowGeometry};
use crate::markets_view;
use crate::prefs::{self, Prefs};
use crate::theme;
use crate::worker::{Profile, Request, Response, Worker, data_dir};

use chrome::{SIDEBAR_W, STATUS_H, SidebarCmd};
use settings_view::SettingsTab;

pub enum PaneKind {
    Chart(Box<ChartState>),
    Feed {
        filter: FeedFilter,
    },
    Ticker {
        symbol: String,
        trades: Vec<Trade>,
        loading: bool,
    },
    Profile {
        query: String,
        data: Option<Box<Profile>>,
        loading: bool,
    },
    Politicians,
    Alerts,
    Watchlist,
    Help,
    Wei,
    Curve,
    Search {
        query: String,
    },
    News(Box<NewsPane>),
    OrderBook(Box<BookPane>),
    Options(Box<OptionsPane>),
    /// Brokerage accounts through SnapTrade (`PORT`, `ACCT`, `POS`, `ORD`, `ACTV`).
    Portfolio(Box<broker::PortfolioPane>),
    Settings,
    /// Ranked catalog matches for an ambiguous entity token (`DIAZ CAST`): ephemeral, never
    /// saved to a layout. Accepting rewrites the token to a direct entity key and re-executes.
    EntityPick(Box<EntityPickPane>),
}

/// The disambiguation pane: ranked `CatalogHit` rows plus the rewriting context.
pub struct EntityPickPane {
    /// The original command line that resolved ambiguously.
    pub command: String,
    /// The ambiguous entity token inside `command`, textually replaced on accept.
    pub token: String,
    /// Ranked catalog hits, most prominent (longest span) first.
    pub hits: Vec<command::CatalogHit>,
    /// Keyboard/click selection into `hits`.
    pub selected: usize,
    /// Accepts used so far on the rewrite chain (capped, see `PICK_ROUNDS`).
    pub rounds: u8,
}

/// Rewrites allowed when a rewritten command resolves to another picker (defensive cap; a
/// rewritten token is a direct key, so the branch should never fire in practice).
const PICK_ROUNDS: u8 = 3;

pub struct OptionsPane {
    pub symbol: String,
    /// Selected expiration (`None` = nearest).
    pub expiration: Option<i64>,
    pub chain: Option<Box<tr_data::OptionChain>>,
    /// Greeks per call and put, aligned with the chain's contracts.
    pub call_greeks: Vec<Option<tr_core::options::Greeks>>,
    pub put_greeks: Vec<Option<tr_core::options::Greeks>>,
    /// Risk-free rate used for the greeks.
    pub rate: f64,
    pub error: Option<String>,
    pub loading: bool,
    pub fetched: Option<Instant>,
    /// Strikes each side of the money (0 = all).
    pub strikes: usize,
    pub show_greeks: bool,
}

impl OptionsPane {
    pub fn new(symbol: &str, strikes: usize, show_greeks: bool) -> Self {
        Self {
            symbol: symbol.into(),
            expiration: None,
            chain: None,
            call_greeks: Vec::new(),
            put_greeks: Vec::new(),
            rate: 0.04,
            error: None,
            loading: true,
            fetched: None,
            strikes,
            show_greeks,
        }
    }

    /// Store a chain and derive greeks from each contract's implied volatility.
    pub fn set_chain(&mut self, chain: Box<tr_data::OptionChain>, rate: f64, now: i64) {
        // Expiry at the 16:00 New York close (≈ 20:00 UTC) of the expiration date.
        let t = ((chain.expiration + 20 * 3600 - now) as f64 / (365.0 * 86_400.0)).max(0.5 / 365.0);
        let g = |call: bool, c: &tr_data::OptionContract| {
            tr_core::options::black_scholes(call, chain.spot, c.strike, t, rate, chain.dividend_yield, c.iv?)
                .map(|(_, g)| g)
        };
        self.call_greeks = chain.calls.iter().map(|c| g(true, c)).collect();
        self.put_greeks = chain.puts.iter().map(|c| g(false, c)).collect();
        self.rate = rate;
        if self.expiration.is_none() || !chain.expirations.contains(&chain.expiration) {
            self.expiration = Some(chain.expiration);
        }
        self.chain = Some(chain);
        self.error = None;
        self.loading = false;
    }
}

pub struct NewsPane {
    /// `None` = general market headlines.
    pub symbol: Option<String>,
    pub items: Vec<NewsItem>,
    pub error: Option<String>,
    pub loading: bool,
    pub fetched: Option<Instant>,
}

impl NewsPane {
    pub fn new(symbol: Option<String>) -> Self {
        Self { symbol, items: Vec::new(), error: None, loading: true, fetched: None }
    }
}

pub struct BookPane {
    pub symbol: String,
    pub depth: usize,
    /// Price bucket; 0 = raw levels.
    pub group: f64,
    pub book: Option<OrderBook>,
    pub error: Option<String>,
    pub fetched: Option<Instant>,
}

impl BookPane {
    pub fn new(symbol: &str, depth: usize, group: f64) -> Self {
        Self { symbol: symbol.into(), depth, group, book: None, error: None, fetched: None }
    }
}

impl PaneKind {
    /// Short tab title.
    fn tab_title(&self) -> String {
        let short = |s: &str| tr_data::security(s).ticker;
        match self {
            PaneKind::Chart(c) => short(&c.symbol),
            PaneKind::Feed { .. } => "Congress".into(),
            PaneKind::Ticker { symbol, .. } => format!("{} Congress", short(symbol)),
            PaneKind::Profile { query, data, .. } => {
                data.as_ref().map_or_else(|| query.clone(), |p| p.politician.name.clone())
            }
            PaneKind::Politicians => "Politicians".into(),
            PaneKind::Alerts => "Alerts".into(),
            PaneKind::Watchlist => "Watchlist".into(),
            PaneKind::Help => "Help".into(),
            PaneKind::Wei => "World Markets".into(),
            PaneKind::Curve => "Yield Curve".into(),
            PaneKind::Search { query } if query.is_empty() => "Search".into(),
            PaneKind::Search { query } => format!("Search: {query}"),
            PaneKind::News(n) => match &n.symbol {
                Some(s) => format!("{} News", short(s)),
                None => "Top News".into(),
            },
            PaneKind::OrderBook(b) => format!("{} Depth", short(&b.symbol)),
            PaneKind::Options(o) => format!("{} Options", short(&o.symbol)),
            PaneKind::Portfolio(p) => p.title(),
            PaneKind::Settings => "Settings".into(),
            PaneKind::EntityPick(p) => format!("Pick: {}", p.token),
        }
    }

    /// The Bloomberg-style function code shown on the tab.
    fn code(&self) -> &'static str {
        match self {
            PaneKind::Chart(_) => "GP",
            PaneKind::Feed { .. } | PaneKind::Ticker { .. } => "CONG",
            PaneKind::Profile { .. } | PaneKind::Politicians => "POL",
            PaneKind::Alerts => "ALRT",
            PaneKind::Watchlist => "WL",
            PaneKind::Help => "HELP",
            PaneKind::Wei => "WEI",
            PaneKind::Curve => "GC",
            PaneKind::Search { .. } => "SECF",
            PaneKind::News(_) => "CN",
            PaneKind::OrderBook(_) => "OB",
            PaneKind::Options(_) => "OMON",
            PaneKind::Portfolio(p) => p.view.code(),
            PaneKind::Settings => "PDF",
            PaneKind::EntityPick(_) => "PICK",
        }
    }

    /// The security this pane follows, for symbol-driven panels.
    fn symbol(&self) -> Option<&str> {
        match self {
            PaneKind::Chart(c) => Some(&c.symbol),
            PaneKind::Ticker { symbol, .. } => Some(symbol),
            PaneKind::News(n) => n.symbol.as_deref(),
            PaneKind::OrderBook(b) => Some(&b.symbol),
            PaneKind::Options(o) => Some(&o.symbol),
            _ => None,
        }
    }

    fn follows_symbol(&self) -> bool {
        matches!(
            self,
            PaneKind::Chart(_)
                | PaneKind::Ticker { .. }
                | PaneKind::News(_)
                | PaneKind::OrderBook(_)
                | PaneKind::Options(_)
        )
    }

    /// Persisted settings for the pane; `None` for the ephemeral entity picker, which is
    /// never saved to a layout.
    fn settings(&self) -> Option<PaneSettings> {
        Some(match self {
            PaneKind::Chart(c) => PaneSettings::Chart {
                symbol: c.symbol.clone(),
                tf: c.tf,
                chart_type: c.chart_type,
                log: c.log,
                indicators: c.indicators.clone(),
                drawings: c.drawings.clone(),
                compare: c.compare.clone(),
                show_volume: c.show_volume,
                show_congress: c.show_congress,
                magnet: c.magnet,
            },
            PaneKind::Feed { filter } => {
                PaneSettings::Feed((*filter != FeedFilter::default()).then(|| (*filter).into()))
            }
            PaneKind::Ticker { symbol, .. } => PaneSettings::Ticker(symbol.clone()),
            PaneKind::Profile { query, .. } => PaneSettings::Profile(query.clone()),
            PaneKind::Politicians => PaneSettings::Politicians,
            PaneKind::Alerts => PaneSettings::Alerts,
            PaneKind::Watchlist => PaneSettings::Watchlist,
            PaneKind::Help => PaneSettings::Help,
            PaneKind::Wei => PaneSettings::Wei,
            PaneKind::Curve => PaneSettings::Curve,
            PaneKind::Search { query } => PaneSettings::Search(query.clone()),
            PaneKind::News(n) => PaneSettings::News(n.symbol.clone()),
            PaneKind::OrderBook(b) => {
                PaneSettings::OrderBook { symbol: b.symbol.clone(), depth: b.depth, group: b.group }
            }
            PaneKind::Settings => PaneSettings::Settings,
            PaneKind::Options(o) => {
                PaneSettings::Options { symbol: o.symbol.clone(), strikes: o.strikes, greeks: o.show_greeks }
            }
            PaneKind::Portfolio(p) => PaneSettings::Portfolio {
                view: p.view,
                account: p.account.clone(),
                symbol: p.symbol.clone(),
                sort: p.sort,
                open_only: p.open_only,
            },
            PaneKind::EntityPick(_) => return None,
        })
    }

    fn from_settings(p: PaneSettings) -> PaneKind {
        match p {
            PaneSettings::Chart {
                symbol,
                tf,
                chart_type,
                log,
                indicators,
                drawings,
                compare,
                show_volume,
                show_congress,
                magnet,
            } => {
                let mut c = ChartState::new(0, &symbol, tf);
                c.chart_type = chart_type;
                c.log = log;
                c.indicators = indicators;
                c.drawings = drawings;
                c.compare = compare;
                c.show_volume = show_volume;
                c.show_congress = show_congress;
                c.magnet = magnet;
                PaneKind::Chart(Box::new(c))
            }
            PaneSettings::Feed(f) => PaneKind::Feed { filter: f.map(Into::into).unwrap_or_default() },
            PaneSettings::Ticker(s) => PaneKind::Ticker { symbol: s, trades: Vec::new(), loading: true },
            PaneSettings::Profile(q) => PaneKind::Profile { query: q, data: None, loading: true },
            PaneSettings::Politicians => PaneKind::Politicians,
            PaneSettings::Alerts => PaneKind::Alerts,
            PaneSettings::Watchlist => PaneKind::Watchlist,
            PaneSettings::Help => PaneKind::Help,
            PaneSettings::Wei => PaneKind::Wei,
            PaneSettings::Curve => PaneKind::Curve,
            PaneSettings::Search(q) => PaneKind::Search { query: q },
            PaneSettings::News(s) => PaneKind::News(Box::new(NewsPane::new(s))),
            PaneSettings::OrderBook { symbol, depth, group } => {
                PaneKind::OrderBook(Box::new(BookPane::new(&symbol, depth, group)))
            }
            PaneSettings::Settings => PaneKind::Settings,
            PaneSettings::Options { symbol, strikes, greeks } => {
                PaneKind::Options(Box::new(OptionsPane::new(&symbol, strikes, greeks)))
            }
            PaneSettings::Portfolio { view, account, symbol, sort, open_only } => {
                PaneKind::Portfolio(Box::new(broker::PortfolioPane { view, account, symbol, sort, open_only }))
            }
        }
    }
}

pub struct Pane {
    pub id: PaneId,
    pub kind: PaneKind,
    pub link: Option<Link>,
    /// User-chosen tab title.
    pub title: Option<String>,
    /// Set while popped out into its own window.
    pub window: Option<WindowGeometry>,
}

impl Pane {
    fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.kind.tab_title())
    }
}

/// App-wide state persisted in `settings.json` (layouts live in their own files).
#[derive(Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    watchlist: Vec<String>,
    #[serde(default = "default_true")]
    sidebar: bool,
    /// Name of the layout to restore on launch.
    #[serde(default)]
    active_layout: Option<String>,
    // Pre-docking settings files stored a preset and a pane list here.
    #[serde(default, skip_serializing)]
    layout: Option<Layout>,
    #[serde(default, skip_serializing)]
    panes: Option<Vec<serde_json::Value>>,
}

fn default_true() -> bool {
    true
}

const DEFAULT_WATCHLIST: [&str; 11] = [
    "SPX Index",
    "AAPL US Equity",
    "NVDA US Equity",
    "SPY US Equity",
    "BTCUSD Curncy",
    "ETHUSD Curncy",
    "EURUSD Curncy",
    "USDJPY Curncy",
    "US10Y Govt",
    "CL1 Comdty",
    "GC1 Comdty",
];

/// Convert a pre-docking settings file into a layout document.
fn migrate_legacy(s: &Settings, defaults: &prefs::PanelDefaults) -> Option<LayoutDoc> {
    let panes: Vec<PaneSettings> = s
        .panes
        .as_ref()?
        .iter()
        .filter_map(|v| {
            // Unit variants were stored as bare strings ("Feed"); wrap them as {"Feed": null}.
            let v = match v {
                serde_json::Value::String(name) => serde_json::json!({ name.as_str(): null }),
                other => other.clone(),
            };
            serde_json::from_value(v).ok()
        })
        .collect();
    if panes.is_empty() {
        return None;
    }
    let _ = defaults;
    Some(LayoutDoc::preset("Default", s.layout.unwrap_or(Layout::Grid2x2), panes))
}

/// How long an alert toast stays up.
const TOAST_SECS: u64 = 10;

/// A pane drag from its tab or header: pending until the pointer moves past a small
/// threshold (so clicks still select tabs), then active until release.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub pane: PaneId,
    pub origin: egui::Pos2,
    pub active: bool,
}

pub struct TerminalApp {
    worker: Worker,
    provider: String,
    panes: Vec<Pane>,
    dock: Node,
    focus: PaneId,
    next_id: PaneId,
    last_opened: Option<PaneId>,
    /// Preset last applied, for the toolbar picker (cleared by manual rearranging).
    preset: Option<Layout>,
    drag: Option<Drag>,
    dock_motion: workspace::DockMotion,
    /// Tab being renamed inline: pane and edit buffer.
    renaming_tab: Option<(PaneId, String)>,

    store: Store,
    layout_name: String,
    layout_names: Vec<String>,
    last_saved: Vec<u8>,
    /// Layout being renamed in the sidebar: old name and edit buffer.
    renaming_layout: Option<(String, String)>,

    prefs: Prefs,
    /// Built-in and custom themes, and the theme last applied to egui.
    themes: Vec<crate::themes::Theme>,
    theme_store: crate::themes::ThemeStore,
    applied_theme: Option<crate::themes::Theme>,
    /// Theme selected in the editor.
    editing_theme: String,
    /// Custom themes edited but not yet written (saved once editing pauses).
    dirty_themes: Vec<String>,
    themes_changed_at: Option<Instant>,
    renaming_theme: Option<(String, String)>,
    theme_io: String,
    /// The OS appearance, for System theme resolution in commands.
    system_theme: Option<egui::Theme>,
    /// The active layout carries its theme.
    layout_theme: bool,
    rebinding: Option<prefs::Action>,
    hotkey_filter: String,
    settings_tab: SettingsTab,
    io_path: String,

    curves: Option<Box<YieldCurves>>,
    last_wei: Option<Instant>,
    last_curve: Option<Instant>,
    wei_updated: Option<String>,
    /// Latest network search results and the query they answer.
    net_hits: (String, Vec<SearchHit>),
    /// Query typed in the command line awaiting a debounced network search.
    search_pending: Option<(String, Instant)>,
    suggestion: Option<usize>,

    cmd: String,
    cmd_history: Vec<String>,
    cmd_hist_pos: Option<usize>,
    focus_cmd: bool,

    status: String,
    toasts: Vec<(String, Instant)>,

    latest: Vec<Trade>,
    politicians: Vec<PoliticianSummary>,
    pol_search: String,
    alerts: Vec<Alert>,
    congress_status: String,
    /// When the last Congress ingest finished.
    congress_checked: Option<chrono::DateTime<chrono::Local>>,

    watchlist: Vec<String>,
    quotes: HashMap<String, Quote>,
    last_quotes: Option<Instant>,
    last_feed: Option<Instant>,
    last_save: Instant,
    started: Instant,
    smoke_done: bool,
    /// Frame-time readout (F12): smoothed CPU time of `ui()` and interval between frames.
    show_perf: bool,
    perf_cpu_ms: f64,
    perf_dt_ms: f64,
    last_frame: Option<Instant>,

    sidebar_open: bool,
    // ---- sabr (wave 3) -------------------------------------------------------------
    /// Request generation for sabr pane loads (cast/projection/backtest), monotonic.
    sabr_gen: u64,
    /// Latest derived series per entity, guarded by load generation (newest wins).
    sabr_series: HashMap<String, (u64, std::sync::Arc<Vec<sabr_model::series::Point>>, String)>,
    /// Tracked sabr entities' alerts (newest first).
    sabr_alerts: Vec<sabr_data::track::AlertRow>,
    /// Tracked entity keys.
    sabr_tracked: Vec<String>,
    /// Latest projection stamp per spec story 17 (`None` until a fit ran).
    sabr_projection: Option<Box<crate::worker::SabrProjectionRow>>,
    /// Latest backtest: (model, pooled wOBA MAE, scored seasons, gate pass).
    sabr_backtest: Option<(String, f64, usize, bool)>,
    /// System accent colour (macOS) or the default blue.
    system_accent: Color32,
    last_accent_poll: Option<Instant>,
    window_styled: bool,
    updater_started: bool,
    /// The window is see-through behind the sidebar (macOS vibrancy).
    vibrancy: bool,
    /// Measured macOS title bar geometry (polled; changes with full screen).
    chrome: Option<tr_macos::Chrome>,
    last_chrome_poll: Option<Instant>,
    /// Brokerage integration (SnapTrade credentials and the loaded portfolio).
    broker: broker::BrokerState,
    /// Order ticket and trading dialogs.
    trading: trading::TradingState,
    /// The paper-trading demo account (selected by default).
    demo: demo::DemoState,
    /// ⌘K command palette.
    palette: crate::palette::Palette,
}

/// The entity key a picked row resolves to: `p:<MLBAM id>:bat|pit`, where the role is the
/// `.BAT`/`.PIT` suffix of the original token when present, else the row's catalog role
/// evidence (a two-way player's bat identity mirrors `command::pick_role`).
fn entity_key(hit: &command::CatalogHit, original_token: &str) -> Option<String> {
    let mlbam = hit.mlbam?;
    let suffix_role = |s: &str| match s.to_ascii_uppercase().as_str() {
        "BAT" => Some("bat"),
        "PIT" => Some("pit"),
        _ => None,
    };
    let role = match original_token.rsplit_once('.').and_then(|(_, s)| suffix_role(s)) {
        Some(r) => r,
        None if hit.pit && !hit.bat => "pit",
        None => "bat",
    };
    Some(format!("p:{mlbam}:{role}"))
}

/// Convert a run of words (`BTCUSD CRNCY AAPL US EQUITY MSFT`) into market keys.
fn keys_from_words(words: &[String]) -> Vec<String> {
    let tokens: Vec<&str> = words.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tr_core::Security::resolve_tokens(&tokens[i..]) {
            Some((sec, used)) => {
                out.push(sec.key());
                i += used;
            }
            None => i += 1,
        }
    }
    out
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

fn prefs_path() -> std::path::PathBuf {
    data_dir().join("prefs.json")
}

impl TerminalApp {
    pub fn new(ctx: &egui::Context, worker: Worker, provider: String) -> Self {
        let system_accent =
            tr_macos::accent_color().map_or(theme::DEFAULT_ACCENT, |[r, g, b]| Color32::from_rgb(r, g, b));
        let mut prefs = Prefs::load(&prefs_path());
        let theme_store = crate::themes::ThemeStore::new(data_dir().join("themes"));
        let mut themes = theme_store.load_all();
        // Appearance overrides from older versions become a custom theme file.
        if let Some(mut t) = prefs.migrated_theme.take() {
            t.name = crate::themes::unique_name(&themes, &t.name);
            prefs.appearance.theme = t.name.clone();
            match theme_store.save(&t) {
                Ok(path) => t.source = Some(path),
                Err(e) => tracing::warn!("saving migrated theme: {e:#}"),
            }
            themes.push(t);
            let _ = prefs.save(&prefs_path());
        }
        if let Ok(v) = std::env::var("TR_THEME") {
            prefs.appearance.theme = match v.to_ascii_lowercase().as_str() {
                "light" => prefs.appearance.light_theme.clone(),
                "dark" => prefs.appearance.dark_theme.clone(),
                _ => crate::themes::find(&themes, &v).map_or(prefs.appearance.theme.clone(), |t| t.name.clone()),
            };
        }
        let settings: Option<Settings> =
            std::fs::read(settings_path()).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let broker = broker::BrokerState::load(provider.eq_ignore_ascii_case("sample"));
        let (demo, demo_note) = demo::DemoState::load(demo::demo_path());
        let mut app = Self {
            broker,
            trading: Default::default(),
            demo,
            worker,
            provider,
            panes: Vec::new(),
            dock: Node::default(),
            focus: 0,
            next_id: 1,
            last_opened: None,
            preset: None,
            drag: None,
            dock_motion: Default::default(),
            renaming_tab: None,
            store: Store::new(data_dir().join("layouts")),
            layout_name: "Default".into(),
            layout_names: Vec::new(),
            last_saved: Vec::new(),
            renaming_layout: None,
            editing_theme: prefs.appearance.theme.clone(),
            themes,
            theme_store,
            applied_theme: None,
            dirty_themes: Vec::new(),
            themes_changed_at: None,
            renaming_theme: None,
            theme_io: String::new(),
            system_theme: ctx.system_theme(),
            layout_theme: false,
            prefs,
            rebinding: None,
            hotkey_filter: String::new(),
            settings_tab: SettingsTab::Appearance,
            io_path: String::new(),
            curves: None,
            last_wei: None,
            last_curve: None,
            wei_updated: None,
            net_hits: (String::new(), Vec::new()),
            search_pending: None,
            suggestion: None,
            cmd: String::new(),
            cmd_history: Vec::new(),
            cmd_hist_pos: None,
            focus_cmd: true,
            status: "Ready. Type a ticker and press <GO> (Enter). F1 for help.".into(),
            toasts: Vec::new(),
            latest: Vec::new(),
            politicians: Vec::new(),
            pol_search: String::new(),
            alerts: Vec::new(),
            congress_status: String::new(),
            congress_checked: None,
            watchlist: DEFAULT_WATCHLIST.map(String::from).to_vec(),
            quotes: HashMap::new(),
            last_quotes: None,
            last_feed: None,
            last_save: Instant::now(),
            started: Instant::now(),
            smoke_done: false,
            show_perf: std::env::var_os("TR_FPS").is_some(),
            perf_cpu_ms: 0.0,
            perf_dt_ms: 0.0,
            last_frame: None,
            sidebar_open: true,
            sabr_gen: 0,
            sabr_series: HashMap::new(),
            sabr_alerts: Vec::new(),
            sabr_tracked: Vec::new(),
            sabr_projection: None,
            sabr_backtest: None,
            system_accent,
            last_accent_poll: None,
            window_styled: false,
            updater_started: false,
            vibrancy: false,
            chrome: None,
            last_chrome_poll: None,
            palette: crate::palette::Palette::load(&data_dir().join("palette.json")),
        };
        let mut doc = None;
        if let Some(s) = &settings {
            if !s.watchlist.is_empty() {
                // Older settings stored bare tickers; normalise to market keys.
                app.watchlist = s.watchlist.iter().map(|w| tr_data::security(w).key()).collect();
                app.watchlist.dedup();
            }
            app.sidebar_open = s.sidebar;
            if let Some(name) = &s.active_layout {
                match app.store.load(name) {
                    Ok(d) => doc = Some(d),
                    Err(e) => tracing::warn!("restoring layout {name:?}: {e:#}"),
                }
            }
            if doc.is_none() {
                doc = migrate_legacy(s, &app.prefs.panels);
            }
        }
        let doc = doc
            .or_else(|| app.store.list().first().and_then(|n| app.store.load(n).ok()))
            .unwrap_or_else(|| LayoutDoc::default_workspace("Default", &app.prefs.panels));
        app.apply_doc(doc);
        if !app.store.exists(&app.layout_name) {
            app.save_layout();
        }
        app.refresh_layout_names();
        app.worker.send(Request::Latest { limit: 2_000 });
        app.worker.send(Request::SearchPoliticians(String::new()));
        app.worker.send(Request::Alerts);
        if let Some(note) = demo_note {
            app.trade_toast(note, false);
        }
        app
    }

    fn save_settings(&self) {
        let s = Settings {
            watchlist: self.watchlist.clone(),
            sidebar: self.sidebar_open,
            active_layout: Some(self.layout_name.clone()),
            layout: None,
            panes: None,
        };
        if let Ok(json) = serde_json::to_vec_pretty(&s)
            && let Err(e) = crate::layouts::write_atomic(&settings_path(), &json)
        {
            tracing::warn!("saving settings: {e}");
        }
    }

    fn save_prefs(&mut self) {
        if let Err(e) = self.prefs.save(&prefs_path()) {
            self.status = format!("Could not save preferences: {e}");
        }
    }

    fn run_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::OpenChart(s) => self.open_security(&s),
                Action::OpenTicker(s) => {
                    self.open_panel(PaneKind::Ticker { symbol: s, trades: Vec::new(), loading: true })
                }
                Action::OpenProfile(q) => self.open_panel(PaneKind::Profile { query: q, data: None, loading: true }),
                Action::AddToWatchlist(s) => {
                    let key = tr_data::security(&s).key();
                    if !self.watchlist.contains(&key) {
                        self.status = format!("Added {key} to the watchlist");
                        self.watchlist.push(key);
                        self.last_quotes = None;
                    }
                }
                Action::LinkedSymbol(link, s) => self.broadcast(link, &s, None),
                Action::Command(line) => match command::parse(&line) {
                    Some(cmd) => self.dispatch(cmd),
                    None => tracing::warn!("internal command did not parse: {line}"),
                },
                Action::Track(q, on) => {
                    self.worker.send(Request::Track { query: q.clone(), on });
                    // Refresh any open profile so the star updates.
                    for p in &self.panes {
                        if let PaneKind::Profile { query, .. } = &p.kind {
                            self.worker.send(Request::Profile { pane: p.id, query: query.clone() });
                        }
                    }
                }
            }
        }
    }

    fn execute(&mut self, input: &str) {
        let Some(outcome) = command::parse_outcome(input) else {
            self.status = format!("Unknown command: {input}  (HELP lists functions)");
            return;
        };
        self.cmd_history.push(input.trim().to_string());
        match outcome {
            CommandOutcome::Command(cmd) => self.dispatch(cmd),
            CommandOutcome::PickList(hits) => self.open_entity_pick(input, hits),
        }
    }

    /// Open (or replace) the ephemeral entity picker for `hits`, remembering the ambiguous
    /// token in `input` for the rewrite. The toast stays as the fallback for the cases the
    /// pane carries no usable rewrite (empty hit list or no token on the line).
    fn open_entity_pick(&mut self, input: &str, hits: Vec<command::CatalogHit>) {
        let token = command::ambiguous_token(input);
        if hits.is_empty() || token.is_empty() {
            self.status = format!("Ambiguous entity ({} matches) — no entity token on the line", hits.len());
            self.toasts.push((self.status.clone(), Instant::now()));
            return;
        }
        self.open_panel(PaneKind::EntityPick(Box::new(EntityPickPane {
            command: input.trim().to_owned(),
            token: token.clone(),
            hits,
            selected: 0,
            rounds: 0,
        })));
        // Up/down and Enter move the picker, not the command line's history.
        self.focus_cmd = false;
        self.status = format!("Pick for {token:?} — Enter accepts, Esc cancels");
    }

    /// The pane index of an entity picker, if one is open.
    fn pick_pane(&self) -> Option<usize> {
        self.panes.iter().position(|p| matches!(p.kind, PaneKind::EntityPick(_)))
    }

    /// Close a picker from its Esc/×/button without executing anything.
    fn close_pick(&mut self, id: PaneId) {
        self.close_pane(id);
        self.focus_cmd = true;
        self.status = "Cancelled the pick".into();
    }

    /// Entity picker accept: rewrite the ambiguous token to the selected row's entity key
    /// (`p:<MLBAM id>:bat|pit`, preserving an explicit `.BAT`/`.PIT` suffix) and re-execute
    /// the original command. A rewritten token is a direct key, so re-resolution is exact;
    /// the rounds cap is a defensive guard against any unexpected re-ambiguation.
    fn accept_pick(&mut self, id: PaneId) {
        let Some(i) = self.pick_pane().or_else(|| self.idx(id)) else { return };
        let Some(PaneKind::EntityPick(p)) = self.panes.get(i).map(|pane| &pane.kind) else { return };
        let Some(hit) = p.hits.get(p.selected) else { return };
        if p.rounds >= PICK_ROUNDS {
            self.status = format!("Pick chain capped after {PICK_ROUNDS} rounds — refine the name");
            self.toasts.push((self.status.clone(), Instant::now()));
            return;
        }
        let Some(key) = entity_key(hit, &p.token) else {
            self.status = format!("Pick: {} has no MLBAM id to resolve", hit.name);
            self.toasts.push((self.status.clone(), Instant::now()));
            return;
        };
        let rounds = p.rounds;
        let command = p.command.clone();
        let token = p.token.clone();
        let rewritten = command.replacen(&token, &key, 1);
        match command::parse_outcome(&rewritten) {
            // Still ambiguous: rewrite again in place, capped at `PICK_ROUNDS` rewrites.
            Some(CommandOutcome::PickList(inner)) if rounds + 1 < PICK_ROUNDS => {
                let Some(PaneKind::EntityPick(p)) = self.panes.get_mut(i).map(|pane| &mut pane.kind) else { return };
                p.rounds = rounds + 1;
                p.command = rewritten;
                p.token = key;
                p.hits = inner;
                p.selected = 0;
                self.status = format!(
                    "Pick for {:?} — still ambiguous (round {}/{}), Enter accepts",
                    p.token,
                    rounds + 1,
                    PICK_ROUNDS
                );
            }
            Some(CommandOutcome::PickList(_)) => {
                self.status = format!("Still ambiguous after {PICK_ROUNDS} rounds — refine the name");
            }
            Some(CommandOutcome::Command(cmd)) => {
                self.close_pane(id);
                self.focus_cmd = true;
                self.dispatch(cmd);
            }
            None => {
                self.close_pane(id);
                self.focus_cmd = true;
                self.status = format!("Unrecognised command: {rewritten}");
            }
        }
    }

    fn dispatch(&mut self, cmd: Command) {
        match cmd {
            Command::Symbol(s) => self.open_security(&s),
            Command::SymbolFunction(s, Function::Congress) => {
                self.open_panel(PaneKind::Ticker { symbol: s.clone(), trades: Vec::new(), loading: true });
                self.status = format!("{s}: congressional trades");
            }
            Command::SymbolFunction(s, Function::News) => {
                self.open_panel(PaneKind::News(Box::new(NewsPane::new(Some(s.clone())))));
                self.status = format!("{s}: news");
            }
            Command::SymbolFunction(s, Function::OrderBook(depth)) => {
                let depth = depth.unwrap_or(self.prefs.panels.book_depth).clamp(5, 200);
                self.open_panel(PaneKind::OrderBook(Box::new(BookPane::new(&s, depth, 0.0))));
                self.status = format!("{s}: order book");
            }
            Command::SymbolFunction(s, Function::Options) => {
                self.open_panel(PaneKind::Options(Box::new(OptionsPane::new(&s, 12, true))));
                self.status = format!("{s}: option chain");
            }
            Command::SymbolFunction(s, Function::Portfolio(view, arg)) => self.portfolio_command(view, &arg, Some(s)),
            Command::SymbolFunction(s, Function::Trade(mut t)) => {
                t.symbol = Some(s);
                self.open_ticket(t, None);
            }
            Command::SymbolFunction(s, f @ (Function::NewPanel(_) | Function::Split { .. })) => {
                self.apply_with_symbol(f, Some(s));
            }
            Command::SymbolFunction(s, f) => {
                self.open_security(&s);
                self.apply(f);
            }
            Command::Function(f) => self.apply(f),
        }
    }

    fn apply(&mut self, f: Function) {
        self.apply_with_symbol(f, None);
    }

    fn apply_with_symbol(&mut self, f: Function, symbol: Option<String>) {
        match f {
            Function::Timeframe(tf) => self.with_chart(|c, w| c.set_timeframe(tf, w)),
            Function::ChartType(t) => self.with_chart(|c, _| c.chart_type = t),
            Function::Indicator(spec) => self.with_chart(|c, w| c.toggle_indicator(spec, w)),
            Function::IndicatorPicker(q) => self.palette.open_scoped(crate::palette::Kind::Indicator, &q),
            Function::ClearIndicators => self.with_chart(|c, _| c.clear_indicators()),
            Function::Compare(s) => self.with_chart(|c, w| c.set_compare(s, w)),
            Function::LogScale => self.with_chart(|c, _| c.log = !c.log),
            Function::Range(r) => self.with_chart(|c, w| c.set_range(r, w)),
            Function::Draw(t) => self.with_chart(|c, _| c.start_tool(t)),
            Function::ClearDrawings => self.with_chart(|c, _| c.drawings.clear()),
            Function::Magnet => self.with_chart(|c, _| c.magnet = !c.magnet),
            Function::Congress => self.open_panel(PaneKind::Feed { filter: FeedFilter::default() }),
            Function::Politician(q) if q.trim().is_empty() => self.open_panel(PaneKind::Politicians),
            Function::Politician(q) => self.open_panel(PaneKind::Profile { query: q, data: None, loading: true }),
            Function::Track(q) => self.worker.send(Request::Track { query: q, on: true }),
            Function::Untrack(q) => self.worker.send(Request::Track { query: q, on: false }),
            Function::Alerts => {
                self.worker.send(Request::Alerts);
                self.worker.send(crate::worker::Request::SabrAlerts);
                self.open_panel(PaneKind::Alerts);
            }
            Function::Refresh => self.worker.send(Request::RefreshCongress),
            Function::Watchlist => self.open_panel(PaneKind::Watchlist),
            Function::WatchAdd(words) => {
                for key in keys_from_words(&words) {
                    if !self.watchlist.contains(&key) {
                        self.watchlist.push(key);
                    }
                }
                self.last_quotes = None;
            }
            Function::WatchRemove(words) => {
                let keys = keys_from_words(&words);
                self.watchlist.retain(|s| !keys.contains(s));
            }
            Function::Layout(arg) => self.layout_command(&arg),
            Function::Layouts => {
                self.settings_tab = SettingsTab::Layouts;
                self.open_panel(PaneKind::Settings);
                self.status = format!("Layouts: {}", self.layout_names.join(" · "));
            }
            Function::NewPanel(spec) => {
                let kind = self.panel_from_spec(&spec, symbol.as_deref());
                match kind {
                    Some(kind) => {
                        let target = self.focused_visible();
                        let id = self.new_pane(kind);
                        self.dock_pane(id, target, crate::dock::Zone::Right);
                    }
                    None => self.status = format!("Unknown panel {spec:?}; try CHART, NEWS, BOOK, WEI, GC, CONG, WL"),
                }
            }
            Function::Split { down, panel } => match self.panel_from_spec(&panel, symbol.as_deref()) {
                Some(kind) => {
                    let target = self.focused_visible();
                    let id = self.new_pane(kind);
                    let zone = if down { crate::dock::Zone::Bottom } else { crate::dock::Zone::Right };
                    self.dock_pane(id, target, zone);
                }
                None => self.status = format!("Unknown panel {panel:?}"),
            },
            Function::ClosePane => self.close_pane(self.focus),
            Function::PopOut => self.pop_out(self.focus),
            Function::Link(link) => {
                let focus = self.focus;
                self.set_link(focus, link);
            }
            Function::News => self.open_panel(PaneKind::News(Box::new(NewsPane::new(self.focused_symbol())))),
            Function::OrderBook(depth) => {
                let sym = self.focused_symbol().unwrap_or_else(|| "BTCUSD Curncy".into());
                let depth = depth.unwrap_or(self.prefs.panels.book_depth).clamp(5, 200);
                self.open_panel(PaneKind::OrderBook(Box::new(BookPane::new(&sym, depth, 0.0))));
            }
            Function::Settings => self.open_panel(PaneKind::Settings),
            Function::Update => {
                if !tr_macos::updater::check_for_updates() {
                    let url = format!("{}/latest", tr_macos::updater::RELEASES);
                    self.status = match broker::open_external(&url) {
                        Ok(()) => "Opened the latest release on GitHub".into(),
                        Err(e) => format!("Could not open {url}: {e}"),
                    };
                }
            }
            Function::Options => {
                let sym = self.optionable_symbol();
                self.open_panel(PaneKind::Options(Box::new(OptionsPane::new(&sym, 12, true))));
            }
            Function::Sidebar(arg) => {
                let side = &mut self.prefs.appearance.sidebar_side;
                match arg.as_str() {
                    "LEFT" => *side = prefs::SidebarSide::Left,
                    "RIGHT" => *side = prefs::SidebarSide::Right,
                    "SWAP" | "FLIP" | "OTHER" => *side = side.other(),
                    "SHOW" | "ON" => self.sidebar_open = true,
                    "HIDE" | "OFF" => self.sidebar_open = false,
                    _ => self.sidebar_open = !self.sidebar_open,
                }
                self.save_prefs();
            }
            Function::Theme(t) => {
                let system = self.system_theme;
                self.theme_command(&t, system);
            }
            Function::Title(t) => {
                let focus = self.focus;
                if let Some(p) = self.pane_mut(focus) {
                    p.title = (!t.trim().is_empty()).then(|| t.trim().to_string());
                }
            }
            Function::Help => self.open_panel(PaneKind::Help),
            Function::Wei => {
                self.last_wei = None;
                self.open_panel(PaneKind::Wei);
            }
            Function::Curve => {
                self.worker.send(Request::YieldCurves);
                self.last_curve = Some(Instant::now());
                self.open_panel(PaneKind::Curve);
            }
            Function::Search(q) => {
                if !q.trim().is_empty() {
                    self.worker.send(Request::Search(q.clone()));
                }
                self.open_panel(PaneKind::Search { query: q });
            }
            Function::Portfolio(view, arg) => self.portfolio_command(view, &arg, None),
            Function::Trade(t) => self.open_ticket(t, None),
            Function::AccountMode(live) => {
                let mode = if live { prefs::AccountMode::Live } else { prefs::AccountMode::Demo };
                if mode == self.prefs.trading.account_mode {
                    self.status = format!("Already using the {} account", if live { "LIVE" } else { "DEMO" });
                }
                self.set_account_mode(mode, false);
            }
            // ---- sabr (wave 3) ------------------------------------------------------------
            Function::Cast { entity, metric, window } => match entity {
                Some(e) => {
                    self.sabr_gen += 1;
                    self.worker.send(crate::worker::Request::Series {
                        entity_key: e.key.clone(),
                        metric: metric.clone().unwrap_or_default(),
                        window,
                        as_of: None,
                        generation: self.sabr_gen,
                    });
                    self.status = format!("{e}: cast series");
                }
                None => self.status = "Cast needs an entity — try `OHTANI.PIT CAST XWOBA 50`".into(),
            },
            Function::Proj { entity, horizon, model } => {
                let Some(e) = entity else {
                    self.status = "Projection needs an entity — try `OHTANI.PIT PROJ ROS`".into();
                    return;
                };
                self.sabr_gen += 1;
                self.worker.send(crate::worker::Request::Projection {
                    entity_key: e.key.clone(),
                    horizon,
                    model,
                    generation: self.sabr_gen,
                });
                // The fan chart lands in a chart pane focused on this entity (DR-3.40).
                self.open_projection_pane(&e.key);
                self.status = format!("{e}: project {}", horizon.label());
            }
            Function::Btst(model) if model.trim().is_empty() => {
                self.status = "Backtest needs a model — try `BTST MARCEL`".into();
            }
            Function::Btst(model) => {
                self.worker.send(crate::worker::Request::Backtest { model: model.clone() });
                self.status = format!("Backtest {model} — full v1 walk-forward suite");
            }
            Function::Rost(team) if team.trim().is_empty() => {
                self.status = "Roster needs a team — try `NYM ROST`".into();
            }
            Function::Rost(team) => self.worker.send(crate::worker::Request::Rost { what: team }),
            Function::TrackPlayer(e) => {
                self.worker
                    .send(crate::worker::Request::TrackEntity { entity_key: e.key.clone(), label: e.label.clone() });
            }
            Function::UntrackPlayer(e) => {
                self.worker
                    .send(crate::worker::Request::UntrackEntity { entity_key: e.key.clone(), label: e.label.clone() });
            }
        }
    }

    // ---- worker responses ----------------------------------------------------------------

    fn chart_mut(&mut self, pane: PaneId) -> Option<&mut ChartState> {
        match self.panes.iter_mut().find(|p| p.id == pane).map(|p| &mut p.kind) {
            Some(PaneKind::Chart(c)) => Some(c),
            _ => None,
        }
    }

    fn drain(&mut self) {
        while let Ok(r) = self.worker.rx.try_recv() {
            match r {
                Response::History {
                    pane,
                    generation,
                    symbol,
                    tf,
                    bars,
                    gpu,
                    prepended,
                    from_cache,
                    exhausted,
                    live,
                } => {
                    let worker = &self.worker;
                    if let Some(PaneKind::Chart(c)) = self.panes.iter_mut().find(|p| p.id == pane).map(|p| &mut p.kind)
                        && c.symbol == symbol
                        && c.tf == tf
                    {
                        c.on_history(generation, bars, gpu, prepended, from_cache, exhausted, live, worker);
                    }
                }
                Response::HistoryError { pane, generation, error } => {
                    // Errors while paging just mean "no older history"; only report failed loads.
                    let empty = self.chart_mut(pane).map(|c| {
                        c.on_history_error(generation, error.clone());
                        c.is_empty()
                    });
                    if empty == Some(true) {
                        self.status = format!("data error: {error}");
                    }
                }
                Response::Indicators { pane, generation, built } => {
                    if let Some(c) = self.chart_mut(pane) {
                        c.on_indicators(generation, built);
                    }
                }
                Response::Compare { pane, generation, symbol, line } => {
                    if let Some(c) = self.chart_mut(pane) {
                        c.on_compare(generation, &symbol, line);
                    }
                }
                Response::Markers { pane, symbol, trades } => {
                    if let Some(c) = self.chart_mut(pane) {
                        c.on_markers(&symbol, trades);
                    }
                }
                Response::Quotes(q) => {
                    if q.len() > 20 {
                        self.wei_updated = Some(format!("updated {}", chrono::Local::now().format("%H:%M:%S")));
                    }
                    let keys: Vec<String> = q.iter().map(|x| x.symbol.clone()).collect();
                    for x in q {
                        self.quotes.insert(x.symbol.clone(), x);
                    }
                    self.demo_on_quotes(&keys);
                }
                Response::Latest(t) => self.latest = t,
                Response::TickerTrades { pane, symbol, trades } => {
                    if let Some(p) = self.panes.iter_mut().find(|p| p.id == pane)
                        && let PaneKind::Ticker { symbol: s, trades: t, loading } = &mut p.kind
                        && *s == symbol
                    {
                        *t = trades;
                        *loading = false;
                    }
                }
                Response::Profile { pane, profile, query } => {
                    if let Some(p) = self.panes.iter_mut().find(|p| p.id == pane)
                        && let PaneKind::Profile { query: q, data, loading } = &mut p.kind
                        && *q == query
                    {
                        *data = profile;
                        *loading = false;
                    }
                }
                Response::Options { pane, symbol, chain } => {
                    // Greeks use the 3-month T-bill yield when the curve is loaded.
                    let rate = self.curves.as_ref().and_then(|c| c.current.get("US3M")).map_or(0.04, |y| y / 100.0);
                    if let Some(p) = self.panes.iter_mut().find(|p| p.id == pane)
                        && let PaneKind::Options(o) = &mut p.kind
                        && o.symbol == symbol
                    {
                        match chain {
                            Ok(c) => o.set_chain(c, rate, chrono::Utc::now().timestamp()),
                            Err(e) => {
                                o.error = Some(e);
                                o.loading = false;
                            }
                        }
                    }
                }
                Response::News { pane, symbol, items } => {
                    if let Some(p) = self.panes.iter_mut().find(|p| p.id == pane)
                        && let PaneKind::News(n) = &mut p.kind
                        && n.symbol == symbol
                    {
                        n.loading = false;
                        match items {
                            Ok(items) => {
                                n.items = items;
                                n.error = None;
                            }
                            Err(e) => n.error = Some(e),
                        }
                    }
                }
                Response::OrderBook { pane, symbol, book } => {
                    if let Some(p) = self.panes.iter_mut().find(|p| p.id == pane)
                        && let PaneKind::OrderBook(b) = &mut p.kind
                        && b.symbol == symbol
                    {
                        match book {
                            Ok(book) => {
                                b.book = Some(book);
                                b.error = None;
                            }
                            Err(e) => b.error = Some(e),
                        }
                    }
                }
                Response::Portfolio(result) => {
                    let result = self.keep_simulated_trades(result);
                    self.on_portfolio(result)
                }
                Response::BrokerRegistered(result) => self.on_broker_registered(result),
                Response::BrokerPortal(result) => self.on_broker_portal(result),
                Response::BrokerDisconnected(result) => self.on_broker_disconnected(result),
                Response::Politicians(list) => self.politicians = list,
                Response::Alerts(a) => self.alerts = a,
                Response::NewAlerts(new) => {
                    for a in new {
                        tracing::info!("ALERT: {}", a.message);
                        self.toasts.push((a.message, Instant::now()));
                    }
                    self.worker.send(Request::Alerts);
                }
                Response::Status(s) => self.status = s,
                Response::Curves(c) => self.curves = Some(c),
                Response::SearchHits { query, hits } => self.net_hits = (query, hits),
                Response::Trade(r) => self.on_trade(*r),
                Response::CongressRefreshed(summary) => {
                    self.congress_status = summary.to_string();
                    self.congress_checked = Some(chrono::Local::now());
                    self.status = format!("Congress disclosures updated: {summary}");
                    self.refresh_congress_views();
                }
                // ---- sabr (wave 3) -------------------------------------------------------
                Response::SabrSeries { generation, entity_key, metric, points } => {
                    // Store by generation: a stale response for the same entity loses to the
                    // newest load (same rule as charts).
                    let stale = self.sabr_series.get(&entity_key).is_some_and(|(g, _, _)| *g > generation);
                    if !stale {
                        self.sabr_series.insert(entity_key.clone(), (generation, points.clone(), metric.clone()));
                        self.status =
                            format!("{entity_key} {metric}: {} points derived (as of view time)", points.len());
                    } else {
                        tracing::debug!("dropped stale sabr series gen {generation} for {entity_key}");
                    }
                }
                Response::SabrProjection { generation, entity_key, stamp, woba, pa, reliability, quantiles } => {
                    // A completed run supersedes any earlier fit for the same request.
                    tracing::debug!("sabr projection gen {generation} for {entity_key}");
                    self.sabr_projection = Some(Box::new(crate::worker::SabrProjectionRow {
                        entity_key: entity_key.clone(),
                        stamp: *stamp.clone(),
                        woba: woba.unwrap_or(f64::NAN),
                        pa: pa.unwrap_or(f64::NAN),
                        reliability: reliability.unwrap_or(f64::NAN),
                        quantiles: quantiles.as_deref().cloned(),
                    }));
                    self.status = format!(
                        "{entity_key}: {} v{} project — wOBA {:.3}, PA {:.0}, reliability {:.2}, as-of {}, band score {}",
                        stamp.id,
                        stamp.version,
                        woba.unwrap_or(f64::NAN),
                        pa.unwrap_or(f64::NAN),
                        stamp.as_of,
                        reliability.unwrap_or(f64::NAN),
                        stamp.backtest_score.map(|s| format!("{s:.3}")).unwrap_or_else(|| "n/a".into()),
                    );
                }
                Response::SabrBacktest { model, seasons, pooled_mae, gate_pass } => {
                    self.sabr_backtest = Some((model.clone(), pooled_mae, seasons.len(), gate_pass));
                    self.status = format!(
                        "Backtest {model}: pooled wOBA MAE {pooled_mae:.3} over {} seasons, gate {}",
                        seasons.len(),
                        if gate_pass { "pass" } else { "FAIL" }
                    );
                }
                Response::SabrTracked { entity_key, label, was_new } => {
                    if !self.sabr_tracked.contains(&entity_key) {
                        self.sabr_tracked.push(entity_key.clone());
                    }
                    let verb = if was_new { "Tracking" } else { "Already tracking" };
                    self.toasts.push((
                        format!(
                            "{verb} {label} ({entity_key}) — {} series cached, {} alerts",
                            self.sabr_series.len(),
                            self.sabr_alerts.len()
                        ),
                        Instant::now(),
                    ));
                    self.status = format!("{verb} {label}");
                }
                Response::SabrUntracked { entity_key, label, was_tracked } => {
                    self.sabr_tracked.retain(|k| *k != entity_key);
                    self.toasts.push((
                        if was_tracked {
                            format!("Stopped tracking {label}")
                        } else {
                            format!("{label} was not tracked")
                        },
                        Instant::now(),
                    ));
                    self.status = format!("Untracked {label}");
                }
                Response::SabrAlerts(list) => {
                    // Toast each unseen alert once (they were persisted seen-state in the store).
                    for a in list.iter().filter(|a| !a.seen) {
                        self.toasts.push((a.message.clone(), Instant::now()));
                    }
                    let unseen = list.iter().filter(|a| !a.seen).count();
                    self.sabr_alerts = list;
                    if unseen > 0 {
                        self.status = format!("sabr alerts: {unseen} unseen");
                    }
                }
                Response::Unsupported { reason } => {
                    tracing::warn!("sabr request unsupported: {reason}");
                    self.status = format!("Unsupported: {reason}");
                }
                Response::OutOfScope { what } => {
                    tracing::info!("sabr request out of scope: {what}");
                    self.status = format!("Out of scope this wave: {what}");
                }
            }
        }
    }

    /// Freshness line for the Congress feed: last check, newest filing, next check.
    fn congress_freshness(&self) -> String {
        let newest = self.latest.iter().map(|t| t.filed_date).max();
        let minutes = std::env::var("TR_CONGRESS_REFRESH_MIN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(crate::worker::CONGRESS_REFRESH_MIN as i64);
        let mut parts = Vec::new();
        match self.congress_checked {
            Some(t) => {
                parts.push(format!("checked {}", t.format("%H:%M")));
                if minutes > 0 {
                    parts.push(format!("next {}", (t + chrono::Duration::minutes(minutes)).format("%H:%M")));
                }
            }
            None => parts.push("checking Senate eFD + House Clerk…".into()),
        }
        if let Some(d) = newest {
            parts.push(format!("newest filing {d}"));
        }
        if !self.congress_status.is_empty() {
            parts.push(self.congress_status.clone());
        }
        parts.join(" · ")
    }

    fn refresh_congress_views(&mut self) {
        self.worker.send(Request::Latest { limit: 2_000 });
        self.worker.send(Request::SearchPoliticians(String::new()));
        for p in &self.panes {
            match &p.kind {
                PaneKind::Ticker { symbol, .. } => {
                    self.worker.send(Request::TickerTrades { pane: p.id, symbol: symbol.clone() })
                }
                PaneKind::Chart(c) => self.worker.send(Request::Markers { pane: p.id, symbol: c.symbol.clone() }),
                _ => {}
            }
        }
        self.last_feed = Some(Instant::now());
    }

    // ---- input ---------------------------------------------------------------------------

    /// Global and chart hotkeys. Runs before the command line so it can claim keys; when
    /// the command line holds text, keys belong to it.
    fn hotkeys(&mut self, ctx: &egui::Context) {
        let presses: Vec<(Key, Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !prefs::is_modifier(*key) => {
                        Some((*key, *modifiers))
                    }
                    _ => None,
                })
                .collect()
        });
        // Settings is capturing a new binding: the next key press is it.
        if let Some(action) = self.rebinding {
            if let Some(&(key, m)) = presses.first() {
                ctx.input_mut(|i| i.consume_key(m, key));
                self.rebinding = None;
                if key != Key::Escape {
                    let chord = prefs::Chord::from_event(key, m);
                    let displaced = self.prefs.hotkeys.bind(action, chord);
                    self.status = match displaced {
                        Some(d) => {
                            format!("{} bound to {} (removed from {})", action.label(), chord.display(), d.label())
                        }
                        None => format!("{} bound to {}", action.label(), chord.display()),
                    };
                    self.save_prefs();
                }
            }
            return;
        }
        // ⌘K (Ctrl+K) opens and closes the palette from anywhere, even from a text field.
        if let Some(&(key, m)) =
            presses.iter().find(|(k, m)| self.prefs.hotkeys.action_for(*k, *m) == Some(prefs::Action::CommandPalette))
        {
            ctx.input_mut(|i| i.consume_key(m, key));
            self.palette.toggle();
            return;
        }
        // While it is open, the palette owns the keyboard.
        if self.palette.open {
            return;
        }
        let cmd_id = egui::Id::new("command-line");
        let focused = ctx.memory(|m| m.focused());
        let cmd_focused = focused == Some(cmd_id);
        let other_text_focused = focused.is_some() && !cmd_focused;
        if other_text_focused {
            return;
        }
        // Chorded bindings (⌘, Ctrl, Alt) work even while the command line has text.
        let mut swallow_text = false;
        for &(key, m) in &presses {
            let Some(action) = self.prefs.hotkeys.action_for(key, m) else { continue };
            let chorded = m.command || m.ctrl || m.alt;
            if !chorded && !self.cmd.is_empty() {
                continue;
            }
            // ⇧B / ⇧S type letters: never while the command line is being typed into.
            let letter = !chorded && Self::types_text(key);
            if letter && cmd_focused {
                continue;
            }
            if !self.hotkey_applies(action) {
                continue;
            }
            ctx.input_mut(|i| i.consume_key(m, key));
            swallow_text |= letter;
            self.run_hotkey(action);
        }
        if !cmd_focused && !swallow_text {
            // Type-anywhere: capture keystrokes until the command line has focus.
            // Judge modifiers per key event: the Alt release can arrive in the same frame
            // as Alt+T, so the frame-level modifier state is unreliable.
            let typed: String = ctx.input(|i| {
                let mut chord = false;
                let mut out = String::new();
                for e in &i.events {
                    match e {
                        Event::Key { pressed: true, modifiers, .. } => {
                            chord = modifiers.alt || modifiers.ctrl || modifiers.command
                        }
                        Event::Text(t) if !chord => out.push_str(t),
                        _ => {}
                    }
                }
                out
            });
            if !typed.is_empty() {
                let starts = !self.cmd.is_empty()
                    || typed.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '^' || c == '?');
                if starts {
                    self.cmd.push_str(&typed.to_ascii_uppercase());
                    self.focus_cmd = true;
                }
            }
        }
        if !self.cmd.is_empty() || !self.focused_is_chart() {
            return;
        }
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if pressed(Modifiers::NONE, Key::Escape) {
            self.with_chart(|c, _| {
                c.cancel();
            });
        }
        if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            self.with_chart(|c, _| {
                c.delete_selected();
            });
        }
        if pressed(Modifiers::NONE, Key::ArrowLeft) {
            self.with_chart(|c, _| c.pan_bars(-5.0));
        }
        if pressed(Modifiers::NONE, Key::ArrowRight) {
            self.with_chart(|c, _| c.pan_bars(5.0));
        }
        if pressed(Modifiers::COMMAND, Key::ArrowUp)
            || pressed(Modifiers::NONE, Key::Plus)
            || pressed(Modifiers::NONE, Key::Equals)
        {
            self.with_chart(|c, _| c.zoom(1.25));
        }
        if pressed(Modifiers::COMMAND, Key::ArrowDown) || pressed(Modifiers::NONE, Key::Minus) {
            self.with_chart(|c, _| c.zoom(0.8));
        }
    }

    /// Keys that also produce text (a bare or shifted letter, digit or symbol).
    fn types_text(key: Key) -> bool {
        let n = key.name();
        n.len() == 1 || matches!(key, Key::Space | Key::Minus | Key::Plus | Key::Equals | Key::Period | Key::Comma)
    }

    fn hotkey_applies(&self, a: prefs::Action) -> bool {
        use prefs::Action as A;
        match a {
            A::DrawTrendline
            | A::DrawHorizontal
            | A::DrawVertical
            | A::DrawFib
            | A::ResetView
            | A::LogScale
            | A::Magnet => self.focused_is_chart(),
            a => a.layout_slot().is_none_or(|i| i < self.layout_names.len()),
        }
    }

    fn run_hotkey(&mut self, a: prefs::Action) {
        use prefs::Action as A;
        match a {
            A::Help => self.apply(Function::Help),
            A::Congress => self.apply(Function::Congress),
            A::Politicians => self.apply(Function::Politician(String::new())),
            A::Alerts => self.apply(Function::Alerts),
            A::Watchlist => self.apply(Function::Watchlist),
            A::Refresh => self.apply(Function::Refresh),
            A::FrameTiming => self.show_perf = !self.show_perf,
            A::Portfolio => self.apply(Function::Portfolio(command::PortView::Summary, String::new())),
            A::Positions => self.apply(Function::Portfolio(command::PortView::Positions, String::new())),
            A::Orders => self.apply(Function::Portfolio(command::PortView::Orders, String::new())),
            A::Buy => self.quick_trade(true),
            A::Sell => self.quick_trade(false),
            A::NextPane => self.cycle_focus(1),
            A::PrevPane => self.cycle_focus(-1),
            A::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
            A::NextTheme => self.cycle_theme(1),
            A::SwapSidebarSide => self.apply(Function::Sidebar("SWAP".into())),
            A::SplitRight => self.apply(Function::Split { down: false, panel: String::new() }),
            A::SplitDown => self.apply(Function::Split { down: true, panel: String::new() }),
            A::NewTab => {
                let target = self.focused_visible();
                let kind = self.panel_from_spec("", None).expect("chart");
                let id = self.new_pane(kind);
                self.dock_pane(id, target, crate::dock::Zone::Center);
            }
            A::ClosePane => self.close_pane(self.focus),
            A::PopOut => self.pop_out(self.focus),
            A::SaveLayout => {
                self.save_layout();
                self.status = format!("Saved layout {:?}", self.layout_name);
            }
            A::Settings => self.apply(Function::Settings),
            A::CommandPalette => self.palette.toggle(),
            A::DrawTrendline => self.with_chart(|c, _| c.start_tool(Tool::Trendline)),
            A::DrawHorizontal => self.with_chart(|c, _| c.start_tool(Tool::Horizontal)),
            A::DrawVertical => self.with_chart(|c, _| c.start_tool(Tool::Vertical)),
            A::DrawFib => self.with_chart(|c, _| c.start_tool(Tool::Fib)),
            A::ResetView => self.with_chart(|c, _| c.reset_view()),
            A::LogScale => self.with_chart(|c, _| c.log = !c.log),
            A::Magnet => self.with_chart(|c, _| c.magnet = !c.magnet),
            a => {
                if let Some(spec) = a.panel_spec() {
                    self.apply(Function::NewPanel(spec.into()));
                } else if let Some(name) = a.layout_slot().and_then(|i| self.layout_names.get(i)).cloned() {
                    self.switch_layout(&name);
                }
            }
        }
    }

    fn periodic(&mut self) {
        let visible = self.visible_panes();
        let shown = |f: &dyn Fn(&PaneKind) -> bool| visible.iter().any(|&i| f(&self.panes[i].kind));
        let visible_wl = self.sidebar_open || shown(&|k| matches!(k, PaneKind::Watchlist));
        if visible_wl && self.last_quotes.is_none_or(|t| t.elapsed() > Duration::from_secs(20)) {
            self.worker.send(Request::Quotes(self.watchlist.clone()));
            self.last_quotes = Some(Instant::now());
        }
        if shown(&|k| matches!(k, PaneKind::Wei)) && self.last_wei.is_none_or(|t| t.elapsed() > Duration::from_secs(20))
        {
            self.worker.send(Request::Quotes(markets_view::overview_keys()));
            self.last_wei = Some(Instant::now());
        }
        if shown(&|k| matches!(k, PaneKind::Curve))
            && self.last_curve.is_none_or(|t| t.elapsed() > Duration::from_secs(600))
        {
            self.worker.send(Request::YieldCurves);
            self.last_curve = Some(Instant::now());
        }
        // News every 5 minutes and order books every second, while visible.
        for &i in &visible {
            let p = &mut self.panes[i];
            match &mut p.kind {
                PaneKind::News(n) if n.fetched.is_none_or(|t| t.elapsed() > Duration::from_secs(300)) => {
                    n.fetched = Some(Instant::now());
                    self.worker.send(Request::News { pane: p.id, symbol: n.symbol.clone() });
                }
                PaneKind::Options(o) if o.fetched.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) => {
                    o.fetched = Some(Instant::now());
                    self.worker.send(Request::Options {
                        pane: p.id,
                        symbol: o.symbol.clone(),
                        expiration: o.expiration,
                    });
                }
                PaneKind::OrderBook(b) if b.fetched.is_none_or(|t| t.elapsed() > Duration::from_millis(1000)) => {
                    b.fetched = Some(Instant::now());
                    self.worker.send(Request::OrderBook { pane: p.id, symbol: b.symbol.clone(), depth: b.depth });
                }
                _ => {}
            }
        }
        self.broker_periodic(&visible);
        if let Some((q, at)) = &self.search_pending
            && at.elapsed() > Duration::from_millis(300)
        {
            self.worker.send(Request::Search(q.clone()));
            self.search_pending = None;
        }
        if self.last_feed.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            self.worker.send(Request::Latest { limit: 2_000 });
            self.last_feed = Some(Instant::now());
        }
        self.save_dirty_themes(false);
        if self.last_save.elapsed() > Duration::from_secs(10) {
            self.autosave();
            self.last_save = Instant::now();
        }
        self.toasts.retain(|(_, t)| t.elapsed() < Duration::from_secs(TOAST_SECS));
    }

    /// Open (or focus) a chart pane and attach the latest projection's fan bands.
    fn open_projection_pane(&mut self, entity_key: &str) {
        if let Some(fan) = self.fan_from_projection(entity_key) {
            // Reuse an existing chart pane showing this entity, else the focused chart pane.
            let idx = self
                .panes
                .iter()
                .position(|p| matches!(&p.kind, PaneKind::Chart(c) if c.fan.as_ref().is_some_and(|f| f.label.starts_with(entity_key))))
                .or_else(|| self.panes.iter().position(|p| p.id == self.focus && matches!(p.kind, PaneKind::Chart(_))))
                .or_else(|| self.panes.iter().position(|p| matches!(p.kind, PaneKind::Chart(_))));
            match idx {
                Some(i) => {
                    if let PaneKind::Chart(c) = &mut self.panes[i].kind {
                        c.fan = Some(fan);
                    }
                }
                None => {
                    // No chart pane exists: the bands ride on the status line only.
                    tracing::debug!("no chart pane to attach the fan to");
                }
            }
        }
    }

    /// Build the fan data from the latest projection row (12 horizon points, Jan..Oct).
    fn fan_from_projection(&self, entity_key: &str) -> Option<crate::chart_view::FanData> {
        let p = self.sabr_projection.as_ref()?;
        if p.entity_key != entity_key {
            return None;
        }
        let q = p.quantiles.as_ref()?;
        let n = 12usize;
        let median: Vec<f64> = (0..n).map(|_| p.woba).collect();
        let band = |lo: f64, hi: f64| -> (Vec<f64>, Vec<f64>) {
            ((0..n).map(|_| p.woba + lo).collect(), (0..n).map(|_| p.woba + hi).collect())
        };
        Some(crate::chart_view::FanData {
            median,
            bands: [band(q.q25, q.q75), band(q.q10, q.q90), band(q.q05, q.q95)],
            label: format!("{entity_key} {}", p.stamp.id),
        })
    }

    /// One headless UI pass over every pane — no window, no display server (the sandboxed
    /// smoke). Returns the clipped-shape count so a blank frame is detectable.
    pub(crate) fn headless_pass(&mut self, ctx: &egui::Context, rect: egui::Rect) -> usize {
        let n = self.panes.len();
        let mut shapes = 0;
        for i in 0..n {
            let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
            let out = ctx.run_ui(input, |ui| {
                let _ = self.render_content(ui, i, rect, i + 1 == n);
            });
            shapes += out.shapes.len();
        }
        shapes
    }

    /// One headless smoke stage: optionally apply a command (a preset), then render every
    /// pane `frames` times and report what was drawn. No window, no display server.
    pub(crate) fn smoke_stage(&mut self, ctx: &egui::Context, command: Option<&str>, frames: u32) -> String {
        if let Some(c) = command {
            self.execute(c);
        }
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        let t0 = std::time::Instant::now();
        let mut shapes = 0;
        for _ in 0..frames.max(1) {
            shapes = self.headless_pass(ctx, rect);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / frames.max(1) as f64;
        format!("{} panes, {shapes} clipped shapes, {ms:.2} ms/frame", self.panes.len())
    }
    /// Save the active layout (only when it changed) and app settings.
    fn autosave(&mut self) {
        self.save_dirty_themes(true);
        let json = self.current_doc().to_json();
        if json != self.last_saved {
            self.save_layout();
        }
        self.save_settings();
    }
}

/// Apply the native macOS window chrome (unified toolbar, inset traffic lights, sidebar
/// vibrancy). Returns whether the window is now translucent.
#[cfg(target_os = "macos")]
fn style_native_window(frame: &eframe::Frame, vibrancy: bool) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let vibrancy = vibrancy && std::env::var("TR_VIBRANCY").map_or(true, |v| v != "0");
    let Ok(handle) = frame.window_handle() else { return false };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return false };
    // SAFETY: called on the main thread (eframe runs `ui` there) with a live NSView.
    let ok = unsafe { tr_macos::style_window(h.ns_view.as_ptr(), vibrancy) };
    tracing::info!("macOS window chrome applied: {ok} (vibrancy {vibrancy})");
    ok && vibrancy
}

#[cfg(not(target_os = "macos"))]
fn style_native_window(_frame: &eframe::Frame, _vibrancy: bool) -> bool {
    false
}

/// The main window's live title bar geometry (macOS).
#[cfg(target_os = "macos")]
fn native_chrome(frame: &eframe::Frame) -> Option<tr_macos::Chrome> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: main thread, live NSView owned by the window.
    unsafe { tr_macos::chrome_metrics(h.ns_view.as_ptr()) }
}

#[cfg(not(target_os = "macos"))]
fn native_chrome(_frame: &eframe::Frame) -> Option<tr_macos::Chrome> {
    None
}

impl eframe::App for TerminalApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.vibrancy { [0.0; 4] } else { visuals.window_fill.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let frame_start = Instant::now();
        if let Some(prev) = self.last_frame.replace(frame_start) {
            let dt = prev.elapsed().as_secs_f64() * 1e3;
            self.perf_dt_ms = if self.perf_dt_ms == 0.0 { dt } else { self.perf_dt_ms * 0.9 + dt * 0.1 };
        }
        let ctx = ui.ctx().clone();
        if !self.window_styled {
            self.window_styled = true;
            self.vibrancy = style_native_window(frame, self.prefs.appearance.vibrancy);
        } else if !self.updater_started {
            // From the second frame, so loading Sparkle doesn't delay the first.
            self.updater_started = true;
            tr_macos::updater::start();
        }
        if self.last_chrome_poll.is_none_or(|t| t.elapsed() > Duration::from_millis(750)) {
            let c = native_chrome(frame);
            if c != self.chrome {
                tracing::info!("window chrome: {c:?}");
                self.chrome = c;
            }
            self.last_chrome_poll = Some(Instant::now());
        }
        if self.last_accent_poll.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            if let Some([r, g, b]) = tr_macos::accent_color() {
                self.system_accent = Color32::from_rgb(r, g, b);
            }
            anim::set_reduced(tr_macos::reduce_motion());
            self.last_accent_poll = Some(Instant::now());
        }
        self.system_theme = ctx.system_theme();
        let ti = self.active_theme_index(&ctx);
        if self.applied_theme.as_ref() != Some(&self.themes[ti]) {
            theme::configure(&ctx, &self.themes[ti], self.system_accent);
            self.applied_theme = Some(self.themes[ti].clone());
        }
        theme::refresh(&ctx, &self.themes[ti], self.system_accent);
        self.drain();
        self.periodic();

        let screen = ui.max_rect();
        let open = anim::spring_with(
            &ctx,
            egui::Id::new("sidebar-open"),
            if self.sidebar_open { 1.0 } else { 0.0 },
            0.32,
            0.9,
        );
        let sb_w = (SIDEBAR_W * open).round();
        let right_side = self.prefs.appearance.sidebar_side == prefs::SidebarSide::Right;
        let main = if right_side {
            Rect::from_min_max(screen.min, pos2(screen.right() - sb_w, screen.bottom()))
        } else {
            Rect::from_min_max(pos2(screen.left() + sb_w, screen.top()), screen.max)
        };
        ui.painter().rect_filled(main, 0.0, theme::window());
        let toolbar = Rect::from_min_size(main.min, vec2(main.width(), self.toolbar_h()));
        let status = Rect::from_min_max(pos2(main.left(), main.bottom() - STATUS_H), main.max);
        let gap = theme::dp(6.0);
        let area = Rect::from_min_max(
            pos2(main.left() + gap, toolbar.bottom() + gap),
            pos2(main.right() - gap, status.top() - gap * 0.5),
        );

        self.hotkeys(&ctx);
        self.command_palette(&ctx);
        let mut sidebar_cmds = Vec::new();
        if sb_w >= 1.0 {
            // The sidebar slides in from its edge.
            let x = if right_side { screen.right() - sb_w } else { screen.left() + sb_w - SIDEBAR_W };
            let full = Rect::from_min_size(pos2(x, screen.top()), vec2(SIDEBAR_W, screen.height()));
            let mut child = ui.new_child(UiBuilder::new().max_rect(full).id_salt("sidebar-root"));
            let visible = if right_side {
                Rect::from_min_max(pos2(screen.right() - sb_w, screen.top()), screen.max)
            } else {
                Rect::from_min_max(screen.min, pos2(screen.left() + sb_w, screen.bottom()))
            };
            child.set_clip_rect(visible);
            sidebar_cmds = self.sidebar(&mut child, full);
        }
        // The traffic lights sit at the window's left edge: clear them unless a left
        // sidebar already covers that corner.
        let inset = if right_side { self.traffic_inset() } else { (self.traffic_inset() - sb_w).max(0.0) };
        self.toolbar(ui, toolbar, inset);
        self.status_bar(ui, status);

        let mut actions = Vec::new();
        for c in sidebar_cmds {
            match c {
                SidebarCmd::Action(a) => actions.push(a),
                SidebarCmd::Function(f) => self.apply(f),
                SidebarCmd::Layout(op) => self.layout_op(op),
            }
        }
        actions.extend(self.render_dock(ui, area, gap));
        actions.extend(self.render_windows(&ctx));
        self.run_actions(actions);
        self.trading_windows(&ctx);
        self.toasts(&ctx);

        // TR_SMOKE_SECS=N: render for N seconds, log frame stats and quit (CI smoke test).
        if let Some(secs) = std::env::var("TR_SMOKE_SECS").ok().and_then(|v| v.parse::<f64>().ok()) {
            ctx.request_repaint();
            if !self.smoke_done && self.started.elapsed().as_secs_f64() > secs {
                self.smoke_done = true;
                tracing::info!(
                    "smoke: {} frames in {:.1}s, ui cpu {:.2} ms/frame, panes {}, layout {:?}",
                    ctx.cumulative_frame_nr(),
                    self.started.elapsed().as_secs_f64(),
                    self.perf_cpu_ms,
                    self.panes.len(),
                    self.layout_name
                );
                self.autosave();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        let cpu = frame_start.elapsed().as_secs_f64() * 1e3;
        self.perf_cpu_ms = self.perf_cpu_ms * 0.9 + cpu * 0.1;
        if self.show_perf {
            // Measure continuously while the readout is visible.
            ctx.request_repaint();
        }
        // Keep the clock ticking and pick up worker results without spinning.
        ctx.request_repaint_after(Duration::from_millis(500));
        if self.started.elapsed() < Duration::from_secs(2) {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        self.autosave();
        if self.demo.save_error.is_some() {
            self.demo.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_migrate_to_a_layout() {
        let old = r#"{"layout":"MainRight2","watchlist":["AAPL","BTCUSD Curncy"],"sidebar":false,
            "panes":[{"Chart":{"symbol":"NVDA","tf":"D1","chart_type":"Candles","log":false,"indicators":[],
            "drawings":[],"compare":null}},"Feed","Wei"]}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert!(!s.sidebar);
        let doc = migrate_legacy(&s, &prefs::PanelDefaults::default()).unwrap();
        assert_eq!(doc.panes.len(), 3);
        assert_eq!(doc.dock.visible(), vec![1, 2, 3]);
        assert!(matches!(&doc.panes[0].settings, PaneSettings::Chart { symbol, .. } if symbol == "NVDA"));
        assert_eq!(doc.panes[1].settings, PaneSettings::Feed(None));
        // New-format files carry no legacy fields and are not migrated.
        let new: Settings = serde_json::from_str(r#"{"watchlist":[],"active_layout":"Desk"}"#).unwrap();
        assert!(migrate_legacy(&new, &prefs::PanelDefaults::default()).is_none());
        assert_eq!(new.active_layout.as_deref(), Some("Desk"));
    }

    #[test]
    fn pane_settings_round_trip_through_kinds() {
        let d = prefs::PanelDefaults::default();
        for s in [
            crate::layouts::chart_settings("BTCUSD Curncy", &d),
            PaneSettings::Feed(Some(crate::layouts::FeedSettings { buys: Some(true), ..Default::default() })),
            PaneSettings::News(Some("AAPL US Equity".into())),
            PaneSettings::News(None),
            PaneSettings::OrderBook { symbol: "ETHUSD Curncy".into(), depth: 50, group: 1.0 },
            PaneSettings::Search("gold".into()),
            PaneSettings::Settings,
            PaneSettings::Wei,
        ] {
            assert_eq!(PaneKind::from_settings(s.clone()).settings(), Some(s));
        }

        // The entity picker is ephemeral: it has no PaneSettings, is skipped by the save
        // path (`current_doc`) and therefore never survives a layout round trip.
        let pick = PaneKind::EntityPick(Box::new(EntityPickPane {
            command: "DIAZ CAST".into(),
            token: "DIAZ".into(),
            hits: Vec::new(),
            selected: 0,
            rounds: 0,
        }));
        assert_eq!(pick.settings(), None);
        assert_eq!(pick.code(), "PICK");
    }

    // ---- entity picker harness ------------------------------------------------------------

    /// A headless app over a probe worker writing nothing outside a temp data directory.
    fn picker_app() -> (TerminalApp, tokio::sync::mpsc::UnboundedReceiver<crate::worker::Request>, egui::Context) {
        let dir = std::env::temp_dir().join("terminal-rs-picker-tests");
        // SAFETY: single-threaded harness startup; values are identical across tests.
        unsafe { std::env::set_var("TR_DATA_DIR", &dir) };
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = egui::Context::default();
        let (worker, reqs) = crate::worker::Worker::probe();
        (TerminalApp::new(&ctx, worker, "sample".into()), reqs, ctx)
    }

    fn pick_of(app: &TerminalApp) -> &EntityPickPane {
        let i = app.pick_pane().expect("picker pane is open");
        match &app.panes[i].kind {
            PaneKind::EntityPick(p) => p,
            _ => unreachable!(),
        }
    }

    #[test]
    fn ambiguous_entity_opens_the_ranked_picker() {
        let (mut app, _reqs, _) = picker_app();
        app.execute("DIAZ CAST");
        let p = pick_of(&app);
        let doc = app.current_doc();
        assert_eq!(doc.panes.len(), 4, "default workspace is saved without the ephemeral picker");
        assert_eq!(p.command, "DIAZ CAST");
        assert_eq!(p.token, "DIAZ");
        assert!(p.hits.len() > 2);
        assert!(p.hits.iter().any(|h| h.name.contains("Díaz")));
        assert!(p.hits.iter().all(|h| h.mlbam.is_some()));
        assert!(!app.focus_cmd, "arrow keys move the selection, not the command line");
        assert!(app.status.contains("Enter accepts"));
    }

    #[test]
    fn accepting_a_pick_rewrites_the_token_and_reexecutes() {
        let (mut app, mut reqs, _) = picker_app();
        app.execute("DIAZ CAST");
        let pane = app.pick_pane().unwrap();
        let id = app.panes[pane].id;
        // The key the selected top row maps to; the rewrite must resolve to exactly it.
        let p = pick_of(&app);
        let hit = &p.hits[p.selected];
        let expected = entity_key(hit, &p.token).unwrap();
        let expected_label = hit.name.clone();
        while reqs.try_recv().is_ok() {}
        app.accept_pick(id);
        let mut series = None;
        while let Ok(req) = reqs.try_recv() {
            if let crate::worker::Request::Series { entity_key, .. } = &req {
                assert!(series.is_none(), "exactly one cast ran");
                assert_eq!(*entity_key, expected, "the rewritten cast targets the picked entity");
                series = Some(entity_key.clone());
            }
        }
        if series.is_none() {
            panic!("the rewritten command never cast a series");
        }
        assert!(app.status.contains(&expected_label) && app.status.contains("cast series"));
        assert_eq!(app.pick_pane(), None, "the picker closed");
        assert!(app.focus_cmd);
        while reqs.try_recv().is_ok() {}
    }

    #[test]
    fn picker_arrow_keys_and_accept_move_the_selection() {
        let (mut app, mut reqs, ctx) = picker_app();
        app.execute("DIAZ CAST");
        let pane = app.pick_pane().unwrap();
        let i = pane;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 320.0));
        let key = |k: egui::Key| egui::RawInput {
            events: vec![egui::Event::Key {
                key: k,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
                physical_key: None,
            }],
            ..Default::default()
        };
        let draw = |input, app: &mut TerminalApp| {
            ctx.run_ui(input, |ui| {
                let _ = app.render_content(ui, i, rect, true);
            })
            .textures_delta
            .clear();
        };
        draw(key(egui::Key::ArrowDown), &mut app);
        assert_eq!(pick_of(&app).selected, 1, "↓ moves the selection");
        draw(key(egui::Key::ArrowUp), &mut app);
        assert_eq!(pick_of(&app).selected, 0, "↑ moves back");
        let p = pick_of(&app);
        let expected = entity_key(&p.hits[p.selected], p.token.as_str()).unwrap();
        while reqs.try_recv().is_ok() {}
        draw(key(egui::Key::Enter), &mut app);
        let mut series = None;
        while let Ok(req) = reqs.try_recv() {
            if let crate::worker::Request::Series { entity_key, .. } = &req {
                assert!(series.is_none(), "exactly one cast ran");
                assert_eq!(*entity_key, expected, "Enter accepts the selected row");
                series = Some(entity_key.clone());
            }
        }
        if series.is_none() {
            panic!("the rewritten command never cast a series");
        }
        assert_eq!(app.pick_pane(), None);
    }

    #[test]
    fn picker_esc_closes_without_rerunning() {
        let (mut app, mut reqs, ctx) = picker_app();
        app.execute("DIAZ CAST");
        let pane = app.pick_pane().unwrap();
        let i = pane;
        while reqs.try_recv().is_ok() {}
        let esc = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
                physical_key: None,
            }],
            ..Default::default()
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 320.0));
        ctx.run_ui(esc, |ui| {
            let _ = app.render_content(ui, i, rect, true);
        })
        .textures_delta
        .clear();
        assert_eq!(app.pick_pane(), None, "Esc closes the picker");
        assert!(app.status.contains("Cancelled"));
        while let Ok(req) = reqs.try_recv() {
            assert!(
                !matches!(req, crate::worker::Request::Series { .. } | crate::worker::Request::TrackEntity { .. }),
                "no command ran after Esc"
            );
        }
        assert!(app.focus_cmd, "the command line takes focus back");
    }

    #[test]
    fn pick_rewrites_are_capped_after_three_rounds() {
        let (mut app, mut reqs, _) = picker_app();
        app.execute("DIAZ CAST");
        let pane = app.pick_pane().unwrap();
        let id = app.panes[pane].id;
        if let PaneKind::EntityPick(p) = &mut app.panes[pane].kind {
            p.rounds = PICK_ROUNDS;
        }
        while reqs.try_recv().is_ok() {}
        app.accept_pick(id);
        let status = app.status.clone();
        assert!(status.contains("capped"), "unexpected status: {status}");
        let p = pick_of(&app);
        assert_eq!(p.rounds, PICK_ROUNDS, "the pane never rewrites past the cap");
        assert!(reqs.try_recv().is_err(), "nothing ran at the cap");
    }

    // ---- named presets (DR-3.41) ----------------------------------------------------------

    /// The pane kinds a preset docks, in document order.
    fn preset_kinds(app: &TerminalApp) -> Vec<std::mem::Discriminant<PaneKind>> {
        app.panes.iter().map(|p| std::mem::discriminant(&p.kind)).collect()
    }

    #[test]
    fn named_presets_apply_in_order_round_trip_and_render() {
        let expected: [&[_]; 3] = [
            &[discriminant_wl(), discriminant_chart(), discriminant_feed(), discriminant_alrt(), discriminant_feed()],
            &[discriminant_wl(), discriminant_feed(), discriminant_chart()],
            &[discriminant_wl(), discriminant_chart(), discriminant_feed(), discriminant_chart()],
        ];
        for (i, (token, label, _)) in TerminalApp::named_presets().into_iter().enumerate() {
            let (mut app, mut reqs, ctx) = picker_app();
            while reqs.try_recv().is_ok() {}
            assert_eq!(app.apply_named_preset(token), Some(token));
            assert_eq!(app.layout_name, label, "{token} runs under its own label");
            let want: Vec<_> = expected[i].to_vec();
            assert_eq!(preset_kinds(&app), want, "{token} docks the mocked panes in order");

            // The document round-trips through the saved layout (same kinds in order).
            let doc = app.current_doc();
            let doc_kinds: Vec<_> = doc
                .panes
                .iter()
                .map(|sp| std::mem::discriminant(&PaneKind::from_settings(sp.settings.clone())))
                .collect();
            assert_eq!(doc_kinds, want, "{token} round-trips through PaneSettings");

            // Each pane renders one frame without panicking.
            for pi in 0..app.panes.len() {
                let input = egui::RawInput::default();
                ctx.run_ui(input, |ui| {
                    let _ = app.render_content(
                        ui,
                        pi,
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 320.0)),
                        true,
                    );
                })
                .textures_delta
                .clear();
            }
            // The preset kicked off its pane loads: a series or projection per CAST/PROJ
            // pane, plus the alert refresh for the morning briefing.
            let mut loads = 0;
            while let Ok(req) = reqs.try_recv() {
                if matches!(req, Request::Series { .. } | Request::Projection { .. } | Request::SabrAlerts) {
                    loads += 1;
                }
            }
            assert!(loads >= 1, "{token} never kicked off its pane loads");
        }
    }

    fn discriminant_wl() -> std::mem::Discriminant<PaneKind> {
        std::mem::discriminant(&PaneKind::Watchlist)
    }
    fn discriminant_feed() -> std::mem::Discriminant<PaneKind> {
        std::mem::discriminant(&PaneKind::Feed { filter: crate::congress_view::FeedFilter::default() })
    }
    fn discriminant_alrt() -> std::mem::Discriminant<PaneKind> {
        std::mem::discriminant(&PaneKind::Alerts)
    }
    fn discriminant_chart() -> std::mem::Discriminant<PaneKind> {
        std::mem::discriminant(&PaneKind::Chart(Box::new(crate::chart_view::ChartState::new(
            0,
            "AAPL",
            Default::default(),
        ))))
    }
}
