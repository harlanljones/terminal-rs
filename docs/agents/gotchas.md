# Gotchas (inherited terminal-rs code, forked at `b81f40b`; sabr additions 2026-10-22)

Facts an agent needs before touching the inherited code. Line numbers are hints as of `b81f40b` unless marked sabr.

## Build/run

- Edition 2024 with let-chains (stable since Rust 1.88). **sabr: `workspace.package.rust-version = "1.95"` is now
  declared** (was 1.85, below the locked egui/eframe/egui-wgpu/epaint 0.36.2 floor of 1.95). No
  `rust-toolchain.toml`. This machine: Arch `rust 1:1.99.1-1` (no rustup; the package tracks latest stable, so a
  pin file would not be honoured).
- Baseline commands (sabr state, 2026-10-22): `cargo build --release --workspace --all-targets`,
  `cargo test --release --workspace` (**359 passed, 5 ignored**), `cargo clippy --release --workspace --all-targets
  -- -D warnings`, `cargo fmt --all -- --check` — all green on branch `sabr`.
- **sabr crates** (new, on `sabr`): `crates/model` (`sabr-model`: entity/role keys, event logs, metric registry
  with coverage intervals and Carleton thresholds, series derivation), `crates/bdata` (`sabr-data`: Savant
  normalizer, catalog, transactions, raw payload store, XDG dirs, bundled-set loader), `crates/forecast`
  (`sabr-forecast`: Marcel batting golden-tested, walk-forward harness, PA-standardised intervals, Wilson gate,
  hashed artifacts). Bundled sample: `sabr-data/sample` (26 MB, manifest.sha verified at load).
- Network tests: `cargo test -p tr-data -- --ignored` (5 tests, hit Yahoo/Binance/Coinbase/Treasury).
- Smoke (sandboxed, preferred): `./target/release/terminal-rs sabr smoke --frames 3` renders every pane
  of the default workspace and all three presets with **no window and no display server** (egui `run_ui`
  passes; asserts non-zero clipped shapes per stage). Safe on any machine.
- Smoke (windowed): `TR_PROVIDER=sample TR_CONGRESS_REFRESH_MIN=0 TR_SMOKE_SECS=6 TR_DATA_DIR=$(mktemp -d)
  env -u WAYLAND_DISPLAY ./target/release/terminal-rs` → `window + GPU ready …` then `smoke: N frames in …`.
  **Never run the windowed path against your own session on this machine: it crashes the compositor**
  (observed 2026-10-22 — the Wayland/XWayland path destabilizes the WM; the process then exits in 3–5 s
  without logging `smoke:`). It reproduces on pristine `b81f40b`, so it is an upstream defect, not a fork
  regression. Verify headlessly (`sabr smoke`) or in CI under `xvfb-run` (the Linux job already does).
- The windowed smoke block lives at app.rs:1706-1724 (was :1545 at `b81f40b`).
- `crates/chart/tests/render.rs` returns `Ok` and prints `no wgpu adapter; skipping` when `Headless::new` finds no
  adapter — run with `-- --nocapture` and look for `adapter: "…"` to know it rendered.
- `tr-broker/tests/throttle.rs` sleeps for real (22 s measured) because of global throttle statics
  (snaptrade.rs:47-50).
- rustfmt: `max_width = 120`, `use_small_heuristics = "Max"` (`rustfmt.toml`). Release profile keeps `debug = 1`
  (234 MB binary); `profile.dev.package."*" opt-level = 2`.

## Env vars

