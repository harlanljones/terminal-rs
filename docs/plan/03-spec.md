# 03 — sabr-rs specification (Phase 6)

A disposable projection of the decision records in `01-grill.md` (DR-3.1–3.41, DR-3.12a, DR-3.21a), the design
verdicts in `02-design-arena.md` (V-A, V-B1, V-B2, V-C) and the audit (`00-audit.md`). Where this document and a
decision record disagree, the record wins and this document is wrong. Working names throughout; crate boundaries
are set in Phase 8.

## 0. Decisions most likely to be revisited

| Area | Decision | Record | Why it may move |
|---|---|---|---|
| Data model | Entity = (player, role ∈ {bat, pit}) on MLBAM id; events canonical, every series derived at view time; windows in event units (PA, BF, pitch, game, AB, BIP) or season | DR-3.2, V-A, DR-3.35 | a third role, or a view-time derive that proves too slow for league-wide windows (bench says 4.9 ms at 700K, so unlikely) |
| Forecasting interface | `Projector` returns a distribution-valued forecast; Marcel returns points and the harness attaches intervals | B-F1 finding | a model that needs the corpus at a different grain |
| Interval model | PA-standardised residual quantiles per reliability bucket, strictly-earlier folds | DR-3.21a | a model family whose error does not scale with √PA |
| Calibration gate | 0.80 inside the Wilson 95% interval per (season, bucket), small cells merged upward | DR-3.12a | larger cells make the fixed band viable again |
| Bundled sample | one real Statcast player-season (Ohtani 2025, both roles) committed; risk accepted | DR-3.1, DR-3.16 | any objection from MLBAM — fallback is the shipped fetch script |
| Payroll | Cot's public sheets fetched at runtime, cited, cached; Lahman ≤ 2016 bundled | DR-3.25 | sheet churn beyond one fix per season |
| In-season corpus | Retrosheet event files parsed in-repo into cut lines | DR-3.37 | pitcher earned-run parsing proving unreliable → RA9 components |
| Grammar | `<ENTITY>[.BAT\|.PIT] <FUNCTION> [ARGS] GO`, picker on ambiguity | DR-3.23, DR-3.39 | prototype P5 said the suffix works; the picker's weight is a spec detail |
| Presets | three, as mocked (5 / 3 / 4 panes) | DR-3.41 | frame budget with real data |
| Comparators | Marcel + naive + Steamer 2017–2024 aggregate scores | DR-3.11 | Steamer withdrawing free downloads |

## 1. Problem statement

Front-office analyst postings in 2026 (Rockies Senior/Lead Analyst (Forecasting) — the anchor, closing
2026-10-30; Orioles, Mets, Blue Jays, Rays, and the September batch) price the same things: deployed predictive
models with honest uncertainty, reproducible pipelines over real baseball data, validation that a reviewer can
re-run, and tools staff actually use rather than notebooks. A portfolio needs to show all four at once, on data
anyone can obtain, offline, with the licensing stated rather than ignored.

`terminal-rs` is a Bloomberg-style trading terminal in Rust: GPU-rendered charts, dockable panes, a
`<SECURITY> <FUNCTION> <GO>` command line, saved layouts, an indicator engine and an alert system. Its vocabulary
is finance — bars, tickers, order books, options — and its data layer is finance providers. The audit found the
shell, chart engine, docking, themes, command mechanism, keyboard model, alert skeleton and link groups reusable,
and everything finance-shaped replaceable.

sabr-rs replaces the finance vocabulary with baseball's: entities are players (by role), teams, games and
matchups; series are pitch- and plate-appearance-indexed metrics; studies are parameterised sabermetric windows;
the "options chain" is a projection distribution; the "portfolio" is a roster with payroll and projected-WAR
gaps; the watchlist is tracked players with projection-delta alerts. The forecasting core is a Marcel-style
baseline with reliability-weighted regression, aging, park factors and a playing-time model, backtested
walk-forward against published systems, with calibration as an acceptance test and a model registry that stamps
every number with its version, training window and last backtest score.

## 2. Solution, from the user's perspective

You launch `sabr-rs` on Arch (or any distro via `cargo install`). With no network it opens on the bundled dataset:
Shohei Ohtani's 2025 Statcast season as a hitter and as a pitcher, season lines for every player 1998–2025,
cut-date lines for the in-season backtests, the league constants, the player catalog, and historical salaries to
2016. Every pane shows where its numbers came from and how old they are.

