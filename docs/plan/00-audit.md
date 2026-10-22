# 00 — Codebase audit: terminal-rs → sabr-rs reuse map

Audited at upstream commit `b81f40b` (michaelshimeles/terminal-rs, 2026-10-20). Method: every crate read file by
file (five parallel read-only passes: data, chart, core, app, periphery), LOC and test counts measured with
`wc -l`/grep, build and smoke runs executed on the development machine (section 6). Line numbers are as of
`b81f40b` and are hints, not contracts.

Effort scale (solo developer + coding agents): XS < ½ day, S ½–1 day, M 2–4 days, L 1–2 weeks, XL > 2 weeks.
LOC are `wc -l` of `.rs`/`.wgsl` including inline tests; per-file figures marked ≈ are ±5 lines.

Verdict vocabulary: **keep** = reuse as-is or with trivial edits; **adapt** = keep the structure, re-enumerate or
re-type the contents; **replace** = the structural role survives, the code does not; **remove** = no baseball analog.
Replacement components are *working names*; crate boundaries and names are decided in Phases 3–4.

## 1. Shape of the codebase

| Crate / dir | LOC | Files | `#[test]` | Role today |
|---|---|---|---|---|
| `app/` (bin `terminal-rs`) | 19,815 | 28 | 80 | egui shell: command line, docking, panels, worker thread, prefs, themes, all views |
| `crates/core` (`tr-core`) | 5,849 | 17 | 72 | `Bar`/`Series`/`Timeframe`/`Security`/catalog, Black-Scholes, synthetic sample generator, 151-study indicator engine |
| `crates/broker` (`tr-broker`) | 4,443 | 11 | 45 | SnapTrade client, order flow, demo paper book, secrets store, portfolio model |
| `crates/congress` (`tr-congress`) | 2,693 | 12 | 25 | Senate eFD + House Clerk ingest, SQLite store, refresh, tracked-politician alerts |
| `crates/chart` (`tr-chart`) | 2,612 | 13 (11 `.rs` + 2 `.wgsl`) | 28 | wgpu renderer, LOD pyramids, viewport/scale math, drawings, headless render |
| `crates/data` (`tr-data`) | 2,614 | 10 | 31 | `MarketData` trait, provider router, Yahoo/Binance/Coinbase/Treasury/FRED/Alpaca/RSS, sample provider, SQLite bar cache |
| `crates/macos` (`tr-macos`) | 292 | 2 | 1 | NSToolbar/vibrancy/accent/Sparkle; no-op stubs off macOS |
| `scripts/`, `site/`, `.github/`, `docs/` | — | — | — | macOS bundling/signing/Sparkle scripts; SvelteKit download site; CI (macOS matrix + Linux job); GOAL.md + TradingView UX analysis |
| **Total** | **38,318** | **93** | **282** | |

Shared vocabulary that every layer depends on (the real coupling, more than any crate boundary):
`tr_core::Bar {ts,open,high,low,close,volume}` (bar.rs:7-14), `tr_core::Timeframe {M1..MN1}` (timeframe.rs:11-22),
`tr_core::Security {ticker, class: AssetClass}` with Bloomberg yellow keys (security.rs:111-134). These three types
flow trait → cache → worker → chart → command parser → layout JSON. Replacing them is the one cross-cutting change;
everything else is local.

## 2. Starting hypotheses — confirmed or refuted from source

