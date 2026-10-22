# Codebase map (inherited terminal-rs code)

Map of the inherited terminal-rs code for agents working on sabr-rs; verdicts and their evidence live in
`docs/plan/00-audit.md`. LOC are `wc -l` of `.rs`/`.wgsl` at upstream `b81f40b`, inline tests included.

## Crates

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

## Files: generic vs finance

### `app/src`

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `anim.rs` | 243 | keep | easing/animation helpers, no domain types |
| `dock.rs` | 609 | keep | pure dock tree (split/tabs/focus), 7 tests |
| `layout.rs` | 65 | keep | preset layout shapes |
| `layouts.rs` | 573 | adapt | `Store`/`LayoutDoc`/`Link`/`write_atomic` generic; finance in the `PaneSettings` variants (:58-117) **and** three constructors: `LayoutDoc::repair` inserts a `Watchlist` pane into an empty doc (:205-214), `default_workspace` hard-codes `AAPL US Equity`/`Feed`/`Wei`/`Watchlist` (:235-243), `chart_settings` seeds tf/chart type/`default_indicators()`/volume/Congress markers (:264-276) |
| `palette.rs` | 446 | keep | fuzzy matcher + recents, no domain types |
| `theme.rs` | 304 | keep (drop `party_color` :139-159, `candle_up/down`) | palette + thread-local scale/density |
| `themes.rs` | 777 | keep | 8 built-ins, TOML/JSON theme store |
| `app/theming.rs` | 169 | keep | theme selection/cycling/edit debounce |
| `widgets.rs` | 379 | keep (drop `buy_sell_pills` :354) | shared egui widgets |
| `main.rs` | 75 | adapt | eframe bootstrap; `TR_MSAA`, macOS viewport branch, renderer install |
| `app.rs` | 1,620 | adapt | `PaneKind` hub, `dispatch`/`drain`/`periodic`, hotkeys, smoke hook — every finance pane is wired here |
| `worker.rs` | 947 | adapt | tokio worker: `Request`/`Response`, bar cache, congress scheduler, series generations |
| `command.rs` | 746 | adapt | parser/hint mechanism generic; `Function` table and fallback resolution are finance |
| `chart_view.rs` | 1,472 | adapt | chart pane: viewport/scale/drawings generic; `Range`, `ts`↔index, OHLC/volume/decimals assumptions finance |
| `prefs.rs` | 897 | adapt | `Chord`/`Keymap`/`Action` generic; `Action` list and `PanelDefaults` finance |
| `app/settings_view.rs` | 719 | adapt | settings tabs; drop `Brokerage`/`Trading` |
| `app/palette_view.rs` | 682 | adapt | command palette UI; `COMMANDS` table is finance |
| `app/hotkeys_view.rs` | 401 | adapt | hotkey + HELP text views; HELP text is finance |
| `app/panels.rs` | 461 | adapt | `render_content` dispatcher; finance arms removed |
| `app/chrome.rs` | 970 | adapt | toolbar/sidebar; portfolio rows, `compact_money`, macOS titlebar consts |
| `app/workspace.rs` | 1,649 | adapt | drag/compass/pop-out/links generic; `set_pane_symbol`, `panel_from_spec`, preset fillers finance |
| `app/trading.rs` | 2,123 | remove | order tickets / trade windows |
| `app/portfolio_view.rs` | 1,211 | remove (keep ≈110 LOC `Col/fit/span/cell/header` :113-226) | portfolio tables |
| `app/broker.rs` | 799 | remove | SnapTrade setup/credentials UI |
| `congress_view.rs` | 447 | remove (structure copied for events feed/profile) | congress feed/ticker/profile views |
| `options_view.rs` | 377 | remove (panel idea → projection distribution) | option chain table |
| `markets_view.rs` | 381 | remove | WEI world markets + GC yield curve |
| `app/demo.rs` | 273 | remove | demo paper-trading state |