| Var | Read at | Effect |
|---|---|---|
| `TR_PROVIDER` | crates/data/src/lib.rs:391 (`from_env`) | `sample` → offline `SampleProvider` with live ticks; the app also gates UI on `provider.name() == "sample"` (app.rs:572, chrome.rs:104, panels.rs:65) |
| `TR_DATA_DIR` | app/src/worker.rs:310 (`data_dir`) | overrides the data directory |
| `TR_MSAA` | app/src/main.rs:32 | 1/2/4/8 samples; default 4 on macOS, 1 elsewhere |
| `TR_THEME` | app/src/app.rs:563 | `light`/`dark`/theme name overrides `prefs.appearance.theme` at startup |
| `TR_FPS` | app/src/app.rs:635 | presence enables the perf overlay (`show_perf`) |
| `TR_VIBRANCY` | app/src/app.rs:1410 | `0` disables macOS sidebar vibrancy (macOS-only block) |
| `TR_CONGRESS_REFRESH_MIN` | app/src/worker.rs:362 (scheduler), app/src/app.rs:1090 (status text) | minutes between Congress refreshes; default 30 (`CONGRESS_REFRESH_MIN`, worker.rs:27); `0` disables the loop |
| `TR_HISTORY_LIMIT` | app/src/worker.rs:479 (`initial_limit`) | bars requested on first load instead of `default_limit(tf)` |
| `TR_SAMPLE_BARS` | crates/data/src/sample.rs:24 | sample depth, default 20 000; regenerated on every call |
| `TR_SMOKE_SECS` | app/src/app.rs:1545 | render N seconds, log `smoke:` stats, autosave, quit |
| `TR_BROKER` | app/src/app/broker.rs:115 | `sample` → simulated broker without credentials |
| `TR_SECRET_STORE` | crates/broker/src/secrets.rs:121 | `file` forces the file backend instead of the macOS keychain |
| `TR_DUMP_DIR` | crates/chart/tests/render.rs:50,127,164 | render tests dump PPMs there |
| `SABR_SAMPLE_DIR` | crates/bdata/src/bundled.rs (`sample_root`) | overrides the bundled-sample root for tests (default: repo `sabr-data/sample`) |
| `SABR_CONFIG_HOME`, `SABR_CACHE_HOME`, `SABR_STATE_HOME` | crates/bdata/src/xdg.rs (`XdgDirs`) | per-class overrides; else `XDG_*` else `$HOME/.config|.cache|.local/{state,share}/sabr-rs` |
| `APCA_API_KEY_ID`, `APCA_API_SECRET_KEY`, `APCA_DATA_FEED` | crates/data/src/lib.rs:394-396 | Alpaca provider inside `Markets`; feed defaults to `iex` |
| `SNAPTRADE_CLIENT_ID`, `SNAPTRADE_CONSUMER_KEY`, `SNAPTRADE_USER_ID`, `SNAPTRADE_USER_SECRET` | crates/broker/src/secrets.rs:19-22 via `Credentials::apply_env` (app/src/app/broker.rs:114) | override stored credentials |
| `SNAPTRADE_BASE_URL` | crates/broker/src/snaptrade.rs:255 | API base override |
| `RUST_LOG` | app/src/main.rs:26 (`EnvFilter::try_from_default_env`) | default filter `info,wgpu_core=warn,wgpu_hal=warn,naga=warn` |
| `XDG_DATA_HOME`, `HOME` | app/src/worker.rs:313-320 | data-dir fallback chain |

## Platform

- `tr-macos` is an **unconditional** dependency (app/Cargo.toml:25) compiled on Linux through no-op stubs
  (`false`/`None`). `chrome.rs:23,25` read `tr_macos::TITLEBAR_HEIGHT`/`TRAFFIC_LIGHTS_WIDTH` inside `const`
  expressions (`TOOLBAR_H`, `TRAFFIC_INSET`; Linux arms `34.0`/`0.0`). Other call sites: app.rs:508, 548, 864-865,
  1407-1437, 1457, 1468, 1471.
- `cfg!(target_os = "macos")` branches: main.rs:36,43; prefs.rs:475-570 (Ctrl↔Cmd mapping); chart_view.rs:1057;
  broker.rs:161; chrome.rs:29,543; settings_view.rs:250,570; workspace.rs:968; worker.rs:314; secrets.rs:122.
- eframe is built with both `wayland` and `x11` features (app/Cargo.toml:24); both paths verified on Hyprland
  (`docs/plan/00-audit.md` §6). The X11 path guesses a 1.083× scale factor on this machine.
- Data dir on Linux: `$TR_DATA_DIR` else `$XDG_DATA_HOME/terminal-rs` else `~/.local/share/terminal-rs`
  (worker.rs:309-321).

## Chart

- `gpu.rs:222-224` builds the line shader by splitting `shader.wgsl` at the literal comment `// ---- bars`
  (shader.wgsl:93) and prepending that prelude to `shader_line.wgsl`; moving or renaming the comment breaks every
  line pipeline at startup.
- `Uniforms` layout is positional across `gpu.rs` and both WGSL files.
- `overlay_cmds` (scene.rs:236) and `sub_pane` (:282) take the index length from `overlays.first()` and
  `Viewport::visible_range` clamps `hi` to it (viewport.rs:64): in a sub-pane a shorter first overlay clips the
  others on the right (the price pane widens the bound by `v.right`).
- `SLOTS = 4096` uniform slots per egui frame across all panes (gpu.rs:17, counter reset at :370); draw commands
  past that are silently dropped (`break` at :376-378).
- GPU frame ids: price pane `chart_id*8`, sub-pane `chart_id*8 + 1 + k`; `MAX_SUB_PANES = 6` (chart_view.rs:33-35).
- `Frame.frame_no` must equal egui's `cumulative_frame_nr()` (chart_view.rs:819); buffers are evicted after 600
  unused frames (`EVICT_AFTER`).
- Mutating a series in place is invisible to the renderer: give it a new `key` or bump `version`.
- Marker LOD is lossy: `build_markers` (lod.rs:143-145) keeps the earlier point of a pair only when the later one
  is NaN, so two finite markers in one LOD group collapse to the later one (`[5.0, 7.0]` → `[7.0]`). Sparse
  markers survive zoom-out; adjacent ones vanish — do not rely on it for per-PA/per-pitch events.

