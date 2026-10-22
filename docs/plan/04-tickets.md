# 04 — Tracer-bullet tickets (executed 2026-10-21/22)

Blocking order and wave assignment as executed. Ticket 01 (the core loop: bundled Statcast for one player →
Marcel projection → fan chart) is split across T1–T8 by design so each slice lands testable; T8 is the first
ticket that shows the complete loop on screen.
**Wave 6–7 additions (2026-10-22):** T14–T17 below; T13's Wayland finding unchanged.

| # | Ticket | Wave / agent | Delivers | Blocking |
|---|---|---|---|---|
| T14 | Retro parser + cut lines | 6 · RetroParser | `crates/retro`: full event-file parser (all measured token families + the corpus-sweep corrections: `/R`-tails, `SB/CSH` markers, `MREV/UREV/BOOT/PASS/UINT/BF` annotations, `K+E2` error compounds, `TH<n>` throws, `#` markers, `GTP/LTP/FTP`, `C/AP/INT/FINT/IF`), `data,er` ER path, `radj` not-a-PA, ARI 2024 reconcile 100% (PA/H/2B/3B/HR/BB/IBB/HBP/SF/SH/SO + pitching BF/H/ER/HR/BB/IBB/HBP/SO), loud unknown-token failures | T2 |
| T15 | Pitching Marcel | 6 + 7 · PitchingMarcel, PitchingGolden | `marcel.pitching` with the recovered constants per DR-3.19a (flat 268-IP regression — reliability 100% match, role-weighted IP = 25 + 35·(GS/G) + 0.5·IP₁ + 0.1·IP₂, inverse age direction, AL/NL split regression); hard golden gate at the measured 0.20 floor (85% wOBA-style bar unattainable from the Lahman sample — diagnosed, logged, deviation in both module headers) | T11 |
| T16 | Entity picker pane | 6 · PickerPane | `PaneKind::EntityPick` (ephemeral, never persisted): ranked rows, keyboard select, token rewrite → entity key, 3-round cap, focus handling; 6 pane tests | T12 |
| T17 | CLI | 7 · CliReport | `sabr fit/backtest/report/refresh/build-corpus/constants/trim-register` (fit+index round-trip, gate exit codes, Markdown+HTML reports, merge-loop for the 25-season cut-line build), artifact read half `ArtifactIndex::latest` | T11, T14 |
| T18 | In-season folds | 7b · InSeasonFolds | `Corpus.cuts` (retro→Lahman id join via retroID), `Model::project_in_season` (partials map; single-partial path delegates), harness loop over the five in-season cuts (**147 scored folds** — 2020's COVID season lacks early cut rows), Future z-basis (√(actual−to_date)), per-cut rolling residual windows; bundled cut-lines installed (133,183 rows); full suite **38.6 s**; PA MAE 147.4; preseason MAE 0.02815, in-season 0.02534; **gate FALSE: 2025/Low PICP 0.877 (n=122) — Wilson interval excludes 0.80** (honest over-coverage, not tuned) | T14, T15 |
| T20 | Headless smoke | 8 · main | `sabr smoke --frames N`: renders every pane of the default workspace and all three presets with no window/display; asserts non-zero clipped shapes per stage; the sandboxed stand-in for the windowed `TR_SMOKE_SECS` run | T19 |
| T19 | Presets | 7 · PresetsAndPanes | the three DR-3.41 presets wired to implemented panes with documented substitutions (`LAYOUT MORNING\|PROSPECTS\|WARROOM`), per-preset worker loads on apply, round-trip + headless render tests | T16 |

Pending: ROST/FARM/COMP/SPLT/GAME worker arms and panes (presets currently substitute — documented), fan-chart
ribbon pane wiring (DR-3.40), R snippet doc, network fetch layer (Wave 8), `05-posting-alignment.md` gap list.

| # | Ticket | Wave / agent | Delivers | Blocking |
|---|---|---|---|---|
| T1 | Workspace scaffold | 1 · ScaffoldDeps | `rust-version = "1.95"`, workspace deps `zstd`/`jiff` | — |
| T2 | Model crate | 1 · ScaffoldModel | `sabr-model`: Entity/Role keys, Unit, Window, Event/EventLog/SourceRev, Metric registry (26, Carleton thresholds, bat-tracking coverage intervals), Series/derive with defined-count windows, reliability weights | T1 |
| T3 | Data primitives | 1 · DataPrimitives | `sabr-data`: Fetch/FetchKey (7 shapes, stable keys), Provenance/Attribution (six sources), Freshness/Loaded, Policy, RawStore (SQLite index + zstd blobs, Mutex'd connection) | T1 |
| T4 | Bundled dataset | main · (agent-assisted) | `sabr-data/sample` 26 MB: Ohtani 2025 bat 2,940 / pit 753 rows (SHA-pinned), Lahman 1998–2025 (Batting/Pitching/People/Appearances/Teams, People from the SABR Box release), Salaries ≤ 2016, catalog.csv (135,208 MLBAM-keyed register rows @ 781c175), manifest.sha, source notes | — |
| T5 | Bundled-set loader | 1 · BundleLoader | `bundled.rs`: manifest verification (hash mismatch = error), BOM-stripping season-line loader | T3, T4 |
| T6 | Savant normalizer | 2 · SavantNormalizer | `savant.rs`: header-name parse with pinned allowlist, BOM strip, schema error naming the diff, alias rules, seq by (date, game_pk, at_bat, pitch), duplicate-triple rejection, PA/AB/BF/BIP flags; fixture tests (2,940/753, shuffle-stable) | T2, T3 |
| T7 | Chart extensions | 2 · ChartExtensions + main | `vs_band` + `BandPyramid` (min/max merge), `LinePyramid::build_gaps` (verified: plain build loses 1-point gaps 50–94%), index-axis month ticks in `scale.rs`, pixel test at two zoom levels; GPU fix: storage bind groups carry both bindings | T2 |
| T8 | Catalog + search | 2 · CatalogSearch | `catalog.rs`: accent/Jr. folding, span-weight (role-neutral) ranking, role filter via Appearances evidence, `search_ranked`; Edwin Díaz top-3 under `DIAZ`+Pit | T4 |
| T9 | Transactions normalizer | 2 · TransactionsFixture | `transactions.rs`: dedupe on (person, date, type, description) — 4 rows → 2 verified | T3 |
| T10 | XDG dirs | 2 · XdgDirs | `xdg.rs`: config/cache/state/data split, 0700, env overrides | T3 |
| T11 | Forecasting core | 3 · ForecastCore | `sabr-forecast`: Corpus from Lahman, Constants (prototype weights + 2009 recovered set), Marcel batting (golden 2010: 90.5% within tolerance, median |ΔwOBA| 0.0018), PriorSeason, walk-forward harness (no-look-ahead perturbation test), PA-standardised intervals (DR-3.21a), Wilson gate (DR-3.12a), hashed artifacts + SQLite index; suite 6.1 s; pooled MAE 0.0281; deviations logged (preseason-only folds until the retro parser lands) | T2, T5 |
| T12 | Grammar + worker | 3 · GrammarWorker | command.rs sabr grammar (CAST/PROJ/BTST/ROST/TRACK/UNTRACK/ALRT, `.BAT/.PIT` suffix, window labels, metric codes, picker stub), worker arms (Series from bundled fixtures, Projection with model stamp, Backtest via spawn_blocking, TrackEntity/UntrackEntity/SabrAlerts), `track.rs` store | T6, T11 |
| T13 | Integration smoke | 4 · main | X11 smoke green (`smoke: 26 frames in 10.1s`, 0.45 ms/frame, panes 4); Wayland run on this machine starves repaints with the window unfocused — reproduces on pristine upstream, so recorded as environment, not a code regression | T7, T12 |

Queued: ROST/FARM/COMP/SPLT/GAME arms, fan-chart pane wiring (DR-3.40), R snippet, network fetch (Wave 8) — see the T14–T19 table above.

## Acceptance status (updated 2026-10-22 through T18)

- `cargo test --workspace --release`: **389 passed, 0 failed** (upstream baseline 277 + 112 new).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `tests/render.rs`: 4 passed on the real GPU adapter (no silent skips).
- Smoke: **headless** `sabr smoke` (T20) — default 4 panes/365 shapes, MORNING 5/131, PROSPECTS 3/103,
  WARROOM 4/144, all stages non-blank, no display server. The windowed path is barred from agent runs:
  it crashed the local compositor (reproduces on pristine upstream; see `06-risks.md` P5).
- Walk-forward: 147 scored folds (25 preseason + 122 in-season), **38.6 s** wall (budget 60 s);
  pooled MAE preseason 0.02815 / in-season 0.02534; PA MAE 147.4.
- **Gate: FALSE** — with in-season folds in the pooled timeline the last three scored folds are 2025's
  in-season cuts and the 2025/Low cell over-covers (PICP 0.877, n=122; Wilson excludes 0.80). Honest failing
  cell per DR-3.12a; reported, not tuned. Operator decision pending: gate preseason separately from in-season
  (story-25's "scored separately") or treat the pooled red as the standing result until the interval model
  improves (v1.1 candidate).

## Known reconciliations (committed fixtures)

- ARI 2024: batting PA/H/2B/3B/HR/BB/IBB/HBP/SF/SH/SO and pitching BF/H/ER/HR/BB/IBB/HBP/SO all **100.00%**.
- Season-wide pitching outs +1.6%, runs attribution ~13–65% (mid-PA substitution run-charging is the known
  hole; ER sidesteps it via the `data,er` path). Reported per-inning in the Report; not gated.