You type `OHTANI CAST` and get a 50-PA rolling xwOBA on a PA-indexed GPU chart with month ticks, season dividers,
IL stints shaded, and an as-of cursor. `OHTANI.PIT CAST` switches role. `OHTANI PROJ` shows the Marcel projection
for the rest of the season and next season as nested 50/80/95 ribbons, with the model id, version, training
window, as-of date and last backtest score printed on the pane, and a sensitivity drawer where you override
playing time, park and age and watch the ribbons move. `BTST MARCEL` shows the walk-forward grid: PA-weighted MAE
of wOBA per season against the naive baseline and the Steamer aggregate, and a (season × reliability bucket)
coverage grid with Wilson whiskers around 0.80 — the "where my system is wrong" panel. `DIAZ CAST` opens a picker;
`DIAZ.PIT CAST` narrows it to pitchers. `NYM ROST` shows the 26-man roster with Cot's salaries (cited), sabr-rs WAR
and the gap to a target by position. `NYM FARM` lists minor-league season lines and the draft feed. `SOTO COMP`
lists comparable players by Statcast profile and age. `TRACK OHTANI` adds a tracked player; a projection move
beyond your threshold or an IL transaction raises a toast. Three presets ship: Morning front-office briefing,
Prospect watch, Trade deadline war room; layouts save and reload.

With network, any player you name is fetched from Savant on demand (one player-season per request, never bulk),
cached locally, and refreshed nightly by the same `refresh` command the CLI exposes. `sabr fit`, `sabr backtest`,
`sabr refresh` and `sabr report OHTANI` run without the window; `report` writes a one-page Markdown/HTML summary for
someone who will never open a terminal. Everything a projection shows can be regenerated from pinned inputs: the
Lahman release hash, the Retrosheet commit, the register commit, the constants file hash and the protocol hash are
in the artifact.

## 3. User stories

Identity and search
1. As an analyst I type a surname and get a ranked picker (accents and Jr./Sr. folded) so a 10-way collision resolves in one keystroke.
2. As an analyst I add `.BAT` or `.PIT` to any player token to fix the role; a two-way player is two series and one profile.
3. As an analyst I type an MLBAM id and the name resolves inline; an unknown id says "not in register".
4. As an analyst I search a minor leaguer by name and find them offline because the catalog covers everyone with an MLBAM id.
5. As a reviewer I see which register commit the catalog was trimmed from.

Series and charts
6. As an analyst I see a per-PA metric (xwOBA, EV, K%, BB%, bat speed, swing length, attack angle) as a rolling window on a PA-indexed axis.
7. As an analyst I change the window (50 PA, 100 PA, 10 games, season) and the chart re-derives from events instantly.
8. As an analyst I read a reliability label on every window (shrinkage weight n/(n+x)) instead of a cliff.
9. As an analyst I see month ticks, season dividers, IL stints as shaded bands, and an as-of cursor that dims the future.
10. As an analyst I see a gap, never a zero, where a feature was not tracked (bat tracking before 2023-H2, the March 2024 hole).
11. As an analyst I zoom out and gaps still show.
12. As an analyst I overlay two players (COMP) on one PA axis with link groups keeping their panes in step.
13. As an analyst I see transaction and IL markers on the series, and a hover card with the event text.
14. As an analyst I view a pitcher's series indexed by batters faced or pitches.

Projections
15. As an analyst I get a rest-of-season and a next-season projection for any player, as-of any date.
16. As an analyst I see 50/80/95 ribbons whose width comes from backtest residuals, not from a formula I cannot audit.
17. As an analyst I see the model id, version, training window, as-of date and last backtest score on the projection pane.
18. As an analyst I override playing time, park and age in a sensitivity drawer and the ribbons update without changing the model's own score.
19. As an analyst I see the playing-time projection and its own backtest error separately from the rate projection.
20. As an analyst I see the reliability bucket the projection fell in.
21. As an analyst I compare the projection with the naive "prior season" baseline and the Steamer aggregate on the same pane.
22. As a reviewer I regenerate any projection from the artifact's pinned inputs and get the same numbers.

Backtests and honesty
23. As an analyst I open BTST for a model and see PA-weighted MAE of wOBA per walk-forward season, re-centred, all players included.
24. As an analyst I see the (season × bucket) coverage grid with Wilson whiskers around 0.80, the [.75, .85] reporting band, cells below n_min flagged for merge, and a pooled row per bucket.
25. As an analyst I see the in-season cuts (May–Sep) scored separately from preseason.
26. As a reviewer I run the backtest suite inside the test suite in under 60 s and the calibration gate is the acceptance test.
27. As a reviewer I can prove no look-ahead: perturbing every season after the as-of date leaves every projection byte-identical.
28. As an analyst I see which seasons were warm-up (no interval) and why.