| # | Hypothesis | Verdict | Evidence |
|---|---|---|---|
| H1 | keep the GPU chart engine | **Confirmed, with an extension list** | x-axis is bar *index*, not time (viewport.rs:1-4; `LinePyramid::reduce` sets x=i+0.5, lod.rs:147-148) — ideal for PA/game-indexed series. Finance is confined to `BarPyramid`/`GpuBar` (lod.rs:10-22, 47-125), `vs_candle`/`vs_volume` (shader.wgsl), `ChartType`, `SeriesGpu` (scene.rs:34-80) and the main-pane path that is typed on both of them — `PricePane {series: &SeriesGpu, chart_type: ChartType, …}`, `visible_extent`, `autoscale`, `price_pane` (scene.rs:100-233; `price_pane` reads `series.bars.pyramid` even for `ChartType::Line`) — plus `scale::time_ticks/label/crosshair_label` (scale.rs:104-135), `drawings::Tool::Fib`/`magnet`. Overlay/sub-pane machinery (`Overlay`, `overlay_cmds`, `sub_pane`) is independent of them. Missing for sabr: filled band between two series (fan chart), min/max band LOD, index/categorical tick labels, per-point colour (pitch scatter); marker LOD is lossy when markers collide (§4.1). |
| H2 | keep docking/layouts | **Confirmed** | `dock.rs` (609 LOC, pure tree, 7 tests), `layout.rs` presets, `layouts.rs` `Store`/`LayoutDoc`/`write_atomic`, `workspace.rs` drag/compass/pop-out (:68-280, :981-1511) carry no finance types. Finance in `layouts.rs` is the `PaneSettings` variants (:58-117) plus three constructors: `LayoutDoc::repair` inserts a `Watchlist` pane into an empty document (:205-214), `default_workspace` hard-codes `AAPL US Equity`/`Feed`/`Wei`/`Watchlist` (:235-243), `chart_settings` seeds timeframe, chart type, `default_indicators()`, volume and Congress markers (:264-276); plus the preset fillers (`workspace.rs:716` `["CONG","WL","WEI","GC","NEWS","BOOK"]`). |
| H3 | keep themes | **Confirmed** | `themes.rs` (8 built-ins, TOML/JSON store), `theme.rs` palette: generic except `party_color` (theme.rs:139-159) and `candle_up/candle_down` fields. |
| H4 | keep command parser | **Confirmed as mechanism, refuted as table** | `Command`/`Function`/`parse`/`hint`/`describe` structure (command.rs:132-430) is reusable; the `Function` enum (44 variants, :210-278), fallback resolution (bare word → `Timeframe` → `Range` → indicator → `Security::resolve_tokens` with yellow keys, :361-413) is entirely finance. Verdict: adapt. |
| H5 | keep keyboard model | **Confirmed** | `prefs.rs` `Chord`/`Keymap`/`Action` (52 actions) with Ctrl↔Cmd mapping (:475-570) and rebinding UI; the `Action` list is finance and gets re-enumerated. |
| H6 | adapt `tr_data::MarketData` | **Confirmed as architecture, refuted as interface** | `#[async_trait] trait MarketData: Send + Sync` (lib.rs:209-282), `Arc<dyn>` provider, `Fallback<P>` decorator, `from_env` switch, paging-by-`end` contract, `clip` — reusable patterns. `Fallback`'s guarantee is narrower than its comment: a failed *older-page* request (`end.is_some()`) is not replaced with sample bars (:309-310), but a failed *first* page is served synthetically (:311-314) and if the primary then recovers, `worker.rs:541-551` prepends the real older page to the synthetic series without any provenance check — mixed real/synthetic series are possible and nothing in `Response::History` marks them. 7 of 8 methods (`quotes` w/ `prev_close`, `live`, `search`, `news`, `options`, `order_book`, `yield_curves`) and every type are finance; `history` returns `Vec<Bar>`. The trait is rewritten; its shape is copied, the fallback semantics get a provenance flag. |
| H7 | adapt the 151-study engine | **Confirmed for the engine, refuted for the studies** | 151 confirmed (registry.rs:434-1165 catalog literal, counted mechanically). Registry plumbing (`Def/Param/Plot/Tone/Output/Spec`, `Spec::parse("BB 20 2.5")`, serde, `all()/find()`) and `stats.rs` (O(1) rolling `windows` engine, variance/zscore/percent_rank/linreg) are domain-neutral. 66 studies read only the close column and nothing else; 85 need OHLC/volume, a session calendar or US-market annualisation (table in §4.3). `Input` exposes only `Ohlcv` + `Timeframe` (registry.rs:103-107); builders and `Input::c()` are private — a sabermetric study must live inside registry.rs unless builders are made `pub`. |
| H8 | adapt the `TRACK` alert system | **Confirmed** | Generic skeleton: `refresh()` orchestration + `store()` (refresh.rs:100-195: upsert entity → insert record → `is_tracked` → `insert_alert` dedup → report), `tracked`/`alerts`/`meta` tables (db.rs:55-72, 425-484), worker interval scheduler (worker.rs:360-400, `TR_CONGRESS_REFRESH_MIN`), `Response::NewAlerts` → toasts (app.rs:1066-1069), `TRACK`/`UNTRACK` commands (command.rs:313-314). Entity = politician → player. |
| H9 | adapt link groups | **Confirmed** | `Link {Red,Orange,Green,Blue,Purple}` (layouts.rs:24-54), `broadcast`/`set_link` (workspace.rs:495-504) generic; `set_pane_symbol` (:457) normalises through `tr_data::security(symbol).key()` → must switch to the player/team resolver. |
| H10 | adapt the offline sample provider | **Confirmed as pattern, refuted as data** | `SampleProvider` (sample.rs) is a seeded GBM generator with simulated 400 ms live ticks; `TR_PROVIDER=sample` gates offline UI (panels.rs:263, broker.rs:115). sabr's offline mode must serve a **bundled real dataset**, so the generator is removed and only the env switch, `Fallback`, and determinism contract are kept. |
| H11 | replace all finance data providers | **Confirmed** | `yahoo.rs` 522, `crypto.rs` 499, `treasury.rs` 264, `alpaca.rs` 134, `rss.rs` 97, `markets.rs` 164 router: all symbol-mapping/interval tables are finance. Reusable *patterns*: page-walk loops (alpaca.rs:75-111), per-year file cache with TTL on the current year (treasury.rs:124-150 — the model for per-season files), WS reconnect/backoff (crypto.rs ~340-365, only if a live game feed is ever wanted), fixture-parsing unit tests. |
| H12 | replace the Congress ingestion pipeline | **Confirmed for sources, refuted for skeleton** | `house.rs`, `senate.rs`, `roster.rs` XML parsers, `performance.rs`, `model.rs` enums/`AmountRange`, `trades` columns: replace. `Source` trait (RPITIT, refresh.rs:21-27) is a five-method *congressional* contract (`rosters`, `house_index`, `house_pdf`, `senate_reports`, `senate_ptr`) and `refresh()` (:100-138) hard-wires `refresh_house`/`refresh_senate`; the app never injects a `Source` — `worker.rs:384-391` constructs `HttpSource` inside `congress_refresh`, which both the scheduler (:373) and `Request::RefreshCongress` (:642) call. So the adaptable parts are `RefreshConfig`, the `refresh()`/`store()` orchestration shape, the discovery pattern (`known_filing_ids` → newest-first → truncate → `buffer_unordered`), `Db` WAL pragmas and dup-merge-by-canonical-id (db.rs:214-249): the trait, `refresh()`'s body and the worker's source construction are rewritten together. Fixture-backed `Source` test (tests/refresh.rs:19-47) is the offline-ingest test pattern to copy (it calls `refresh()` directly, which is why it works without the worker). |
| H13 | replace options | **Confirmed** | `tr_core::options` (Black-Scholes, 141 LOC), `OptionChain`/`OptionContract` types, `Markets::options` routing, `options_view.rs` (377), `simulated_chain`: remove. Only the *panel idea* (a table of strikes × greeks beside the chart) survives as the projection-distribution panel. |
| H14 | replace SnapTrade | **Confirmed** | `snaptrade.rs` 579, `trading.rs` 946, `demo.rs` 579, `sample.rs` 241, `secrets.rs` 316, app `trading.rs` 2,123, `broker.rs` 799, `portfolio_view.rs` 1,211, `demo.rs` 273: remove. Adapt only `model.rs` `Position`/`Portfolio::totals` closure-repricing (:112-146, 330-393) as the roster/payroll base and `portfolio_view.rs:113-226` column-fitting helpers (`Col/fit/span/cell/header`). |
| H15 | remove the macOS crate | **Confirmed, cheap** | `tr-macos` is an **unconditional** dependency (app/Cargo.toml:25) compiled on Linux through no-op stubs; `chrome.rs:23,25` read `tr_macos::TITLEBAR_HEIGHT/TRAFFIC_LIGHTS_WIDTH` inside `const` expressions (fold to `34.0`/`0.0`); other call sites app.rs:508, 548, 864-865, 1407-1437, 1457, 1468, 1471; `cfg!(target_os="macos")` branches at main.rs:36,43, prefs.rs:475-570, chart_view.rs:1057, broker.rs:161, chrome.rs:29,543, settings_view.rs:250,570, workspace.rs:968, worker.rs:314 fold to the Linux arm. Also remove `[target.'cfg(target_os = "macos")'.dependencies] raw-window-handle`, `scripts/*.sh` (all macOS), `site/`, `app/assets/AppIcon.icns`, Sparkle updater, CI macOS matrix. |
| H16 | remove anything with no baseball analog | **Confirmed list** | `WEI` world markets + `GC` yield curve (`markets_view.rs` 381, `YieldCurves` types, `catalog::treasury_curve`), order book (`OrderBook`, `BookPane`, `simulated_book`, `OB` polling at 1 s), news (`NewsPane`, Yahoo/CoinDesk RSS — no key-free baseball news source; MLB transactions feed is the analog and lives in the events ingest), Yahoo symbol search, buy/sell pills (widgets.rs:354), DEMO/LIVE badge, `Settings › Brokerage/Trading` tabs. |

