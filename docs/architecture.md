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
| HTTP client | `reqwest` (rustls) | GitHub source only, plus Google Drive/OpenID when sync is enabled |

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
  google/
    mod.rs           Google state + status exposed to the API
    oauth.rs         Authorization Code + PKCE, state, token exchange/refresh
    tokens.rs        token persistence (DPAPI on Windows, 0600 file elsewhere)
    drive.rs         appDataFolder metadata/download/upload with version+ETag
  sync/
    mod.rs           module wiring
    document.rs      versioned cloud document, validation, serialisation
    merge.rs         three-way field merge + conflict records + resolutions
    store.rs         SQLite <-> document mapping, conflicts, snapshots
    engine.rs        single-flight cycle, migration, retry, restore
  routes/
    mod.rs           router assembly
    health.rs        GET /api/health
    dashboard.rs     GET /api/dashboard
    communities.rs   CRUD, review, list/filter/sort/pagination
    discovery.rs     sources, runs, search
    csv.rs           preview, commit, export
    messages.rs      draft generation
    auth.rs          Google sign-in start/callback/status/logout
    sync.rs          sync status/run/conflicts/migrate/export/snapshots
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

sync_state                     -- singleton row (id = 1)
  workspace_id, device_id      -- random ids, no personal data
  base_revision, base_etag     -- last confirmed cloud version
  base_document                -- last synced document (merge base)
  last_synced_at, last_error
  dirty INTEGER                -- counter: any local write increments it
  status TEXT                  -- disconnected|synced|pending|syncing|offline|
                               -- conflict|auth_expired|failed|migration_required

sync_conflicts
  id TEXT PK                   -- deterministic: "<community_id>::<field>"
  community_id, community_name, field
  base_value, local_value, remote_value
  local_record, remote_record  -- full JSON for the UI and resolution
  created_at
```

Write triggers on `communities`, `community_sources`, `settings` and
`discovery_runs` increment `sync_state.dirty`, so every write path schedules a
sync without editing each handler. A sync clears `dirty` only if the counter is
unchanged, so a write that lands mid-cycle is not lost. There is no separate
tombstone table: deletions propagate because the base document still lists the
record while the local snapshot does not.

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

## Local-first sync (optional)

SQLite is the operational store; the UI always reads and writes locally first.
When Google sync is configured and signed in, the app keeps one JSON document,
`scout-data.json`, in the user's Drive `appDataFolder`.

- The document has `schema_version`, `workspace_id`, `generation`, `updated_at`,
  `updated_by`, communities (with sources), non-secret settings and the latest
  50 discovery runs. Tokens, HTML, caches and scores derived from other fields
  are excluded; scores are recomputed locally after a merge.
- The cycle is single-flight. Save immediately, debounce (~2 s), pull at startup
  and every 2 minutes while signed in, and offer a manual "Sync Now". Upload uses
  the Drive `version`/`ETag` with `If-Match`; a `412` re-downloads and re-merges
  with bounded retries.
- Merge is three-way: last synced base, local document, remote document.

| Situation | Result |
| --- | --- |
| Different fields changed | merged automatically |
| Same field, same value | accepted |
| Same field, different values | conflict row; both values preserved |
| One side changed, other equals base | changed value kept |
| Local deleted, remote modified | deletion conflict, record preserved |
| Remote deleted, local modified | deletion conflict, record preserved |
| Both deleted | deletion applied |

- A conflict blocks upload until resolved, so "Synced" is never shown early.
  Resolutions are applied to the merged result (not the base), so choosing
  "keep local" actually uploads the local value.
- First run with records on one or both sides sets `migration_required`. The UI
  offers merge, use-cloud or upload-local; the two lossy choices require explicit
  confirmation and a snapshot first. A non-empty cloud document is never
  replaced automatically, and an empty local database never silently overwrites
  a non-empty cloud one.
- Integrity: schema validation before applying, malformed cloud data copied to
  `data/recovery/` and never overwritten, bounded snapshots (five) before risky
  changes, JSON export, and restore with confirmation. Restore only replaces the
  ids present in the snapshot.
- Google is contacted only when `GOOGLE_CLIENT_ID` is set and the user signs in.
  Without it the sync panel says sync is off and all features remain local.

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
- Google OAuth uses Authorization Code + PKCE (S256), a single-use 10-minute
  state, and only the `drive.appdata` scope. The client secret stays on the
  server. Tokens never appear in URLs, logs, errors or API responses; the
  callback redirects to `/` with no token.
- Tokens are stored with Windows DPAPI (`data/google-tokens.bin`) or a `0600`
  file elsewhere. Logout revokes the refresh token and deletes the local
  credential while keeping local and cloud data.
- Cloud documents are validated (schema version, duplicate ids, size cap)
  before being applied; invalid data is preserved, not overwritten.

## Operations

- `PORT`, `DATABASE_URL`, `RUST_LOG`, `SCOUT_CACHE_TTL_SECS`, `SCOUT_DATA_DIR`,
  `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` and `SCOUT_PUBLIC_URL` are read
  from the environment with safe defaults. Sync is disabled unless
  `GOOGLE_CLIENT_ID` is set; `SCOUT_PUBLIC_URL` defaults to
  `http://127.0.0.1:<PORT>`.
- SQLite uses WAL, `foreign_keys=ON`, `busy_timeout=5000`.
- Health endpoint reports database reachability, not just process liveness.
- Tokens, snapshots and recovery copies live under `SCOUT_DATA_DIR` (default
  `data/`), which must be writable and is never served statically.

## Deployment

The app is a single binary plus a SQLite file. A `Dockerfile` builds it with the
GNU toolchain for the host architecture (amd64 locally, arm64 on the Oracle
Ampere VM). CI builds and runs that image on a native ARM64 runner and verifies
that a record survives a container restart.

The packaged deployment (`deploy/`) runs two containers on a private Docker
network:

- `caddy` publishes 80/443, terminates TLS with automatic certificates, applies
  HSTS and other headers, and requires **Basic Auth** (bcrypt hash) for every
  request. The admin API is disabled.
- `app` binds `0.0.0.0:8080` only inside that network. It has no published port,
  so the API cannot be reached without passing through Caddy's auth.

SQLite lives in the named volume `discord-community-scout_app_data` mounted at
`/app/data`; Caddy certificate state lives in `caddy_data`/`caddy_config`.
`scripts/backup-db.sh` uses SQLite's backup API and verifies each copy, and
`scripts/restore-db.sh` swaps in a verified backup with rollback on failure.

Hosting is **Oracle Cloud Always Free only** (one Ampere A1 VM inside the 4
OCPU/24 GB allowance, no load balancer, no managed database). The account, VM
and DNS record are not created yet; see `docs/deployment.md` for the procedure,
cost safeguards and the outstanding live checks.

Google Drive sync is optional and orthogonal to hosting. It stores data only;
it does not serve the app. A phone must reach a running backend (LAN, Tailscale
free plan, a temporary tunnel, or this VM) before it can use Scout.