### `crates/chart/src` (+ tests/benches)

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `gpu.rs` | 466 | keep | renderer: buffer cache, uniform ring, pipelines |
| `lod.rs` | 281 | keep (drop `BarPyramid`/`GpuBar` :10-22, 47-125; fix marker merge :143-145) | `LinePyramid`, markers (lossy on collision) |
| `viewport.rs` | 231 | keep | index-based viewport |
| `scale.rs` | 192 | keep (drop `time_ticks/label/crosshair_label` :104-135) | `nice_step`, `price_ticks`, formatting |
| `scene.rs` | 532 | adapt (keep `Overlay`/`overlay_cmds`/`sub_pane`; rewrite the main-pane path) | `PricePane`/`visible_extent`/`autoscale`/`price_pane` (:100-233) are typed on `SeriesGpu` + `ChartType` and read the bar pyramid even for line charts; `SeriesGpu`/`ChartType` (:34-80) go |
| `drawings.rs` | 114 | keep (drop `Tool::Fib`/`magnet`) | drawing tools |
| `headless.rs` | 135 | keep | offscreen render for pixel tests |
| `shader_line.wgsl` | 117 | keep + extend (`vs_band`) | line/steps/area/hist/dots vertex fns |
| `shader.wgsl` | 164 | keep prelude, drop `// ---- bars` section | shared uniforms + candle/volume |
| `lib.rs` | 18 | keep | re-exports |
| `tests/render.rs` | 202 | keep | 4 headless pixel tests |
| `benches/frame.rs`, `examples/framebench.rs` | 64, 96 | keep | 1M-bar frame benchmarks |

### `crates/core/src`

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `indicators/registry.rs` | 1,357 | adapt (plumbing ≈430 lines kept, catalog re-enumerated) | `Def/Spec/Param/Plot/Tone/Output/Input` contract + 151-entry catalog |
| `indicators/stats.rs` | 372 | keep | O(1) rolling windows, variance/zscore/percent_rank/linreg |
| `indicators/ma.rs` | 491 | keep (drop VWMA/EVWMA/FRAMA) | moving averages on one series |
| `indicators/mod.rs` | 209 | keep | shared helpers |
| `indicators/momentum.rs` | 655 | adapt | 27 close-only kept, 16 OHLC removed |
| `indicators/volatility.rs` | 307 | adapt | 4 close-only kept (BBW, PCTB, ULCER, RVI); HV/PARKINSON/GK/RSVOL/YZ annualise via `periods_per_year(tf)` (252 × 23,400 s US sessions, :108-115) |
| `indicators/trend.rs` | 521 | mostly remove | 19 of 22 need OHLC |
| `indicators/volume.rs` | 395 | remove | all 23 need volume |
| `indicators/levels.rs` | 176 | remove | pivots on UTC sessions |
| `bar.rs` | 165 | replace | `Bar`/`Series` → entity/metric series |
| `timeframe.rs` | 179 | replace | `Timeframe` → event-count/calendar `Window` |
| `security.rs` | 303 | replace | yellow keys, `looks_like_ticker` rejects numeric ids |
| `catalog.rs` | 396 | replace (keep `search` ranking idea :305) | static `Listing` table |
| `sample.rs` | 134 | remove | seeded GBM generator |
| `options.rs` | 141 | remove | Black-Scholes |
| `lib.rs` | 14 | adapt | re-exports |
| `benches/indicators.rs` | 34 | keep | catalog-wide bench pattern |

### `crates/data/src`

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `lib.rs` | 510 | adapt (trait shape, `Fallback`, `clip`, `from_env` ≈250 lines) | `MarketData` methods and types are finance |
| `cache.rs` | 84 | adapt | open/WAL/upsert pattern; PK `(symbol,tf,ts)` cannot key per-PA events |
| `sample.rs` | 231 | remove (keep env switch + determinism contract) | GBM provider with 400 ms ticks |
| `yahoo.rs` | 522 | remove | |
| `crypto.rs` | 499 | remove (WS reconnect pattern ~340-365 noted) | |
| `treasury.rs` | 264 | remove (per-year file cache :124-150 is the per-season model) | |
| `markets.rs` | 164 | remove | `AssetClass` router |
| `alpaca.rs` | 134 | remove (page-walk loop :75-111 noted) | |
| `rss.rs` | 97 | remove | |
| `tests/live.rs` | 109 | remove | 5 `#[ignore = "network"]` tests |