Data, provenance and policy
29. As a user I run everything offline from a fresh clone with the bundled dataset.
30. As a user I fetch a player's Statcast season on demand; the app never pulls a season in bulk and refuses fan-out beyond a per-source budget, showing "stale" instead.
31. As a user I see every pane's source, fetch time and age (freshness chip), and the attribution each source requires.
32. As a user I run `refresh` from the CLI, from the app, or from a systemd user timer and get the same result.
33. As a reviewer I see the SHA-256 of every pinned input (Lahman files, Retrosheet commit, register commit, constants) in the artifact and the risks document.
34. As a developer I re-run a fixed normaliser over stored raw payloads without re-fetching.
35. As a user I am told, in the app and in the docs, which terms govern each source and what the project assumes.

Roster, payroll, WAR
36. As an analyst I open ROST for a team and see the roster by position with salary (Cot's, cited), sabr-rs WAR and surplus.
37. As an analyst I see WAR gaps by position against a target team and the components of the WAR number.
38. As an analyst I see the CBT line and the deadline T-minus on the war-room preset.
39. As a user with no network I see historical salaries to 2016 and "no current payroll — fetch" for later seasons.

Farm and draft
40. As an analyst I open FARM for an org and see minor-league season lines by level.
41. As an analyst I see the draft feed for a year with picks, bonuses, schools and the scouting blurb.

Alerts
42. As an analyst I TRACK a player and get a toast when their projection moves beyond my threshold since the last refresh.
43. As an analyst I get a toast on an IL or transaction event for a tracked player, once per event (duplicates deduplicated).
44. As an analyst I see my alerts in the ALRT pane with seen/unseen state.

Presets and layouts
45. As a user I pick one of three presets and get a working screen on the bundled data.
46. As a user I save and reload layouts; stale codes from upstream layouts are dropped, not resurrected.
47. As a user my config, caches and layouts live in the XDG directories a Linux user expects.

Reporting and CLI
48. As an analyst I run `sabr report <entity>` and get a one-page Markdown/HTML with the projection, interval, backtest score, as-of, model version and attributions.
49. As an R user I load the exported CSV artifacts with the documented snippet.
50. As a developer I run `sabr fit`, `sabr backtest`, `sabr build-corpus`, `sabr trim-register` and `sabr constants` and each writes a hashed artifact.

Packaging
51. As an Arch user I install from the PKGBUILD; as any Linux user I `cargo install --locked --git`.
52. As a reviewer I see CI prove a non-Arch build and an actual rendered frame, not a silent skip.

## 4. Implementation decisions

### 4.1 Finance → baseball mapping

| terminal-rs | sabr-rs | Record |
|---|---|---|
| security (ticker + yellow key) | entity: player (MLBAM id + role), team, game, matchup | DR-3.2, DR-3.23 |
| bar (ts, OHLCV) | event (seq, date, fields) in an event log; derived series of (season, seq, date, value, reliability) | V-A |
| timeframe / range | window: event-count (PA, BF, pitch, game, AB, BIP) or season or date span | DR-3.35 |
| indicator study | parameterised metric over a window, with coverage intervals and a reliability threshold | DR-3.35, B3 |
| Congress trade markers | transaction / IL events as markers and alert sources | DR-3.13 |
| options chain | projection distribution: quantiles with playing-time / park / aging sensitivities | DR-3.21a, DR-3.36 |
| portfolio | roster + payroll + projected-WAR gaps | DR-3.25, DR-3.26 |
| watchlist | tracked players with projection-delta and transaction alerts | DR-3.13 |
| Live/Offline chip | as-of + freshness per source | DR-3.10 |
| sample provider (synthetic) | bundled real dataset | DR-3.1, DR-3.16, DR-3.30 |

### 4.2 Module map (working names)

- **model** — entity, role, window, unit, metric registry, event log, derived series, reliability labels,
  coverage intervals, the sabermetric rolling engine, chart adapters.
- **forecast** — projector interface, Marcel and naive baselines, playing-time model, aging, park factors, the
  walk-forward harness, the interval model, scores and the gate, artifacts and the registry index.
- **retro** — Retrosheet event-file parser, cut-line builder, league-constants tool (wOBA weights, park factors).
- **data** — sources (Savant, Stats API, Cot's), raw payload store with provenance, normalisers, per-domain row
  tables, the bundled set, the `Data` facade, refresh.
- **app** — the inherited shell: command parser (re-enumerated), panes, presets, worker requests/responses,
  alerts, prefs; chart engine kept with three extensions (band primitive, gap-preserving LOD policy, index-axis
  labels).
- **cli** — `fit`, `backtest`, `refresh`, `report`, `build-corpus`, `trim-register`, `constants`, `export`.

Kept from upstream unchanged: docking tree, layout store, themes, keyboard model and rebinding, palette, animation,
GPU renderer, viewport, line/area/hist/dots/steps pipelines, headless render harness, alert tables and refresh
orchestration shape, link groups. Removed: every finance provider, options, SnapTrade/broker, order book, world
markets, yield curves, news, the macOS crate, the synthetic sample generator, 85 OHLC/volume/session studies.

### 4.3 model

**Entities.** A player entity is an MLBAM id plus a role (bat or pit). Teams are MLBAM team ids. Games are game
pks. A matchup is a batter entity and a pitcher entity. Entities serialise as text keys (`p:660271:bat`,
`t:119`, `g:777288`, `m:660271:bat/592789:pit`) used in layouts, caches and the command line's rewritten tokens.

**Catalog.** The Chadwick register trimmed to rows with an MLBAM id (≈136K rows), fourteen columns: the stable
UUID, MLBAM, Retrosheet, bbref, bbref-minors and FanGraphs ids, last/first/given names, birth year, MLB first/last
season, professional first/last season. Pinned to a register commit SHA recorded in the bundled manifest. Search
folds accents and suffixes, ranks exact token first, then a role-neutral weight; `.BAT`/`.PIT` filters by role.
Players with an empty FanGraphs id are normal (new debuts).

**Events.** An event log is one (entity, season) sequence of events. Each event carries a sequence number, a game
pk, a date, a unit marker (which counters it increments: PA, AB, BIP, pitch, BF, game) and a field map keyed by the
pinned Savant allowlist names. Sequence numbers are assigned by sorting on (game date, game pk, at-bat number,
pitch number) — never by file position. A log carries its source revision: source name, URL, fetch time,
SHA-256 of the raw payload.

**Windows and units.** A window is either an event count in a unit (PA, BF, pitch, game, AB, BIP), a season, or a
date span. Windows are backward-looking from each point and may cross season boundaries when the derive is given
several logs.

**Metrics.** A metric declares an id, a version, the unit it counts, its coverage intervals (date intervals where the
inputs exist; outside them the value is undefined), its reliability threshold as (unit, x) from the Carleton table
or none, and an evaluation over a slice of events returning undefined when inputs are missing. "Undefined is not an
error" (saberkit rule): undefined never becomes zero or NaN in storage, and renders as a gap. The v1 metric set:
wOBA, xwOBA, EV, launch angle, barrel rate, hard-hit rate, K%, BB%, whiff rate, chase rate, bat speed, swing length,
attack angle, attack direction, swing path tilt (hitters); velocity, spin rate, extension, whiff rate, K%, BB%,
xwOBA against, xERA components (pitchers). Rolling studies kept from upstream's engine: mean, EMA-style smoothing,
z-score, percent rank, linear regression slope — re-targeted at metric values with defined-count windows.

**Derived series.** A series is (entity, metric, window, as-of, points); a point is (season, seq, date, value or
undefined, reliability n, reliability weight). Derivation is a pure function of the logs, the metric and the
window, evaluated at view time (bench: 4.9 ms at 700K events); nothing is memoised in v1.

**Reliability labels.** Weight = n/(n+x) with n counted in the metric's threshold unit and x from the Carleton
table (K% 60 PA, BB% 120 PA, ISO 160 AB, OBP 460 PA, AVG 910 AB, BABIP 820 BIP; r = 0.7 convention stated in the
UI); metrics without a published point are labelled "no threshold".

**Coverage intervals.** Static per metric: bat tracking = {2023 second half (partial)} ∪ [2024-04-03, ∞); swing
path metrics share it; plate_x/plate_z and sz_top/sz_bot carry an epoch flag (front-of-plate through 2025,
middle-of-plate/ABS from 2026) exposed to any metric that uses them; velocities carry the PitchFX/Statcast epoch.

**Chart adapters.** A series becomes a main-pane line series (the bar pyramid path is replaced by a line-primary
pane), with: a gap-preserving LOD merge (any undefined point in a group makes the group undefined), index-axis
labels (month ticks at the first PA of each month, season dividers, IL/coverage gaps as shaded bands whose width
encodes days, clamped), an as-of cursor, and markers from events (transactions, IL) using the count-preserving
marker merge from the audit's extension list. Fan charts use a band primitive: a ribbon between two series with a
min/max LOD merge, drawn three times for 50/80/95.

### 4.4 forecast

**Corpus.** Season lines (batting and pitching, separate shapes) for 1998–2025 from Lahman (Batting, Pitching,
People, Appearances, Teams; SHA-256 pinned), the open season's partial lines from the Stats API, cut lines for the
in-season as-of dates from Retrosheet (see 4.5), and the constants file. The corpus revision is the tuple of input
hashes plus the open-season fetch time.

**Projector interface.** A projector has an id (name + semantic version) and produces, for a player entity and an
as-of date over a corpus view restricted to information dated on or before the as-of, a forecast: a playing-time
distribution, a map of metric → distribution, and an optional reliability share. Distributions are points,
quantile sets, samples or normals, all exposing mean and quantile. Marcel returns points; the harness attaches
intervals. A model that returns its own quantiles or samples skips the harness interval and is scored identically.
The corpus view is the ergonomic API; the enforcement is the harness's perturbation test (4.9).

**Marcel (batting).** Seasons weighted 5/4/3; components (uBB, HBP, 1B, 2B, 3B, HR over the wOBA denominator) summed
with those weights; regression toward the league rate of the same weighted seasons with 1200 PA of league-average
performance; age adjustment multiplicative, (29 − age) × 0.006 below 29 and −(age − 29) × 0.003 above, seasonal age
from birth year and month (July cut); reliability = own-data share of the regressed denominator; projected PA =
0.5·PA₋₁ + 0.1·PA₋₂ + 200. Rest-of-season adds the current partial season as a fifth-weighted term scaled by its PA
and projects remaining PA from the projected-PA formula less PA to date. Pitching: seasons 3/2/1 on innings, +25
(relievers) / +60 (starters), components K, BB, HR, H over batters faced, ERA via earned runs where the corpus has
them and RA9 components otherwise. No park adjustment inside Marcel (the sensitivity drawer applies the DR-3.18
park factor to the displayed rate, not to the scored projection). Verified against Tango's 2001–2012 files as
golden values (bbref-style ids map through the catalog).

**Naive baseline.** Prior season's rate and PA; missing players get the floor convention.

**Playing-time model.** Marcel's PA formula is the model; it is scored on PA MAE with missing players at 100 PA
(Whiffs convention). The sensitivity drawer's override never enters the score.

**Aging.** Marcel's linear adjustment in v1. The later aging ticket compares GAM-across with a career-mean control
and age re-centring against MGL's season-completion method on the same gate; the Lichtman phantom-year correction
is excluded.

**Park factors and constants.** Per-season wOBA linear weights from run expectancy on Retrosheet events; as-of park
factors from past seasons only, regressed 1 − (1 − iPF) · X with X = .6/.7/.8/.9 by seasons available; frozen in a
versioned constants file with its SHA-256. No FanGraphs constants anywhere.

**Walk-forward protocol.** Target seasons 2001–2025; cuts per season: January 1 (preseason) and May 1, June 1,
July 1, August 1, September 1 (rest-of-season); population per cut = every player with a projection or an
appearance in the target season; rates scored as PA-weighted MAE of wOBA after re-centring every system to a common
league mean; missing players at the floor convention (default: Whiffs — league mean of the last training season
with 100 PA; see 7); playing time scored as PA MAE; results bucketed by Marcel reliability (High ≥ 0.7, Med ≥ 0.45,
Low ≥ 0.15, None). Seasons 2001–2003 are interval warm-up.

**Interval model.** Residuals from strictly-earlier folds, standardised as z = err · √PA (actual PA when fitting,
projected PA when forecasting), quantiled per bucket (p05/p10/p25/p50/p75/p90/p95), de-standardised at forecast
time. Buckets below n_min (default 50 residuals) merge upward.

**Scores and gate.** Per model: MAE per season and pooled, PA MAE, 80% coverage per (season, bucket) with n, mean
interval score, and the gate: 0.80 inside the Wilson 95% interval of observed coverage in every scored cell of the
last three seasons, after merges. The [.75, .85] band is reported, not tested. The gate result is part of the
artifact and of the test suite.

**Comparators.** Marcel (in-repo), naive, Steamer 2017–2024 as a committed fixture of aggregate scores (system,
season, metric, value) computed once from the free-registration downloads; ZiPS aggregate-only if a membership
export is ever used, else absent. Rows of any external system are never committed.

**Artifacts and registry.** One JSON document per fit: model id and version, as-of, corpus revision, protocol hash,
constants hash, scores and gate result, every forecast with its quantiles and bucket, creation time, tool version;
file name = content hash of (model id, version, corpus revision, protocol hash, as-of) so re-fitting is idempotent.
An index table in the app store maps (model, as-of) → artifact path and scores. Every projection pane reads from the
artifact and prints version, training window, as-of and last backtest score.

### 4.5 retro

**Parser.** Reads Retrosheet event files (regular season, pinned Chadwick-mirror commit) into per-game, per-player
batting and pitching lines: plate appearances, at-bats, hits by type, walks (intentional separately), hit-by-pitch,
sacrifice flies and hits, strikeouts, runs, RBI, stolen bases; batters faced, outs, hits, home runs, walks,
strikeouts, runs, earned runs (from explicit advancement and unearned markers). Handles the 24 base play-token
families and 23 modifier families measured on 2024, compound K+/W+ plays, `NP` records, extra-inning placed-runner
adjustments, and the earned-run data records. Unknown tokens are counted, logged and never silently dropped.

**Cut lines.** Cumulative lines per player at each as-of cut (through the day before the cut) for 2001–2025;
≈190K rows; bundled with the Retrosheet notice.

**Validation.** Season sums of the parsed lines reconcile with Lahman totals per player-season; the reconciliation
report (count of exact matches, discrepancies by stat, worst cases) is a committed fixture and a test — this is the
official-totals cross-check the Phase 2 record asked for.

**Constants tool.** Run-expectancy matrix per season from the same events → wOBA weights; park factors from
home/away run environments by season and park; both written to the constants file with provenance.

### 4.6 data

**Sources.** Savant (per player per season CSV, both roles), Stats API (open-season lines, transactions by day,
rosters, minor-league season lines by sport id, draft by year, people by id), Cot's (Opening-Day salary sheet per
season via the sheet's CSV export, per-team sheets for contract length). Each source declares a policy: minimum
interval between requests, TTL for closed and for open seasons, maximum requests per user command, attribution
text, and the rule that bulk season pulls are not a request shape at all. Each source is a typed slot on the data
facade, present only when network is enabled.

**Raw store.** Every fetched payload is kept as bytes with provenance (source, URL, fetch time, SHA-256,
attribution), keyed by an explicit per-request key (player/role/season; season; transaction day; team/season;
season/sheet). Range requests decompose into day keys. Payloads are zstd files with an index table in SQLite.

**Normalisers.** Versioned pure functions from payload to domain rows: Savant allowlist (parse by header name,
strip the byte-order mark, reject unknown header sets with a schema error naming the diff, map renamed columns,
assign sequence numbers by the sort rule), Stats API season lines (batting and pitching), transactions (dedupe on
person, date, type code, description), rosters, minor-league lines, draft picks, Cot's sheets (two-row header,
`*` deferral suffix, empty salary = no Opening-Day MLB salary, trailing 40-man block, summary cells ignored, totals
reconciled against the sheet's total cell). A normaliser bump re-runs over the raw store without refetching.

**Row tables.** Events and log revisions; season lines; cut lines; transactions; contracts; rosters; minor-league
lines; draft picks; catalog; artifacts index; tracked entities; alerts; meta. All in one SQLite file in the cache
directory, WAL, upserts, `WITHOUT ROWID` keys where the key is the whole identity. The connection is behind a mutex;
no guard is held across a network await.

**Facade.** Typed asynchronous methods: events for (player, season); season lines for a season; cut lines; transactions
since a date; roster for (team, season); payroll for a season; minor-league lines; draft; catalog lookups; and
refresh with a scope. Every returned value carries provenance and freshness (bundled, cached with age, fetched,
stale-refused). Resolution order: bundled set → row tables → raw store → source (if present and within budget).

**Bundled set.** The Ohtani 2025 event logs (bat 2,940 rows, pit 753 rows), Lahman 1998–2025 subset (five tables),
cut lines 2001–2025, constants file, catalog subset, Lahman salaries ≤ 2016, the Steamer score fixture, and a
manifest with every input's hash and licence notice. Total single-digit MB plus the catalog's ≈14 MB.

**Refresh.** One code path: for each tracked or recently viewed entity fetch what is stale within policy, dedupe and
store transactions since the last refresh, refit tracked players' projections, raise alerts. Invoked by the CLI,
by the worker's interval scheduler, or by a documented systemd user timer.

### 4.7 app

**Grammar.** `<ENTITY>[.BAT|.PIT] <FUNCTION> [ARGS] GO`. Entity: player name, MLBAM id, team abbreviation, or a
rewritten entity key. Functions and arguments:

| Function | Entity | Arguments | Pane |
|---|---|---|---|
| CAST | player, matchup | metric, window | PA/BF/pitch-indexed series with markers and as-of cursor |
| PROJ | player, team | horizon (ROS, next), model | nested ribbons, registry stamp, sensitivity drawer; team = win-total fan |
| SPLT | player | split kind (vs L/R, home/away, pitch type, count), window | split table + series |
| GAME | player, team, game | date or game pk | game log / slate / box |
| ROST | team | season | roster by position, salary (cited), WAR, surplus, gaps vs target |
| FARM | team | level or sport id, season | minor-league lines, draft feed |
| COMP | player | n, metric set | comparables by Statcast profile and age, overlay |
| BTST | model | seasons | MAE table, coverage grid with Wilson whiskers, pooled row |
| ALRT | — | — | alerts pane; `TRACK`/`UNTRACK <entity>` add and remove tracked players |

Bare words resolve in order: function, window label, metric code, entity. Ambiguous names open the ranked picker;
picks rewrite the token to the entity key. The hint line shows the parsed interpretation before GO.

**Panes.** Re-enumerated pane kinds: Series, Projection, Splits, Game, Roster, Farm, Compare, Backtest, Alerts,
Feed (transactions/draft), Watchlist (tracked). Each pane renders its freshness chip and attribution footer.

**Presets.** Morning front-office briefing: tracked-player series, movers, transactions feed, alerts, today's
slate. Prospect watch: FARM table, draft/intl feed, MLB-equivalent projection. Trade deadline war room: roster
table (wide column), WAR-gap bars, comparables, ROS win-total fan with the CBT line and the deadline T-minus.

**Alerts.** Tracked entities; trigger = projection delta beyond the user's threshold since the last refresh, or a
transaction/IL event; one alert per deduplicated event; toasts and the ALRT pane with seen state.

**Persistence.** Config in the XDG config dir, caches (SQLite, raw payloads, artifacts) in the XDG cache dir,
layouts and open state in the XDG state dir; layout and prefs documents carry a new version; upstream migrations are
dropped; unknown codes in a saved layout are discarded with a log line.

**Polling cadence.** No sub-minute polls; refresh on the worker's interval (default daily), on demand, and on
preset open if stale.

### 4.8 cli and report

Subcommands: `fit` (model, as-of, writes an artifact), `backtest` (model, protocol, writes scores and the gate),
`refresh` (scope), `report` (entity → Markdown/HTML one-pager from the latest artifact and series: projection,
interval, bucket, backtest score, as-of, model version, attributions), `build-corpus` (Lahman subset + cut lines
with hashes), `trim-register` (catalog from a pinned register commit), `constants` (constants file from Retrosheet),
`export` (CSV artifacts for R/Python with the documented snippet). Every subcommand prints the hashes it consumed
and produced.

### 4.9 Testing decisions

A good test here proves a consumer-visible behaviour at a seam with real or fixture data, is deterministic, and
fails for the bug it names. No tests of wiring, copies or defaults.

| Feature | Seam | The test that proves it |
|---|---|---|
| Entity identity | model | a two-way player yields two series keys and one catalog profile; keys round-trip through layout JSON |
| Catalog search | model | `DIAZ` returns the ranked list with Edwin Díaz in the first two under the role-neutral weight; `TATÍS JR`, `TATIS JR`, `tatis` resolve to one id |
| Event sequencing | data | a Savant payload whose rows are out of order yields sequence numbers sorted by (date, game pk, at-bat, pitch); duplicate triples are rejected |
| Savant schema drift | data | the 119-column fixture parses; a fixture with one inserted column still parses by name; a fixture with a renamed required column fails with a schema error naming it; the BOM is stripped |
| Coverage gaps | model, chart | bat speed before 2024-04-03 is undefined, never zero; the gap survives the LOD pyramid at level ≥ 2 (pixel test at two zoom levels, asserting a rendered frame — no silent skip) |
| Rolling windows | model | a 50-PA window over events with undefined values counts defined events only; cross-season windows continue across logs; reliability weight equals n/(n+x) for the Carleton table |
| Marcel | forecast | reproduces Tango's 2010 BattingMarcel values (wOBA, PA, reliability) for the full file within rounding; pitcher file likewise |
| No look-ahead | forecast | for every fold, perturbing every corpus row dated after the as-of leaves all projections byte-identical |
| Interval model | forecast | quantiles for fold y use residuals from folds < y only (pooled-vs-nested differ on the fixture, the nested result is the one produced); standardisation reproduces the prototype's per-bucket coverage within 0.02 |
| Gate | forecast | the gate passes on a synthetic calibrated model at p = 0.80 with cells of 150 in ≥ 95% of seeded runs and fails at p = 0.70 in ≥ 95%; cells below n_min merge upward |
| Backtest budget | forecast | the full v1 suite (Marcel + naive, 25 seasons × 6 cuts) completes in < 60 s in the test runner |
| Playing time | forecast | PA MAE on the fixture matches the committed reference; the override changes the pane, not the score |
| Retrosheet parser | retro | 2024 season: every play token accounted (unknown count 0); parsed season sums reconcile with Lahman per player for ≥ 99.5% of player-seasons on PA, H, HR, BB, SO (the report is the fixture); placed runners do not create plate appearances |
| Constants | retro | wOBA weights for a season reproduce the committed values; park factors for season y use no season > y |
| Transactions | data | the four known duplicate rows collapse to two events; an alert fires once |
| Cot's sheets | data | the three-season fixture parses with ≤ 0.1% unparsed; the trailing 40-man block yields "no salary" rows; the parsed sum equals the sheet total cell |
| Fan-out budget | data | a command needing 30 fetches with a budget of 5 yields 5 fetched and 25 stale-refused values and no further requests |
| Provenance | data, app | every value returned by the facade carries source, hash and freshness; the pane shows the attribution text for Cot's |
| Artifacts | forecast | re-running a fit with the same inputs produces the same file name and byte-identical content; a changed constants file changes it |
| Grammar | app | the parse table: `OHTANI.PIT PROJ ROS`, `660271 CAST XWOBA 50`, `NYM ROST`, `DIAZ CAST` (picker), `BTST MARCEL`, `TRACK OHTANI` |
| Layouts | app | the three presets load on the bundled set and render a frame each in the smoke run; a layout containing an upstream finance code loads without it |
| Report | cli | the one-pager for the bundled player contains the model version, as-of, backtest score and every attribution string |
| Offline | cli, app | with network disabled, Ticket 01's path (bundled player → projection → fan chart) and all three presets run; the smoke run logs frames |
| CI | repo | the Linux job asserts the render tests did not skip (adapter line present) and greps the smoke line |

Fixture rule: captured real payloads (Savant game and player-season CSVs, Stats API JSON, a Cot's export, Retrosheet
team files) are committed small and named by source and date; network tests are ignored by default.

## 5. Out of scope (v1)

Live in-game mode; a Python or R runtime; Parquet (until the pitch-level corpus); MiLB Statcast; amateur tracking
data; a networked service; Flatpak/AppImage; FanGraphs-sourced anything; a second model family (xwOBA-informed,
hierarchical Bayes, survivor-corrected aging) — each a named v1.1 ticket; committed rows from any external
projection system; macOS.

## 6. Open risks

Licensing: the committed Statcast slice and runtime Savant/Stats API fetches rest on de-facto tolerance plus the
copyright notice's individual/non-bulk grant, not on conformity with the MLB terms' automated-access clause; Cot's
sheets are used under Baseball Prospectus's noncommercial fair-use wording with citation and no explicit
redistribution grant; Lahman's share-alike binds the bundled derived corpus. Source fragility: Savant export
columns change without notice (four count changes in 26 months, one error-shape payload); Cot's sheet ids and
layout are hand-maintained; the Chadwick register updates monthly with a documented gap; Retrosheet's own site is
intermittently unreachable (the mirror is the pinned source). Methodology: regime years (2019 ball) fail the
calibration gate for every system; pitcher earned-run parsing may not reconcile; the Steamer id join is unverified
until the fixture ticket. Performance: view-time derivation over league-wide windows is 5 ms per derive on the
bench but untested in the GPU frame; the band primitive is new shader work. Process: upstream moves daily and
cherry-picks are manual after the vocabulary swap. Full list with owners: `06-risks.md`.

## 7. Not yet specified

- The missing-player floor convention for the rate MAE: default Whiffs (league mean of the last training season,
  100 PA); Tango's fixed .315 and THT's mean − .020 are alternatives; the comparator fixture must state which it used.
- `n_min` for bucket merging (default 50) and the reliability bucket edges (0.7 / 0.45 / 0.15) — defaults from the
  prototype, to be fixed by the first real backtest.
- The role-neutral weight for picker ranking (search frequency vs a WAR proxy).
- The minor-league level set shown by default in FARM and whether winter leagues appear.
- The exact Savant allowlist (the 119 current columns minus deprecated ones) and the rename map — fixed when the
  first fixture is captured.
- Whether pitcher rest-of-season uses ERA or RA9 components — decided by the parser's reconciliation report.
- The count-preserving marker merge's visual (stack vs count badge) at coarse zoom.
- The register and Lahman refresh procedure (who bumps the pinned commit, how the reconciliation report is reviewed).
- Steamer fixture join key and unmatched-player accounting.
- The 7-day Savant churn result (re-fetch due 2026-10-28) and its effect on cache TTL for the open season.