## Core

- Indicator builders (`n`, `out`, `Def` helpers, registry.rs:348-430) and `Input::c()` are private; a new study
  must live inside `registry.rs`.
- Catalog invariants (unique upper-case codes, contiguous categories, `range`/`levels` only on panes, no `Hist`
  overlays, `values.len() >= bars.len()`, non-blank and finite outputs, values inside `range`, no panics on tiny
  inputs) are enforced only by tests (registry.rs:1172-1316). The NaN warm-up prefix is **not** tested — it is a
  convention from indicators/mod.rs:1-4; a study must assert its own warm-up if it matters.
- `Spec` deserialises from its label and keeps unknown codes (registry.rs:275-280): stale codes in saved layouts
  survive silently.
- Two different `CATALOG` statics: `tr_core::catalog::CATALOG` (`Vec<Listing>`, pub, catalog.rs:151) and the private
  `indicators::registry::CATALOG` (`Vec<Def>`, registry.rs:227).
- Indicator params are positional `f64`s; `Spec::new`/`Spec::parse` clamp and round them through `Def::normalize`
  (registry.rs:199-207), but `compute` reads `p[k] as usize` directly (`n(p, k)`, :348-350) — calling a `Def`'s
  `compute` with hand-built params bypasses the clamp.

## Data/worker

- `MarketData::live()` is sync and calls `tokio::spawn` (sample.rs:77, crypto.rs:366): it panics without an
  ambient tokio runtime.
- `from_env` installs the global rustls `aws_lc_rs` provider (lib.rs:389); call it once.
- `BarCache::load(symbol, tf, limit)` has no `end` bound (cache.rs): the cache serves the first page only; every
  older page goes to the network (worker.rs:541).
- `Fallback::history` serves synthetic sample bars when the *first* page fails, logging only a `warn`
  (lib.rs:311-314); an older-page failure (`end.is_some()`) propagates instead (:309-310). That is the whole
  guarantee: if the primary recovers on the next older page, `worker.rs:541-551` prepends real bars onto the
  synthetic series with no provenance check, and `Response::History` carries no flag for it.
- Series loads are generation-guarded by `claim_series`/`store_series` (worker.rs:742-770); a response for an older
  generation is dropped, not merged.
- `theme::CURRENT`/`SCALE`/`DENSITY` are thread-locals (theme.rs:50-53) that start as the dark palette with the
  default accent and scale 1.0; only `theme::configure` (theme.rs:249) on the UI thread sets the real theme, so
  palette reads from another thread (the worker) or in headless tests see the defaults.

## Congress/broker

- `Db` is a sync rusqlite connection behind `tokio::sync::Mutex` (worker.rs `Shared.db`); hold the guard briefly.
- The refresh loop (worker.rs:361-375) has no cancellation token; it lives until the runtime drops.
- There is no injection point for a `Source`: `congress_refresh` (worker.rs:384-391) builds `HttpSource::new()`
  itself and is the only path for both the scheduler (:373) and `Request::RefreshCongress` (:642). Implementing the
  trait alone changes nothing at runtime; `tests/refresh.rs` works because it calls `refresh()` directly.
- `trades.id` is unstable across re-ingest (rows deleted and reinserted per filing); `alerts.filing_id UNIQUE` means
  one alert per filing.
- `pdf-extract` panics on some filings; `house::pdf_text` wraps it in `catch_unwind` — never call the crate directly.
- Global throttle statics (snaptrade.rs:47-50) make `tests/throttle.rs` sleep ~22 s real time.

## Codebase graph

Indexed in codebase-memory as project **`sabr-rs`** (mode `moderate`, root `/home/harlan/dev/sabr-rs/terminal-rs`,
indexed 2026-10-20T18:58:29Z): **3,962 nodes, 22,267 edges**, status `ready`. Excluded by design: `.git`, `docs`,
`scripts`, `target`, `site/scripts` (12 dirs), `Cargo.lock`, lockfiles and images; 10 `site/**/*.svelte` files
parsed partially. No `.codebase-memory/` artefact is written into the tree. The index is not auto-watched (not the
MCP session project): re-run `index_repository` after large edits.

- Symbols: `search_graph` — `{"project":"sabr-rs","query":"claim_series"}` →
  `sabr-rs.app.src.worker.claim_series Function app/src/worker.rs 742-752`.
- Relationships: `trace_path` — `{"project":"sabr-rs","function_name":"sabr-rs.app.src.worker.claim_series",
  "direction":"inbound"}` → 7 callers (`handle`, `Worker::spawn`, `TerminalApp::layout_op`, `main`, …).

## Learned failures

(empty — add an entry only when a mistake repeats)