### `crates/congress/src`

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `refresh.rs` | 510 | adapt (≈250 lines) | `Source` trait, `RefreshConfig`, `refresh()`/`store()` orchestration |
| `db.rs` | 604 | adapt (`tracked`/`alerts`/`meta` + pragmas ≈150 lines) | schema, dup-merge, WAL |
| `roster.rs` | 194 | adapt (`normalize_name`/`find_by_name`) | XML roster parsers removed |
| `model.rs` | 284 | replace | chamber/party/`AmountRange` enums |
| `house.rs` | 341 | remove | PDF filing parser |
| `senate.rs` | 273 | remove | eFD scraper |
| `performance.rs` | 135 | remove | trade performance |
| `lib.rs` | 13 | adapt | |
| `tests/refresh.rs` | 93 | keep as pattern | fixture-backed `Source` |
| `tests/parsers.rs` | 190 | remove | |
| `examples/ingest.rs`, `examples/ptr.rs` | 40, 16 | remove | |

### `crates/broker/src`

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `model.rs` | 748 | remove (pattern only: `Position` row + `Portfolio::totals(filter, price_of)` closure repricing, :112-146, 330-398) | fields (`units`, `multiplier`, `cash`, `buying_power`, `cash_equivalent`) and the broker-reported-totals branch are brokerage semantics — rewrite, don't retype |
| `snaptrade.rs` | 579 | remove | |
| `trading.rs` | 946 | remove | |
| `demo.rs` | 579 | remove | |
| `sample.rs` | 241 | remove | |
| `secrets.rs` | 316 | remove | no API keys on the default path |
| `lib.rs` | 19 | remove | |
| `tests/{client,demo,throttle,trading}.rs` | 274, 272, 175, 294 | remove | mock-server + throttle tests |

### `crates/macos/src`

| File | LOC | Verdict | Reason |
|---|---|---|---|
| `lib.rs`, `updater.rs` | 189, 103 | remove | objc2 chrome + Sparkle; stubs off macOS |

## Shared vocabulary

Every layer depends on three `tr_core` types (the real coupling, more than any crate boundary):
`tr_core::Bar {ts,open,high,low,close,volume}` (bar.rs:7-14), `tr_core::Timeframe {M1..MN1}` (timeframe.rs:11-22),
`tr_core::Security {ticker, class: AssetClass}` with Bloomberg yellow keys (security.rs:111-134). They flow
trait → cache → worker → chart → command parser → layout JSON. Replacing them is the one cross-cutting change;
everything else is local.

## Persisted files and formats

`settings.json`, `prefs.json` (v3 with migrations, prefs.rs), `layouts/*.json` (`LayoutDoc {version, name, dock:
Node, panes: Vec<SavedPane>, focus, sidebar_side, theme}`), `themes/*.toml|json`, `congress.sqlite3`,
`bars.sqlite3`, `demo-account.json`, `trading-access.json`, palette recents. Indicator `Spec` serialises as its
label and preserves unknown codes on load (registry.rs:275-280) — persisted finance codes would silently survive a
cutover unless the layout version is bumped and migrations dropped. A 6 s smoke run writes `bars.sqlite3`,
`congress.sqlite3`, `layouts/Default.json`, `settings.json` (520 KB); `prefs.json` only when prefs change.

## Data directory

Linux: `$TR_DATA_DIR` else `$XDG_DATA_HOME/terminal-rs` else `~/.local/share/terminal-rs` (worker.rs:309-321).
