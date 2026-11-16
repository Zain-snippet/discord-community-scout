# Architecture

Discord Community Scout is a single-binary Rust web application. It serves a
static frontend and a JSON API from the same Axum server.

## Stack

| Concern | Choice | Reason |
| --- | --- | --- |
| HTTP | `axum` 0.8 | Small, Tokio-native, no macro DSL |
| Runtime | `tokio` | Required by axum |
| Serialization | `serde` / `serde_json` | Standard |
| Database | SQLite via `sqlx` 0.8 (runtime queries) | Single file, no server, no cost. Compile-time macros are **not** used, so `cargo build` never needs a live database |
| Migrations | `sqlx::migrate!` embedded SQL | Versioned, applied at boot |
| Templating | none — deterministic Rust string building | Message drafts must be testable |
| Frontend | plain HTML + CSS + ES modules | No build step, no framework |
| Logging | `tracing` + `tracing-subscriber` | Structured, level via `RUST_LOG` |
| HTTP client | `reqwest` (rustls) | One outbound source only |

## Module map

```
src/
  main.rs            bootstrap: config -> tracing -> db -> router -> serve
  config.rs          env parsing + startup validation
  error.rs           AppError -> RFC7807-style JSON responses
  db.rs              pool, migrations, SQLite pragmas
  models.rs          DTOs shared by routes and repositories
  repo/              SQL access (communities, sources, runs, settings)
  scoring.rs         deterministic 0-100 relevance scoring
  templates.rs       message drafting
  csv_io.rs          import parse/validate, export rendering
  discovery.rs       discovery runs, caching, orchestration
  sources/
    mod.rs           SourceConnector trait + registry
    awesome.rs       GitHub-hosted CC-BY-4.0 list connector
  routes/
    mod.rs           router assembly
    health.rs        GET /api/health
    dashboard.rs     GET /api/dashboard
    communities.rs   CRUD, review, list/filter/sort/pagination
    discovery.rs     sources, runs, search
    csv.rs           preview, commit, export
    messages.rs      draft generation
```

`repo/` exists as a directory only because it holds more than one logical group
(communities, sources, runs, settings); there are no empty layers.

## Data model

```
communities
  id TEXT PK (uuid v4)
  name, description, invite_url, invite_code, canonical_url
  tags (JSON array as TEXT), member_count, language
  relevance_score INTEGER, score_breakdown (JSON as TEXT)
  review_status TEXT            -- not_reviewed|suitable|unsuitable|needs_manual_check|contacted
  promotion_policy TEXT         -- allowed|prohibited|unknown
  policy_evidence, policy_checked_at
  channel_name                  -- only ever verified or user-entered
  user_notes
  contacted_at, last_checked_at
  created_at, updated_at

community_sources          -- one row per provenance reference
  id INTEGER PK, community_id FK -> communities(id) ON DELETE CASCADE
  source_name, source_external_id, listing_url
  first_seen_at, last_seen_at
  UNIQUE(community_id, source_name, source_external_id)

discovery_runs
  id TEXT PK, query, sources (JSON), requested_limit
  status, started_at, finished_at
  result_count, saved_count, error, partial (BOOL)

settings
  key TEXT PK, value TEXT
```

`member_count` is only ever populated from a source that supplies it. The
awesome-list connector supplies none, so it stays `NULL` there.

## Relevance scoring

Implemented in `scoring.rs`, pure function, no I/O, fully unit-tested.

| Factor | Max | Rule |
| --- | --- | --- |
| Topic match | 50 | Weighted token overlap between query terms and name + description + tags |
| Audience match | 20 | Matched audience keywords (student, freelance, entrepreneur, remote, career) |
| Description relevance | 15 | Presence of opportunity/career vocabulary in the description |
| Public evidence of allowed promotion | 15 | **Only** when `promotion_policy = allowed` **and** evidence text exists |

Hard override: `promotion_policy = prohibited` forces the record to be reported
as `unsuitable_for_outreach = true` and caps the score, regardless of topic
match. Unknown policy never earns the last 15 points.

The breakdown is returned as a list of `{factor, points, max, reason}` so the UI
can explain the number instead of just showing it.

## Security posture

- All SQL uses bound parameters; no string interpolation of user data.
- CSV import capped at 1 MiB and 2,000 rows; preview is capped too.
- URL validation rejects any scheme other than `http`/`https`.
- Community notes and names are escaped by the frontend via `textContent`
  (never `innerHTML` for user data).
- The server never fetches user-supplied URLs (no SSRF surface).
- Request body limit set on the router; destructive actions require an explicit
  confirmation flag in the request and a confirm dialog in the UI.
- No secrets in the repo; `.env` is ignored, `.env.example` holds placeholders.

## Operations

- `PORT`, `DATABASE_URL`, `RUST_LOG`, `SCOUT_CACHE_TTL_SECS` are read from the
  environment with safe defaults.
- SQLite uses WAL, `foreign_keys=ON`, `busy_timeout=5000`.
- Health endpoint reports database reachability, not just process liveness.

## Deployment

The app is a single binary plus a SQLite file. A `Dockerfile` builds it with the
GNU toolchain. Durable storage is the constraint: any host that resets the
container filesystem loses the database, so a provider offering a persistent
volume (or a single always-on VM) is required. Free-tier provider terms change
frequently and were **not** verifiable in this environment (no web search
available), so no provider is claimed as verified; see `docs/deployment.md`.
