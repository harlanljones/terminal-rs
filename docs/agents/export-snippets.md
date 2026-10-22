# Export snippets: reading sabr-rs artifacts from R and Python

Exports live under the artifacts dir (default `$XDG_DATA_HOME/sabr-rs/artifacts` — set
`SABR_DATA_DIR` to relocate; `sabr fit` prints the written hash and path). One JSON artifact per
fit; `docs/plan/01-grill.md` DR-3.22 defines the shape (model/as_of/scores/forecasts).
`sabr export` (planned Wave 9) will add CSV dumps; until then consume the JSON directly.

Schema (as of T15/T18):

- `model` — `"marcel.batting/1.0.0"` (name/version split on `/`)
- `as_of`, `created` — labels (informational; `created` is excluded from the content hash)
- `corpus_rev` — 3 input SHA-256s (Lahman inputs, fixed order)
- `protocol_hash`, `constants_hash` — provenance pins
- `scores` — per-season `mae` (PA-weighted, re-centred), `pa_mae`, coverage cells, gate cells
- `forecasts[]` — `player_id` (Lahman id, joinable to lahaman `People.csv` → `retroID` → the
  bundled catalog's `key_mlbam`), `season`, `woba` (batting) / `era` (pitching), `pa`/`ip`,
  `reliability`, `bucket` ∈ High/Med/Low/None, and de-standardised error quantiles `q05..q95`
  (add to the projection for bands).

## R (jsonlite + data.table)

```r
# dalahman: read one sabr-rs artifact and join to Lahman for names/roles.
library(jsonlite); library(data.table)

artifacts_dir <- file.path(Sys.getenv("XDG_DATA_HOME", "~/.local/share"), "sabr-rs", "artifacts")
latest <- function(model_prefix = "marcel.batting") {
  files <- list.files(artifacts_dir, paste0("^", model_prefix, ".*\\.json$"), full.names = TRUE)
  stopifnot(length(files) > 0)
  # one file per (as-of, inputs); the content hash names it, so "latest" = newest mtime.
  files[order(file.mtime(files))][length(files)]
}

art <- fromJSON(latest())
rows <- as.data.table(art$forecasts)
rows$woba_lo80 <- rows$woba + rows$`q10`   # the 80% band at 10/90
rows$woba_hi80 <- rows$woba + rows$`q90`

# join names: People.csv ships inside the repo (sabr-data/sample/lahman)
# people <- fread("terminal-rs/sabr-data/sample/lahman/People.csv")
# rows[people, on = .(player_id = playerID)] |> head()

# the honest number a reviewer asks for first:
art$scores$gate$pass
```

Join key through Lahman: `player_id` in the artifact is the Lahman `playerID`
(`ohtansh01`); the bundled catalog (`sabr-data/sample/catalog.csv`) maps via
`People.csv$bbrefID`/`retroID` to MLBAM ids — the same chain the engine itself uses
(`TerminalApp`/CLI resolve `OHTANI` → id → backtest artifact). Deterministic by construction:
re-running `sabr fit` with identical inputs yields the same content hash.

## Python (stdlib)

```python
import json, pathlib
# `sabr fit` prints the exact JSON path and content hash; read it back the same way the CLI does.
a = json.loads(pathlib.Path("/path/to/artifact.json").read_text())
rows = a["forecasts"]                          # one dict per player
ohtani = next(r for r in rows if r["player_id"] == "ohtansh01")
band = [ohtani["woba"] + ohtani[k] for k in ("q10", "q90")]  # the 80% band
assert a["scores"]["gate"]["pass"], "gate: see docs/plan/03-spec.md DR-3.12a"
```

Join key through Lahman: the artifact's `player_id` is the Lahman `playerID`
(`ohtansh01`); `People.csv` (bundled) maps `bbrefID`/`retroID` → back to the engine's own
resolving chain. Deterministic by construction: identical inputs → identical content hash.
