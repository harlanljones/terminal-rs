# 06 — Risks: licensing/ToS, source fragility, performance, unresolved fog

Derived from the decision records (`01-grill.md`), the arena (`02-design-arena.md`) and the spec's open-risk and
not-yet-specified sections (`03-spec.md` §6–7). Each row names the accepted exposure, the evidence, the mitigation
that is already decided, and the trigger that would reopen the decision. Dates are the day the evidence was read.

## 1. Licensing and terms of use

| # | Exposure | Evidence (read 2026-10-20/21) | Decided mitigation | Reopen when |
|---|---|---|---|---|
| L1 | Upstream `terminal-rs` has no licence; sabr-rs is a public detached mirror with attribution | GitHub `license: null`, no LICENSE file, no `license` key | D0.2: proceed public with attribution, upstream credited in README | upstream adds a licence (adopt its terms) or objects |
| L2 | Committed Statcast slice (Ohtani 2025, both roles, ≈3.7K rows) exceeds the MLB terms' "one copy … personal, non-commercial home use" | MLB ToU §1 ¶2; gdx copyright notice "individual, non-commercial, non-bulk use" | DR-3.1/3.16: one player-season only; fetch script shipped as the fallback; prior art `statcast-lakehouse` did the same publicly | any objection from MLBAM/Savant → remove the slice, Ticket 01 runs after a documented fetch |
| L3 | Runtime Savant / Stats API fetches are a programmatic client under ToU §1(xi) ("automated scripts to collect information"); no user-initiated or non-bulk exception exists in the terms | §1(xi) verbatim; "non-bulk" appears only in copyright.txt | DR-3.6: per-entity, on demand, cached locally, never bulk, per-source fan-out budget; wording in docs is "de-facto tolerance plus the copyright notice's grant", never "conforms" | enforcement or rate limiting → import-only default with fetch as documented opt-in |
| L4 | Cot's Contracts sheets used under Baseball Prospectus's fair-use wording ("noncommercial purposes such as research", cite author and source); no explicit redistribution grant; "legal action against anyone who misappropriates … for commercial use" | BP terms and conditions; sheet export verified | DR-3.25: runtime fetch only, cached, cited in every pane that shows money; no Cot's rows committed; Lahman ≤ 2016 bundled; manual-import path kept | BP restricts the sheets or objects → manual import only |
| L5 | Lahman CC BY-SA 3.0 share-alike binds the bundled derived corpus (not the code) | readme §0.1; legalcode §1(h), §4(b) | DR-3.17/3.30: the bundled corpus is distributed under BY-SA with attribution; code licence separate | nothing foreseeable |
| L6 | Retrosheet-derived cut lines and constants require the Retrosheet notice verbatim | retrosheet notice (Chadwick mirror README) | DR-3.37/3.18: notice in the bundled manifest and in `report` output | nothing foreseeable |
| L7 | Chadwick register subset is ODC-By 1.0 | register README | DR-3.29: attribution in manifest; pinned commit SHA | nothing foreseeable |
| L8 | Steamer historical files have no licence text; FanGraphs ToS forbids non-interface access and redistribution; Spotrac/USA Today/ESPN/B-Ref forbid automated extraction | steamerprojections.com ("Copyright … All Rights Reserved"); FG ToS §6/§11/§17; Spotrac ToS; Gannett ToS; Disney ToS; SR §5 | DR-3.11: aggregate scores only as a committed fixture; no FanGraphs data anywhere; none of the forbidden sources are fetched | a licensed WAR or payroll feed appearing |
| L9 | Lahman's Negro League tables are "licensed from Seamheads.com" with no terms in the readme | SABR page | not bundled (corpus is 1998–2025) | the corpus window widening before 1998 |

## 2. Source fragility

