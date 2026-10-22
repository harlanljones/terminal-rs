# Extension points (inherited terminal-rs code, as of `b81f40b`)

The "add a feature" checklists an agent follows in the inherited code. Line numbers are hints as of `b81f40b`;
re-locate with `grep` before editing. Verdicts on what survives the fork live in `docs/plan/00-audit.md`.

## New panel kind

`PaneKind` variant (app.rs:46-77) + its six match fns (:165-330: `tab_title`, `code`, `symbol`, `follows_symbol`,
`settings`, `from_settings`) → `PaneSettings` variant (layouts.rs:58-117) → `render_content` arm (panels.rs:34-116)
→ `init_kind` (workspace.rs:368) if it loads data; `set_pane_symbol` (:457) if symbol-driven → `panel_from_spec`
word (:656) → optional preset filler (:716) → `prefs::Action` + `Keymap::default` (:570) + `run_hotkey`
(app.rs:1280) + `palette_view::COMMANDS` (:23-93) → test `pane_settings_round_trip_through_kinds` (app.rs:1605).

## New function code

`Function` variant (command.rs:210-278) → `parse_function` arm (:284-379; fallback order :361-376 matters) →
`describe` (:430, powers the live hint) → `dispatch`/`apply_with_symbol` (app.rs:748-926; symbol-prefixed form needs
a `Command::SymbolFunction` arm :751-782) → `palette_view::COMMANDS` → HELP text in `hotkeys_view.rs` → tests
`functions`/`hints`.

## New worker request

`Request` variant (worker.rs:29-126) → `Response` variant (:200-280) → `handle` arm (:482-699; each request is its
own tokio task over `Shared`) → `drain` arm (app.rs:935-1085) → sender in `periodic` (:1333) or a pane method.
Series generations are guarded by `claim_series/store_series` (:742-770).

## New study (indicator)

One line in `registry::catalog()` (registry.rs:434) using the private builders (`n`, `out`, `Def` helpers around
:348-430); the metric arrives through `Bar.close` via `Input::c()` — there is no named-metric column. What the tests
enforce (registry.rs:1172-1316, must stay green): codes uppercase/unique/not a catalog ticker, categories contiguous
and non-empty, `range`/`levels` only on panes, overlays never `Plot::Hist`, `values.len() >= bars.len()`, outputs not
entirely blank and never infinite, values inside `range` (sweep :1242-1277), no panics on empty/1/2/flat inputs
(:1300-1316). What they do **not** enforce: the NaN warm-up prefix ("positions without enough history hold
`f64::NAN`", indicators/mod.rs:1-4) — a study that emits finite values before its window fills passes the sweep, so
add a study-specific warm-up assertion when the convention matters.

## New data provider

Implement `MarketData` (lib.rs:209-282; `name` and `history` are the only required methods, the other seven have
default bodies — `quotes` and `yield_curves` derive from `history`, `live`/`search`/`news` return empty,
`options`/`order_book` bail), wrap in `Fallback::new` (:293-298, impl :301; note it substitutes sample bars only for
a failed *first* page and the worker merges later real pages onto them without a provenance flag — gotchas.md),
register in `from_env` (lib.rs:387-404).
Fixture tests via `include_str!` of a captured response
(see `yahoo.rs`/`crypto.rs`/`treasury.rs` unit tests); network tests go under `#[ignore = "network"]`
(`tests/live.rs`).

## New ingest source

`congress::refresh::Source` (RPITIT, `Sync`; refresh.rs:21-27) is a five-method congressional contract
(`rosters`, `house_index`, `house_pdf`, `senate_reports`, `senate_ptr`) and `refresh()` (:100-138) hard-wires
`refresh_house`/`refresh_senate` over it, so a new kind of source means changing the trait and `refresh()`
together, reusing `store()` (:140-198) and `RefreshConfig`. Then wire it: the worker never injects a `Source` —
`congress_refresh` (worker.rs:384-391) constructs `HttpSource::new()` itself and is the single path for the
scheduler (:373) and `Request::RefreshCongress` (:642); replace that construction (or add a selector) or the new
source never runs. `Response::NewAlerts` → toasts (app.rs:1066-1069) and the scheduler need no change. Copy
`tests/refresh.rs` (:19-47) fixture-backed `Source` for the offline test — it calls `refresh()` directly, which is
exactly why it passes without the worker wiring.

## New chart primitive

`gpu.rs` pipeline + `DrawCmd` kind → WGSL vertex fn in `shader_line.wgsl` (the shared prelude is the part of
`shader.wgsl` before `// ---- bars`, concatenated at gpu.rs:222-224) → pyramid in `lod.rs` (`LinePyramid::reduce`
:147-162 keeps one point per pair; `build_markers` :143-145 drops the earlier of two finite markers; a band needs
min/max merge, dense markers need a count-preserving merge) → scene builder in `scene.rs` (`overlay_cmds` :235,
`sub_pane` :281; the main pane `price_pane` :179 is typed on `SeriesGpu`/`ChartType`) → pixel test in
`tests/render.rs` (`Headless::new(800,400)`, `TR_DUMP_DIR` dumps PPMs).
