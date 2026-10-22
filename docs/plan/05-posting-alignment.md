# 05 — Posting alignment: 2026 front-office requirements → sabr-rs evidence

Anchors: the live postings recorded in the Phase 2 record (review-corrected) — **Rockies Senior or Lead Analyst
(Forecasting)** ([2195095](https://www.teamworkonline.com/baseball-jobs/colorado-rockies/colorado-rockies-jobs/senior-or-lead-analyst-forecasting-2195095),
applications "must be received by October 30, 2026"; Senior $110–140K, Lead $130–150K), **Orioles Data Scientist**
($90K), **Mets data roles** (3 postings, $80–200K), **Blue Jays Hitting Analyst** ([2195067](https://www.teamworkonline.com/baseball-jobs/toronto-blue-jays-jobs/toronto-blue-jays/hitting-analyst-2195067),
$80–100K), **Rays Amateur Data Technician**. Recording text lives in the Phase 2 record; DR-3.13 resolved its
capture. Quotes below are verbatim from those captures.

Legend: ✅ = implemented and test-proven (the ticket is cited); 🔶 = landed with a documented substitution or
partial; ⬜ = not yet scheduled (named gap).

## Rockies — Senior or Lead Analyst (Forecasting) [the anchor posting]

| Requirement (verbatim) | sabr-rs evidence | Status |
|---|---|---|
| "Building, testing, documenting, and deploying predictive models and forecasts with a high degree of autonomy" | native models + versioned CLI (`sabr fit`, `backtest`); artifacts carry model id/version/training window/as-of/backtest score (DR-3.4, T11, T14) | ✅ |
| "Communicating findings, uncertainty, and practical implications" | 50/80/95 ribbons ground in backtest residuals (DR-3.21a/3.40); one-pager `sabr report` (DR-3.32) | 🔶 (report CLI ✅ T14; fan-ch ribbons render — DR-3.40 pending pane wiring) |
| "Reproducible work and honest communication" | content-hashed artifacts, pinned inputs (Lahman/Retrosheet commit, constants, register SHA), no-look-ahead perturbation test, calibration as acceptance test (DR-3.22, 3.7) | ✅ |
| "deploying predictive models" | nightly refresh via the same CLI (`refresh`, systemd timer documented) | 🔶 (refit runs; network fetch is Wave 8) |
| "Strong practical ability with R or Python, SQL, reproducible workflows, and model evaluation" | approachable artifacts: SQLite stores + CSV/JSON artifacts; SQL exercised throughout (RawStore, track store, artifact index) | 🔶 (CSV/JSON export exists; R snippet doc pending) |
| "honest uncertainty" + anticipation of evaluation | PA-weighted MAE convention, PICP-per-cell with Wilson whiskers, merged small cells (DR-3.12/3.12a) | ✅ |
| "portfolio, GitHub, writing sample (highly suggested)" | this repo: tests + specs + decision log (docs/plan/) | ✅ |
| "clear communication with credibility across highly technical to nontechnical audiences" | `report` Markdown/HTML + terminal panes | ✅ |
| "at least 4 years… model development, validation, reproducibility" | n/a (the person), but the repo demonstrates the artifacts asked for | ✅ |

Rest-of-season (the brief's own phrase): **in-season as-of folds landed 2026-10-22** — 147 scored folds over
May–Sep cut lines, pooled in-season MAE 0.02534 vs preseason 0.02815; ROS projections with model stamps render
from the artifact registry. One honest finding: the calibration gate goes red with in-season folds pooled
(2025/Low over-covers, PICP 0.877) — reported per DR-3.12a, operator decision pending (T18/note).

## Orioles — Data Scientist, Baseball Analytics

| Requirement | Evidence | Status |
|---|---|---|
| "production data science" (not notebooks) | CLI + nightly artifacts | ✅ DR-3.4, 3.27 |
| "model validation" | per-season MAE table + gate, exit-code CLI | ✅ DR-3.7, 3.12a |
| "Bayesian methods, and probabilistic programming (e.g. Stan, PyMC)" — a listed DU| distribution-valued`Projector` interface (Forecast{pa,rates}: Distribution::{Point,Quantiles,Normal}) ready for a Bayesian family | ⬜ (interface ready — DR-B-F1; no Bayesian model scheduled for v1) |
| "modern AI tools and agents" | out of scope for a desktop terminal; the doc is the deliverable | ⬜ (explicit) |

## Mets — Data Scientist / Senior DS / Applied AI Engineer / Associate Analyst

| Requirement | Evidence | Status |
|---|---|---|
| "modeling at scale… R/Python + SQL" | MySQL-scale columns across 25 seasons×6 cuts in SQLite; R/Python via export; R snippet pending | 🔶 |
| "present to non-technical audiences" | `sabr report` one-pager | ✅ DR-3.32 |
| "agentic workflows with guardrails / MCP-style connectors" | out of scope (desktop terminal) | ⬜ |

## Blue Jays — Hitting Analyst

| Requirement | Evidence | Status |
|---|---|---|
| "translating biomechanical information into actionable insights" | bat-tracking columns in the registry (bat_speed, swing_length, attack_angle/direction, swing_path_tilt) with the measured coverage intervals; SPLT panels | 🔶 (registry ✅; SPLT pane ⬜ pending) |
| "model-building, reports, and visualizations" | same CLI/artifact/report paths | ✅ |

## Rays — Amateur Data Technician

| Requirement | Evidence | Status |
|---|---|---|
| "data-delivery tracking, duplicate/erroneous data checks, query optimisation at volume" | the retro parser's loud failure for unknown tokens (fail-fast over 2.4M plays), Lahman reconciliation (100% on PA/H/HR/BB/SO), Savant schema-error naming, transactions dedupe; SQLite WITHOUT ROWID keys | ✅ |
| "data-quality auditing at volume" | same; the reconciliation Report is a committed fixture | 🔶 (pitcher outs/R attribution partial — 65–85% on R, +1.6% on outs) |

## Explicit unmatched gaps (filed for the record)

1. **Bayesian projected family** (Orioles DU): the interface is ready; a model lands as v1.1 (DR-3.19's
   "would change our mind").
2. **R notebook consuming sabr-rs artifacts** (DR-3.9): export exists; the documented snippet + runner is
   unscheduled work.
3. **Agentic workflows** (Mets Applied AI): out of scope by DR-3.1's constraint set (no network model calls; the
   CPU path is local).
4. **Biomechanics modelling** (Blue Jays): descriptive panels only in v1 (DR-3.33); a swing-model is v1.1.
5. **Amateur-draft analytics** (Rays, lane #3): the draft feed is verified live (Rules draft ENDPOINT via
   Stats API `/api/v1/draft/{year}`) but no FARM/draft pane yet (T18 pending).

## Verification trail

- Every ✅ item names its record (01-grill.md DR-3.*) and the executed ticket (04-tickets.md).
- Gate proofs run: `cargo test --workspace --release` (383 passed), `backtest` exit 0 under 60 s, X11 smoke line.