## 3. Crate-by-crate reuse map

| Crate | Verdict | Effort | Baseball component that takes its place (working name) | What survives |
|---|---|---|---|---|
| `tr-chart` | **keep + extend** | S–M | same crate; add fan-chart band primitive (`vs_band` + band pyramid with min/max merge), index/categorical tick labels, optional per-point colour/size for pitch scatter, collision-safe marker LOD | `gpu.rs` renderer (buffer cache, uniform ring, integer+fraction f32 split), `LinePyramid` + Line/Area/Hist/Dots/Steps pipelines, `Viewport`, `PriceScale` (rename only), `scale::{nice_step, price_ticks, format_*}`, `Headless`, `Overlay`/`overlay_cmds`/`sub_pane`, benches, 28 tests. Adapted: `PricePane`/`visible_extent`/`autoscale`/`price_pane` (typed on `SeriesGpu` + `ChartType`, scene.rs:100-233). Removed: `BarPyramid`/`GpuBar`, candle/volume shaders, `ChartType::{Candles,Bars}`, `Timeframe` coupling in `scale.rs`, `Tool::Fib`/`magnet`. |
| `app/` generic shell (≈14.2K LOC) | **adapt** | L | sabr terminal shell: `PaneKind`/`PaneSettings`/`Function`/`Request`/`Response`/`Action` re-enumerated for players/teams/games; command resolver for player/team ids; `periodic` polls re-timed to baseball cadence (daily, not 1 s) | keep as-is: `anim.rs` 243, `dock.rs` 609, `layout.rs` 65, `palette.rs` 446, `theme.rs` 304, `themes.rs` 777, `theming.rs` 169, `widgets.rs` 379 (minus `buy_sell_pills`). adapt: `layouts.rs` 573 (`PaneSettings` + `repair`/`default_workspace`/`chart_settings`), `app.rs` 1,620, `workspace.rs` 1,649, `chrome.rs` 970, `worker.rs` 947, `command.rs` 746, `chart_view.rs` 1,472, `prefs.rs` 897, `settings_view.rs` 719, `palette_view.rs` 682, `hotkeys_view.rs` 401, `panels.rs` 461, `main.rs` 75. |
| `app/` finance views (≈5.6K LOC) | **remove** (patterns reused) | — | projection-distribution panel (← options_view layout idea), roster/payroll panel (← portfolio tables), events feed/profile views (← congress_view structure), player search (← SECF search) | `app/trading.rs` 2,123, `portfolio_view.rs` 1,211 (keep ≈110 LOC table helpers), `broker.rs` 799, `congress_view.rs` 447, `options_view.rs` 377, `markets_view.rs` 381, `demo.rs` 273 → deleted |
| `tr-core` indicator engine (≈4.5K LOC, `indicators/`) | **adapt** | M | parameterized sabermetric study registry: same `Def/Spec/Param/Plot/Tone/Output` contract; `Input` widened from `Ohlcv` to named metric columns + event-count windows; generic rolling studies kept (final list is a grill decision), the 85 OHLC/volume/session/annualisation studies removed, sabermetric studies added | `registry.rs` plumbing (≈430 of 1,357 lines), `stats.rs` 372, `ma.rs` 491 (minus VWMA/EVWMA/FRAMA), `mod.rs` 209 helpers, close-only momentum/volatility functions; catalog-wide property tests (registry.rs:1242-1316) as the test pattern |
| `tr-core` vocabulary (≈1.3K LOC: `bar.rs`, `timeframe.rs`, `security.rs`, `catalog.rs`, `sample.rs`, `options.rs`) | **replace** | M | entity/series/window model: `Entity` (player / team / game / matchup; MLBAM-id keyed — numeric ids fail `looks_like_ticker`'s alpha-first rule today), `Window` (event-count: PA/BF/pitches/games; calendar: season), `Series` of named metric points with as-of dates; entity catalog loaded from the bundled dataset instead of a static `Listing` table | `Series::normalize/lower_bound/merge` ideas; `catalog::search` scoring (ticker > prefix > name-word > substring, catalog.rs:305) as the player-search ranking |
| `tr-data` | **adapt shape, replace providers** | M–L | baseball data layer: one object-safe async trait (history/entities/search/events), `Fallback` decorator, `from_env` switch, bundled-dataset provider as the offline default, SQLite cache keyed by entity + window + sequence (not `ts` — per-PA events collide on timestamps, cache.rs:26-33 PK `(symbol,tf,ts)`), live sources decided in Phases 2–3 | `lib.rs` trait shape/`Fallback`/`clip`/`from_env` (≈250 lines), `cache.rs` open/WAL/upsert/`prepare_cached` pattern, per-season file cache pattern (treasury.rs:124-150), fixture-parsing tests. Removed: yahoo/crypto/treasury/alpaca/rss/markets (≈1.7K), `simulated_chain/book`, `tests/live.rs`. |
| `tr-congress` | **adapt skeleton, replace sources** | M | events ingest: transactions + injuries (IL stints) per player/team as the chart-marker + `TRACK <player>` alert source; `Source` trait re-parameterised (`transactions(since)`, `injuries(since)`), `events` table replaces `trades`, entity = player (MLBAM id) with dup-merge | `refresh.rs` ≈250 lines, `db.rs` `tracked`/`alerts`/`meta` + pragmas ≈150, `roster.rs` `normalize_name`/`find_by_name`, worker scheduler, toast wiring, fixture-backed `Source` test. Removed: `house.rs` 341, `senate.rs` 273, roster XML parsers, `performance.rs` 135, PDF/XML/zip/scraper deps, real-filing fixtures (~500 KB PDFs). |
| `tr-broker` | **remove** (one pattern reused) | S (for the rewritten part) | roster/payroll model: a `Position`-like row (player, contract $, projected WAR, surplus) and a `Portfolio::totals(filter, price_of)`-like aggregation with closure-based projection repricing | the *shape* of `model.rs:112-146, 330-398` only — its fields (`units`, `multiplier`, `cash`, `buying_power`, `cash_equivalent`) and branches (trusting broker-reported totals) are brokerage semantics, so the code is rewritten, not retyped; everything else deleted: `snaptrade.rs`, `trading.rs`, `demo.rs`, `sample.rs`, `secrets.rs` (no API keys on the default path → no secret store), hmac/sha2/base64/security-framework deps, mock-server tests |
| `tr-macos` | **remove** | XS | — (Linux-only chrome: fixed toolbar height, no vibrancy, no accent query, no Sparkle) | nothing |
| `scripts/`, `site/`, CI macOS jobs, `AppIcon.icns` | **remove** | XS | Linux CI job only (Phase 8 wires Arch-aware instructions) | `ci.yml` Linux job steps as the template |
| `docs/` | **keep** `analysis/` (UX research record); archive `GOAL.md` as upstream history | XS | `docs/plan/`, `docs/agents/` | |

Disposition of the 38,318 LOC by whole-file verdict (an adapted file counts fully as adapt, including the parts it
keeps; inline tests travel with their file; `docs/agents/codebase-map.md` lists every file): keep 6,271 · adapt 15,992 ·
replace 1,327 · remove 14,728. Whole-file deletions alone remove ≈ 14.7K LOC before any baseball code is written;
the finance parts of the adapted files (view arms, provider methods, 85 studies) come out on top of that.

## 4. Crate detail

### 4.1 `tr-chart`

Public API the app uses (scene.rs): `SeriesGpu::build(&[Bar])`, `line_ref(&[f64])`, `marker_ref`, `Overlay {line, color,
kind: LineKind, scale}`, `PricePane {series: &SeriesGpu, chart_type: ChartType, show_volume, overlays, viewport, height_px, log,
manual_scale, style}` → `price_pane()` / `sub_pane(overlays, viewport, height_px, fixed, style)` → `PaneOutput {cmds,
scale, level, vol_max}`; `gpu::ChartRenderer::{new, prepare, paint}`, `ChartCallback(Arc<Frame>)`; `Viewport`,
`PriceScale`; `scale::{price_ticks, time_ticks}`. Call sites: chart_view.rs:10-14, 754-938; worker.rs:402-404 maps
`registry::Plot {Line,Hist,Dots,Steps}` → `LineKind`; main.rs:63 installs the renderer into eframe
`callback_resources`; markets_view.rs:191 reuses `price_ticks` for a CPU-painted plot. The main-pane path is not
finance-free: `PricePane` owns a `SeriesGpu` and a `ChartType` (scene.rs:100-111), `visible_extent`/`autoscale`
(:135-177) size the scale from the bar pyramid and the volume overlay, and `price_pane` (:179-233) reads
`series.bars.pyramid` and emits `DrawCmd::Volume`/`Candles` before dispatching on `ChartType` — removing
`SeriesGpu`/`ChartType` means rewriting these four, keeping only `Overlay`/`overlay_cmds`/`sub_pane` as-is.

Capability check for sabr visuals:

| Need | Status | Evidence |
|---|---|---|
| Line / step / area series on an index axis | present | `shader_line.wgsl` `vs_line/vs_steps/vs_area`; `Overlay.kind` |
| Several series in one pane, sub-panes with fixed range (0–100 style) | present | `overlay_cmds` scene.rs:210-258; `sub_pane` :281-316 |
| Horizontal reference levels | CPU only | app paints `def.levels` with egui `hline` (chart_view.rs:836-841) |
| Filled band between two series (percentile ribbon) | **missing** | `vs_area` fills to pane bottom (shader_line.wgsl:62-77); points are `array<vec2<f32>>`; needs a `vs_band` reading (lo,hi) and a band pyramid with min/max merge (`LinePyramid::reduce` lod.rs:139-154 keeps one point per pair) |
| Fan chart = N nested ribbons | missing | same primitive, drawn N times |
| Scatter with arbitrary x (pitch location) | missing | `vs_dots` uses x as bar index (shader_line.wgsl:122-133); `LinePyramid` fields are `pub`, so a custom `{data, offsets:[0]}` can bypass `line_ref`; size fixed at `thickness_px*1.25`; no per-point colour (one `u.color` per `DrawCmd`) |
| Index / categorical tick labels (PA #, game #) | missing | only `time_ticks(times:&[i64], …, tf: Timeframe)` scale.rs:135; `Viewport` itself is index-based |
| Markers surviving zoom-out | **partial** | `build_markers` (lod.rs:143-145) keeps the earlier point of a pair only when the later one is NaN; two finite markers in the same LOD group collapse to the later one (probe: `build_markers(&[5.0, 7.0])` reduces to `[7.0]`), so sparse markers survive and adjacent ones are silently dropped — test `marker_pyramid_keeps_sparse_points` (lod.rs:266-280) asserts exactly that. Dense event streams (per-PA, per-pitch) need a count-preserving marker LOD. |
| Hit-testing GPU markers | app-side | chart_view.rs:1311-1360 geometric test → hover card; keep this pattern |
| Log y, manual y drag | present | `PriceScale::{stretch, pan_px}` |

Tests: 24 CPU unit tests (lod 5, viewport 4, scale 3, drawings 3, scene 9) + `tests/render.rs` 4 headless pixel tests
(`Headless::new(800,400)`, skip silently without a wgpu adapter; `TR_DUMP_DIR` dumps PPMs). Benches: criterion
`frame_1M_bars_1600x900` × 3 zoom levels; `examples/framebench.rs` p50/p99 over 3 × 300 frames (8.33 ms budget).

Gotchas: `gpu.rs:222-224` concatenates shaders by splitting `shader.wgsl` at the literal comment `// ---- bars` (shader.wgsl:93);
`Uniforms` layout is positional across `gpu.rs` and both WGSL files; `overlay_cmds`/`sub_pane` take the index length
from `overlays.first()` (scene.rs:236, 282); `SLOTS=4096` draw commands per frame (gpu.rs:17); GPU frame ids are `chart_id*8` for the price pane and `chart_id*8 + 1 + k` for sub-panes (chart_view.rs:33-35, 820, 842; `MAX_SUB_PANES = 6`);
`Frame.frame_no` must equal egui's `cumulative_frame_nr()`; mutating a series needs a new `key` or bumped `version`.

### 4.2 `app/`

Extension points (the "add a feature" checklists an agent follows today; carried into `docs/agents/extension-points.md`):

- New panel kind: `PaneKind` variant (app.rs:46-77) + its six match fns (:165-330: `tab_title`, `code`, `symbol`,
  `follows_symbol`, `settings`, `from_settings`) → `PaneSettings` variant (layouts.rs:58-117) → `render_content` arm
  (panels.rs:34-116) → `init_kind` (workspace.rs:368) if it loads data; `set_pane_symbol` (:457) if symbol-driven →
  `panel_from_spec` word (:656) → optional preset filler (:716) → `prefs::Action` + `Keymap::default` (:570) +
  `run_hotkey` (app.rs:1280) + `palette_view::COMMANDS` (:23-93) → test `pane_settings_round_trip_through_kinds`
  (app.rs:1605).
- New function code: `Function` variant (command.rs:210-278) → `parse_function` arm (:284-379; fallback order
  :361-376 matters) → `describe` (:430, powers the live hint) → `dispatch`/`apply_with_symbol` (app.rs:748-926;
  symbol-prefixed form needs a `Command::SymbolFunction` arm :751-782) → `palette_view::COMMANDS` → HELP text in
  `hotkeys_view.rs` → tests `functions`/`hints`.
- New worker request: `Request` variant (worker.rs:29-126) → `Response` variant (:200-280) → `handle` arm (:482-699;
  each request is its own tokio task over `Shared`) → `drain` arm (app.rs:935-1085) → sender in `periodic` (:1333)
  or a pane method. Series generations are guarded by `claim_series/store_series` (:742-770).

Finance coupling that dangles when the views are deleted (must be cleaned in the same change): `PaneKind`
variants `Feed/Ticker/Profile/Politicians/Alerts/Wei/Curve/News/OrderBook/Options/Portfolio`; `PaneSettings`
mirror; `dispatch`/`drain`/`periodic` arms (quotes 20 s, WEI, curves 600 s, news 300 s, options 30 s, book 1 s,
congress latest 60 s); `apply_preset` fillers; `render_content` arms; `run_actions` (:705) over
`congress_view::Action`; `hotkey_applies/run_hotkey` (:1266-1331); `palette_view::COMMANDS`; `chrome.rs` sidebar
portfolio rows and `compact_money` (:934); `SettingsTab::{Brokerage,Trading}`; `TerminalApp` fields
`broker/trading/demo/curves/latest/politicians/alerts/quotes/watchlist`; `DEFAULT_WATCHLIST` (app.rs:369-381);
`theme::party_color`; `Request::{Portfolio,Broker*,Trade,Options,OrderBook,News,YieldCurves,Markers,Latest,
TickerTrades,Profile,SearchPoliticians,Track,Alerts,MarkAlertsSeen,RefreshCongress,Quotes,Search}`;
`PanelDefaults {chart_type, timeframe, show_volume, congress_markers, book_depth}` (prefs.rs:177-183).

Time/OHLC assumptions in the shell: `Range {D1,D5,M1,M3,M6,Ytd,Y1,Y5,All}` and `Range::timeframe()`/`span_secs`
(command.rs:141-195), `chart_view.rs` `index_of_ts`/`ts_of_index` extrapolate by `tf.seconds()` (:488-510),
`apply_range` (:246), `chart_type` chosen by `AssetClass::Govt` (:126, :202), `show_volume` (:133), legend `Vol`
(:1107), `sec.decimals()` (:814, :921, :952, :1094), trade buttons gated on Equity/Etf (:969), congress markers
(:931-935, :1290-1364). `worker.rs` history is `Arc<Vec<Bar>>`, `build_gpu` → `SeriesGpu::build`,
`align_compare` (:451) aligns on `ts`/`close`, `default_limit(tf)` (:467), `TR_HISTORY_LIMIT` (:478).

Persisted files and formats: `settings.json`, `prefs.json` (v3 with migrations, prefs.rs), `layouts/*.json`
(`LayoutDoc {version, name, dock: Node, panes: Vec<SavedPane>, focus, sidebar_side, theme}`), `themes/*.toml|json`,
`congress.sqlite3`, `bars.sqlite3`, `demo-account.json`, `trading-access.json`, palette recents. Data dir on Linux:
`$TR_DATA_DIR` else `$XDG_DATA_HOME/terminal-rs` else `~/.local/share/terminal-rs` (worker.rs:309-321). Indicator
`Spec` serialises as its label and preserves unknown codes on load (registry.rs:275-280) — persisted finance codes
would silently survive a cutover unless the layout version is bumped and migrations dropped.

Tests: 80 unit tests, all `#[cfg(test)]` (dock 7, palette 6, anim 6, command 9, layouts 5, prefs 5, themes 5,
worker 4, portfolio_view 3 headless-egui, …); no integration dir, fixtures or mock servers. Two use
`egui::Context::run_ui` headless.

### 4.3 `tr-core`

Indicator contract (registry.rs): `type Compute = fn(&Input<'_>, &[f64]) -> Vec<Output>`; `Def {code, name, aliases,
category: Category (8 fixed variants), params: Vec<Param {name, default, min, max, int}>, pane: bool, range:
Option<(f64,f64)>, levels: &[f64], multi: bool, compute}`; `Output {name, values: Vec<f64>, plot: Plot
{Line,Hist,Dots,Steps}, tone: Tone {Main,Signal,Dim,Up,Down,Series(u8)}}`; `Input {bars: &[Bar], tf: Timeframe,
d: Ohlcv}`; `Spec {code, params}` with `parse(head, rest)`, `label()`, serde as label string. Invariants enforced
only by tests (registry.rs:1172-1316): codes uppercase/unique/not a catalog ticker, categories contiguous and
non-empty, `range`/`levels` only on panes, overlays never `Plot::Hist`, `values.len() >= bars.len()`, outputs not
entirely blank and never infinite, values inside `range`, no panics on empty/1/2/flat inputs (:1300-1316). **Not**
tested: the NaN warm-up prefix — "positions without enough history hold `f64::NAN`" is a documented convention
(indicators/mod.rs:1-4) that the sweep never checks, so a study returning finite values during warm-up passes.

| Category | Studies | Reads only the close column | Needs OHLC/volume, a session calendar or annualisation |
|---|---|---|---|
| Moving averages | 23 | 20 | 3 (VWMA, EVWMA, FRAMA) |
| Bands | 8 | 3 (BB, ENV, SEB) | 5 (KC, DONCHIAN, ACCBANDS, STARC, VWAPB) |
| Momentum | 43 | 27 | 16 |
| Trend | 22 | 3 | 19 |
| Volatility | 16 | 4 (BBW, PCTB, ULCER, RVI) | 12 (ATR, NATR, TRANGE, ADR, HV, PARKINSON, GK, RSVOL, YZ, CHV, MASS, SQUEEZE) |
| Volume | 23 | 0 | 23 |
| Statistics | 11 | 9 | 2 (HIGHEST, LOWEST) |
| Levels (pivots) | 5 | 0 | 5 |
| **Total** | **151** | **66** | **85** |

Counted mechanically from the catalog literal: an entry is "close-only" when its closure touches nothing but
`i.c()`; it is "dependent" when it reads `i.d` (OHLC/volume), `i.bars`, or `i.tf`/`periods_per_year` (HV is the
one close-reading study that annualises by timeframe: registry.rs:984-986 → volatility.rs:108-115, so identical bars
give HV(D1)=17.53 and HV(W1)=7.96). "Close-only" means input-column independence, not domain neutrality — the
surviving list is still a grill decision.

Session/annualisation hard-coding: `levels::session_key` (UTC day / month / year by `Timeframe`; used by the 5
pivots, VWAP and VWAPB), `vwap` resets per 86,400 s, `volatility::periods_per_year` assumes 252 × 23,400 s US
sessions (used by HV, PARKINSON, GK, RSVOL, YZ). Minimal plug-in for a new study today is
one line in `catalog()` using the private builders, with the metric arriving through `Bar.close` — i.e. the engine
cannot take a named metric column without widening `Input`.

Identity: `Security::parse/resolve_tokens/key` hard-code `"{} US Equity"` and classify unknown tickers as `Equity`;
`looks_like_ticker` requires alpha-first ≤ 14 chars (security.rs:242-248) so MLBAM ids like `660271` are rejected.
`catalog::Listing` carries provider symbols and `sample_level`; a test asserts every listing has a source
(catalog.rs:358-361). App gating on `AssetClass` at chart_view.rs:969, palette_view.rs:293, panels.rs:190,
portfolio_view.rs:235/262/733, trading.rs:531, workspace.rs:326/675.

Tests: 72 inline, golden values (Wilder RSI, Black-Scholes 10.4506), brute-force cross-checks, catalog-wide
property sweeps on a seeded random walk; criterion bench `indicators` (1M bars sma/ema/rsi/macd/bb/vwap, whole
catalog on 100k).

### 4.4 `tr-data`

Trait (lib.rs:209-282; `name` and `history` required, the other seven defaulted), `Provider = Arc<dyn MarketData>`, `Fallback<P>` (:287-379; first-page failures are served synthetically, older-page failures propagate — see H6 for the mixed-series consequence), `from_env` (:387-405: `TR_PROVIDER=sample`
→ sample with live ticks; `APCA_*` → Alpaca inside `Markets`; else `Fallback<Markets>`; installs rustls
`aws_lc_rs`), `security(symbol)` (:381), `clip` (:407-415: bars `< end`, ≤ `limit`). `Markets` routes on
`AssetClass` (markets.rs:40-181). `BarCache` (cache.rs): `bars(symbol, tf, ts, o,h,l,c,v) PRIMARY KEY(symbol,tf,ts)
WITHOUT ROWID`, WAL, upsert; `load(symbol, tf, limit)` has no `end` bound → cache serves first paint only, every
older page hits the network (worker.rs:541). `SampleProvider` regenerates `depth` bars per call (O(depth); with
`TR_SAMPLE_BARS=1000000` every page is a 1M-bar generation). Tests: 26 offline unit tests (fixture parsing, grouping, simulated books) + 5
network tests `#[ignore = "network"]` (`cargo test -p tr-data -- --ignored`).

### 4.5 `tr-congress`, `tr-broker`, `tr-macos`

Congress schema (db.rs:12-72): `politicians(id, name, chamber, party, state, district, bioguide, UNIQUE(chamber,name))`,
`filings(id 'house:<DocID>'|'senate:<uuid>', chamber, politician_id, filed_date, url, kind, status
'parsed'|'scanned'|'error', …)`, `trades(…, UNIQUE(filing_id,row_idx))` + 3 indexes, `tracked(politician_id)`,
`alerts(id, politician_id, filing_id UNIQUE, created_at, message, seen)`, `meta(key,value)`.
`RefreshConfig::incremental(db)` rescans 7 days back from `meta.last_refresh_date`; `refresh()` takes
`&tokio::sync::Mutex<Db>`; no cancellation token (loop lives until runtime drop); `trades` rows are deleted and
reinserted per filing (ids unstable). The worker constructs `HttpSource::new()` itself inside `congress_refresh`
(worker.rs:384-391) for both the scheduled loop (:373) and `Request::RefreshCongress` (:642) — there is no
injection point for another `Source`. Deps to drop with the sources: scraper 0.27, quick-xml 0.42, zip 9,
pdf-extract 0.12 (`house::pdf_text` wraps it in `catch_unwind` because it panics).

Broker: `Position {account_id, symbol, description, kind, units, price, cost_basis, multiplier, currency,
cash_equivalent, underlying}` + `market_value_at/pnl_at/pnl_pct_at`; `Portfolio::totals(filter, price_of)` →
`Totals {value, cash, buying_power, invested, cost, pnl}`. Global throttle statics (snaptrade.rs:47-50) make
`tests/throttle.rs` timing-sensitive (real sleeps: 22.03 s measured in §6). `secrets.rs` file backend writes 0700/0600 via tmp+rename.

macOS: pure objc2 behind `cfg(target_os = "macos")`; `TITLEBAR_HEIGHT = 38.0`, `TRAFFIC_LIGHTS_WIDTH = 76.0`;
`updater` loads Sparkle at runtime; off-macOS stubs return `false`/`None`; one test `stubs_are_safe_off_macos`.

## 5. Prior-art borrow map (methodology only; no code or runtime dependency — decision D0.1)

| Reference | Informs which replaced piece | What to borrow / where we will diverge |
|---|---|---|
| `saberkit` (Rust `saberkit-core`, Apache-2.0/MIT) | metrics layer replacing `tr_core::options` and the finance studies | rate/plus/minus families, Savant-style percentile ranks, `LeagueContext` season constants derived from Retrosheet (not FanGraphs Guts), "undefined is not an error" (null, never NaN/inf), "misconfiguration is an error", park factors on the 100 scale, IP-as-outs notation. Diverge: re-implemented inside sabr-rs (no dependency); constants provenance recorded the same way (SHA-256 of source files). |
| `statcast-lakehouse` (Python/TS) | data layer replacing `tr-data` providers; bundled sample dataset | MLB Stats API game-feed ingestion (`ingestion/mlb_client.py`), curated real-game Arrow slices as committed fixtures with exact game ids and selection rules, "one day per query", physics cross-checks pinned by tests. Diverge: no BigQuery/cloud; storage decided in the grill. |
| `sabr-jev` (Elixir) | evaluation protocol for the forecasting core; data licensing | pinned + checksummed Lahman inputs (CC-BY-SA-3.0 share-alike applies to derived data), frozen wOBA weights file, prospective evaluation with a Jan-1 cutoff, Brier vs baseline with reliability buckets, "no T+1 leakage: oracle joins are eval-only". Diverge: no LLM judgments; projections are statistical. |
| `bayes-horizon` (Python) | backtest harness, model registry, fan charts | point-in-time feature store with release-lag alignment, purged expanding-window walk-forward CV with embargo, quantile forecasts (p10/p50/p90), PICP/MIS coverage + pinball loss, run tracking as JSON with figures, frozen cross-workstream contracts. Diverge: Rust-native or sidecar runtime is a grill decision; quantiles rendered on the GPU engine instead of SVG. |
| `baseball-dashboard` (TypeScript) | events ingest replacing `tr-congress` sources; GAME/SPLT panels | MLB Stats API client shaping with gzip fixtures (schedule, boxscore, game feed, standings), career batter-vs-pitcher splits, bullpen workload, fail-soft sections per upstream endpoint, `docs/data-sources.md` operational limits. Diverge: desktop/offline-first, SQLite cache instead of edge fetch cache. |

## 6. Build & run on Arch (measured)

Measured 2026-10-20 on the development machine, branch `sabr` at `b81f40b`, working tree clean, cold `target/`
(406 crates compiled, crates.io index updated during the build). Every command exited 0; nothing in upstream was
patched to get there.

Machine: Arch Linux, kernel 7.2.8-9-omarchy, Hyprland (Wayland session, `WAYLAND_DISPLAY=wayland-1`, XWayland on
`DISPLAY=:0`), Intel Core Ultra 9 285K (24 threads), 30 GiB RAM, NVIDIA GeForce RTX 4070 Ti (nvidia 615.78.08) +
Intel Arrow Lake iGPU (Mesa 26.2.5). Installed: `vulkan-icd-loader`, `vulkan-intel`, `mesa`, `libxkbcommon`,
`wayland`, `libx11`, `libxcb`, `fontconfig`, `freetype2`, `openssl`, `sqlite`. Not installed: `xorg-server-xvfb`,
`vulkan-swrast`, `rustup`, `time` (GNU `/usr/bin/time`), `bc`.

| Command | Result | Wall |
|---|---|---|
| `rustc --version` / `cargo --version` | `rustc 1.99.1 (58cf8f9c3 2026-10-13) (Arch Linux rust 1:1.99.1-1)`, `cargo 1.99.1 (5f94df478 2026-08-27)` — Arch package, no rustup, no `rust-toolchain.toml` | — |
| `cargo build --release --workspace --all-targets` | `Finished release profile [optimized + debuginfo]`, 0 warnings; `target/` 3.5 GB; `target/release/terminal-rs` 234.7 MB (release profile keeps `debug = 1`) | 88.4 s (`1m 28s` reported by cargo) |
| `cargo test --release --workspace` | 277 passed, 0 failed, 5 ignored (all five are `tr-data/tests/live.rs` `#[ignore = "network"]`); 282 test fns total, matching the §1 count | 24 s, of which `tr-broker/tests/throttle.rs` alone took 22.03 s |
| `cargo clippy --release --workspace --all-targets -- -D warnings` | clean | 12 s |
| `cargo fmt --all -- --check` | clean | <1 s |
| `cargo test --release -p tr-chart --test render -- --nocapture` | 4 passed; log shows `adapter: "NVIDIA GeForce RTX 4070 Ti"` and no `no wgpu adapter; skipping` line — the pixel tests actually rendered | 0.67 s |

Per-binary test totals from `/tmp/sabr-test.log`:

| Test binary | passed | failed | ignored |
|---|---|---|---|
| `terminal-rs` unittests | 80 | 0 | 0 |
| `tr-broker` unittests / `tests/client.rs` / `tests/demo.rs` / `tests/throttle.rs` / `tests/trading.rs` | 22 / 3 / 15 / 1 / 4 | 0 | 0 |
| `tr-chart` unittests / `tests/render.rs` | 24 / 4 | 0 | 0 |
| `tr-congress` unittests / `tests/parsers.rs` / `tests/refresh.rs` | 11 / 13 / 1 | 0 | 0 |
| `tr-core` unittests | 72 | 0 | 0 |
| `tr-data` unittests / `tests/live.rs` | 26 / 0 | 0 | 0 / 5 |
| `tr-macos` unittests | 1 | 0 | 0 |
| doc-tests (6 crates) | 0 | 0 | 0 |

Smoke runs (`TR_PROVIDER=sample TR_CONGRESS_REFRESH_MIN=0 TR_SMOKE_SECS=6 TR_DATA_DIR=$(mktemp -d)
./target/release/terminal-rs`, exit 0 both times). wgpu enumerated three adapters in both runs: Vulkan/DiscreteGpu
`NVIDIA GeForce RTX 4070 Ti` (driver 615.78.08), Vulkan/IntegratedGpu `Intel(R) Graphics (ARL)` (Mesa 26.2.5-arch1.1),
Gl `NVIDIA GeForce RTX 4070 Ti/PCIe/SSE2`; eframe picked the NVIDIA Vulkan adapter both times.

| Run | Window backend (proof) | `window + GPU ready` | `smoke:` line |
|---|---|---|---|
| Wayland (session env as-is) | native Wayland — Hyprland `hyprctl clients -j` reports `"xwayland": false` for the `terminal-rs` window; no `winit::platform_impl::linux::x11` log lines | `238.9 ms (adapter: NVIDIA GeForce RTX 4070 Ti)` | `1418 frames in 6.0s, ui cpu 0.28 ms/frame, panes 4, layout "Default"` → ≈4.2 ms/frame wall (≈236 fps, uncapped `request_repaint`) |
| X11/XWayland (`env -u WAYLAND_DISPLAY`) | XWayland — `hyprctl` reports `"xwayland": true`; winit logs `x11::xdisplay: error setting XSETTINGS; Xft options won't reload automatically` (WARN) and `x11::window: Guessed window scale factor: 1.0833333333333333` | `291.2 ms (adapter: NVIDIA GeForce RTX 4070 Ti)` | `1426 frames in 6.0s, ui cpu 0.27 ms/frame, panes 4, layout "Default"` → ≈4.2 ms/frame |

Both logs are three lines long apart from the winit X11 lines: no panics, no wgpu validation errors, no
Wayland-specific warnings. Verdict for the brief's Arch/Wayland question: the upstream binary runs natively on
Wayland on this machine with no code change; the only platform difference observed is the X11 path's guessed
1.083× scale factor (the Wayland run logs no scale factor at all). The smoke run leaves `bars.sqlite3`, `congress.sqlite3`,
`layouts/Default.json` and `settings.json` (520 KB total) in `TR_DATA_DIR`; `prefs.json` is not written when nothing
changed.

Deviations from the plan's recipe: `/usr/bin/time` and `bc` are not installed, so wall times were taken with
`date +%s.%N` before/after; `tests/throttle.rs` took 22 s rather than the ≈4 s the periphery pass estimated
(the global throttle statics sleep for real — a fact for the broker removal, nothing to fix).

Arch package equivalents of the CI apt list (`libxkbcommon-x11-0 libxcb-xkb1 mesa-vulkan-drivers xvfb`):
`libxkbcommon-x11`, `libxcb`, `vulkan-intel`/`vulkan-radeon`/`vulkan-swrast` + `vulkan-icd-loader`, `xorg-server-xvfb`
(unverified until Phase 8 wires CI/dev docs; the build above ran with the packages listed at the top of this
section). Wayland: eframe is built with both `wayland` and `x11` features; upstream only ever ran on X11/software
Vulkan, so the Wayland result above is the first evidence either way.

## 7. Noted for Phase 2 (blindspot pass) — pointers only

- Upstream is 8 days old and pushing daily; hard fork at `b81f40b`, cherry-pick by hand (D0.3).
- Persisted layouts/prefs carry finance codes and versions (prefs v3, `LayoutDoc.version`); the cutover must bump
  versions and drop upstream migrations rather than inherit them.
- `bars.sqlite3` PK `(symbol, tf, ts)` cannot key per-PA events; cache design is part of the storage fork.
- Lahman is CC-BY-SA-3.0 (share-alike on derived data); saberkit's constants cite Retrosheet; MLB Stats API and
  Savant terms unverified — licensing section of `06-risks.md`.
- Toolchain floor is misdeclared: `workspace.package.rust-version = "1.85"` is inherited by no member (every crate's
  `cargo metadata` `rust_version` is null), the code uses let-chains (stable since 1.88) and the locked
  egui/eframe/egui-wgpu/epaint 0.36.2 declare `rust-version = "1.95"` — so the real floor is 1.95, verified on
  1.99.1. No `rust-toolchain.toml`; CI uses unpinned stable; Arch's `rust` package tracks latest stable and has no
  rustup to honour a pin.
- `tests/render.rs` skips silently without an adapter — CI must assert it actually rendered.