| # | Exposure | Evidence | Decided mitigation | Reopen when |
|---|---|---|---|---|
| S1 | Savant export columns change without notice | baseballr: 92→94 (2024-05), 94→113, 113→116, 116→119 (2026-06 `miss_distance`); a 12-column error payload (#390); misalignment (#416) | parse by header name against a pinned allowlist; schema error names the diff; error-shape payloads are rejected, not parsed; raw bytes retained for re-normalisation | a renamed required column → allowlist bump + fixture |
| S2 | Savant row order is not pitch order; `sv_id` empty; `pitch_type` may be re-reviewed | probe 2026-10-21 (game 777288) | sequence by (date, game pk, at-bat, pitch); 7-day churn re-fetch due 2026-10-28 sets the open-season TTL | churn > 0 rows → TTL shortened, log revision drives invalidation |
| S3 | Feature coverage is non-contiguous (bat tracking 2023-H2 partial, 2024-03-20→04-02 hole) | Savant leaderboard notes; CSV probes | coverage intervals per metric; undefined never zero; gap-preserving LOD | a new metric with its own epoch → interval added |
| S4 | Cot's sheet ids and layout are hand-maintained; `gviz` export drops a header cell; an unmarked trailing 40-man block | C-F3 probe: 1 unparsed of 3,394 rows | `export?gid=` route; parser rules in the spec; totals reconciled against the sheet's Total cell; degrade to "unparsed" rows, never a failed fetch | > 1 fix per season → manual import becomes the default |
| S5 | Chadwick register cadence: weekly through 2025-12, gap to 2026-05, monthly since; `key_person` reusable | commit log; README | pinned commit; `key_uuid` only; `trim-register` is a one-command refresh | nothing foreseeable |
| S6 | Retrosheet's own site times out; the mirror is the source; event files are large per season | probes (3 timeouts); mirror listing | mirror commit pinned; only derived cut lines and constants are bundled | mirror disappears → vendor the derived files only (already the case) |
| S7 | Stats API transactions feed returns byte-identical duplicates; docs portal is Okta-gated; rate limit undocumented | probe (Sale 852022/867165, Lewis 851742/901976) | dedupe on (person, date, type code, description); per-source minimum interval | duplicates with differing text → dedupe key widened |
| S8 | Upstream `terminal-rs` pushes daily; cherry-picks are manual after the vocabulary swap | repo history | D0.3 hard fork; cherry-pick only; budgeted per ticket list | upstream refactors the chart engine → re-audit |

## 3. Methodology

| # | Exposure | Evidence | Decided mitigation | Reopen when |
|---|---|---|---|---|
| M1 | Regime years (2019 ball) fail the calibration gate for every system | prototype: Low/None coverage 0.67 in 2019 | the gate is honest; reported per season; no tuning to pass | a defensible year-effect model (v1.1) |
| M2 | Pitcher earned runs from event parsing may not reconcile with Lahman | parser not yet built; 1,602 `(UR)` plays in 2024 | reconciliation report is a fixture and a test; fallback RA9 components (DR-3.37) | reconciliation < 99.5% on ER |
| M3 | Steamer id join unverified (files need registration) | B-F3 | deferred to the comparator-fixture ticket; unmatched players accounted explicitly | — |
| M4 | Bucket cell sizes (76–501 players) make a fixed band flaky | prototype; adversary B-obj3 | DR-3.12a Wilson gate, upward merge | cells ≥ 3 SE wide → fixed band returns |
| M5 | Marcel prototype MAE (0.033) is above the published bar (.027–.029) | prototype used fixed weights and all batters | real constants (DR-3.18), population rules (DR-3.12), re-centring; the bar is a target, not a gate | the real harness still above .030 → investigate population and weights before any new model |
| M6 | No-look-ahead is enforced by a test, not by types | adversary B-obj2 | perturbation test per fold in the suite | — |

## 4. Performance and platform

| # | Exposure | Evidence | Decided mitigation | Reopen when |
|---|---|---|---|---|
| P1 | View-time derivation over league-wide windows inside the GPU frame | bench: 1.7 ms at 180K, 4.9 ms at 700K events (naive ring) | derive on the worker thread; no memo in v1 | frame drops in the smoke run → memo keyed by (entity, metric, version, window, log hash) |
| P2 | Band primitive (fan ribbons) is new shader work; LOD merge must be min/max | audit §4.1 | pixel test at two zoom levels; quantile lines are the interim if it slips Ticket 01 | — |
| P3 | Headless render tests skip silently without an adapter; `vulkan-swrast` not yet probed | probe: pointing at a missing ICD → 4 tests "skipping" and passing | CI asserts the adapter line; lavapipe probe pending the package install | — |
| P4 | Toolchain floor misdeclared (1.85 vs real 1.95); Arch has no rustup | audit; registry manifests | Phase 8 declares `rust-version = "1.95"`; CI on stable + 1.95 | — |
| P5 | Windowed run destabilises the local compositor: the Wayland/XWayland path crashed the user's window manager (2026-10-22); the process then exits in 3–5 s without logging `smoke:` | Reproduces on pristine `b81f40b` → upstream defect, not fork code | Verification is **headless**: `sabr smoke` (no window, asserts shapes per preset) + `tests/render.rs` (offscreen wgpu) + CI's `xvfb-run` job; never launch the GUI against a live session from an agent | A fix upstream, or a reproducer that isolates winit/egui vs the compositor |
| P6 | Catalog subset ≈14 MB in the repo; every `cargo install --git` clones it | measured | accepted (DR-3.29); zstd if it grows | > 25 MB |

## 5. Unresolved fog (from `03-spec.md` §7)

Floor convention for the rate MAE (default Whiffs); `n_min` and bucket edges (prototype defaults); picker weight;
FARM default levels; the exact Savant allowlist and rename map; ERA vs RA9 for pitcher ROS; marker-merge visual;
register/Lahman refresh procedure; Steamer join key; 7-day Savant churn result. Each is resolved by the ticket that
first needs it and recorded as a deviation in the implementation notes.
