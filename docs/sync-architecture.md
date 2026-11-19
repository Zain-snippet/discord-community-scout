# Local-first sync architecture (Google Drive)

Status: **design + implementation in progress.** Google OAuth has not been run
against a real Google account from this environment. Nothing in this document
claims that a live Drive sync has been verified.

## 1. Audit of the existing application

Discord Community Scout is a single Rust binary (Axum) that serves a static
single-page frontend and a JSON API from the same origin, backed by SQLite.

| Area | Current implementation | Needs the Rust backend? |
| --- | --- | --- |
| UI | `static/index.html`, `app.js`, `styles.css` — no build step, `textContent` only | No |
| Persistence | SQLite via `sqlx`, migrations in `migrations/` | Yes |
| Community CRUD / review | `src/repo.rs`, `src/routes/communities.rs` | Yes |
| Scoring | `src/scoring.rs` (deterministic, explainable factors) | Yes |
| Message drafts | `src/templates.rs` (deterministic, no AI) | Yes |
| CSV preview/commit/export | `src/csv_io.rs`, `src/routes/csv.rs` | Yes |
| Discovery | `src/discovery.rs`, `src/sources/awesome.rs` (server-side fetch, bounded timeouts, in-memory cache) | Yes |
| Dashboard / meta | SQL aggregates in `src/repo.rs` | Yes |
| Health | `src/routes/health.rs` | Yes |

Everything except static-file delivery is implemented in Rust. Porting scoring,
templates, CSV and discovery into the browser would be a rewrite of the parts
that define the product, so this work keeps the Axum backend and adds cloud
synchronization around it.

Existing data model: `communities` (stable UUID primary key), `community_sources`
(append-only provenance), `discovery_runs`, `settings`. Migrations are forward
only and applied at boot.

The local database in this repository is currently **empty** (0 communities,
0 discovery runs), verified through the running API on 2026-11-19. The migration
path is still implemented and tested with synthetic data.

## 2. What Google Drive can and cannot do

Google Drive is **data storage**, not application hosting. It cannot serve the
Axum server, the JSON API, discovery fetches or CSV parsing to a browser on
another device. This distinction drives the deployment options below.

| Concern | Where it lives |
| --- | --- |
| Application hosting | The machine running the Rust binary (this laptop, another PC, or a VM) |
| Local backend execution | That same process, listening on `0.0.0.0:PORT` |
| Cloud data storage | One JSON document in the user's Drive `appDataFolder` |
| Cross-device access | A browser reaching a backend over LAN, VPN/mesh, a tunnel, or a host |

Consequences, stated plainly:

- A phone can use Scout only while it can reach a running backend.
- Two backends (for example laptop and desktop) each keep a local SQLite cache
  and reconcile through the Drive document.
- Google Drive does not make the backend reachable. If the laptop is asleep and
  nothing else forwards the port, the phone has no application to talk to.

### Free options for reaching the backend (no card)

1. **LAN (default, zero setup).** Both devices on the same Wi-Fi:
   `http://<laptop-lan-ip>:8080`. The laptop must allow inbound 8080 in the
   Windows firewall (private network only).
2. **Tailscale free plan.** Up to 3 users / 100 devices, no payment method.
   Laptop and phone join the same tailnet and use `http://<machine>:8080` from
   anywhere. This is a private network, not public hosting.
3. **Cloudflare quick tunnel** (`cloudflared tunnel --url http://localhost:8080`).
   Free, no account or card, gives a temporary `https://*.trycloudflare.com`
   URL that changes on each run. Suitable for occasional use, not a stable URL.
4. **Oracle Cloud Always Free VM** (already packaged in `deploy/`). Free within
   the Always Free limits, but it is a cloud server, which this brief excludes
   as a requirement. It remains an optional path, not the default.

Authentication is server-side: the backend holds the Google tokens. Sign-in is
performed once on the backend host (loopback redirect), and every browser that
reaches that backend shares the connection. A phone does not need its own
Google login when it talks to the laptop's backend.

## 3. Google OAuth design

- OAuth 2.0 **Authorization Code with PKCE** (S256), server-side.
- Scope: `https://www.googleapis.com/auth/drive.appdata` — the app-specific
  folder only. No profile, Gmail, Photos, contacts, or full-Drive access.
- `access_type=offline` with `prompt=consent` to obtain a refresh token.
- The client secret (if the OAuth client type has one) is read from
  `GOOGLE_CLIENT_SECRET` on the server and is never sent to the browser.
- State + PKCE verifier are random, single-use and expire after 10 minutes.
- Tokens never appear in URLs, logs, errors, or the repository. The callback
  exchanges the code and redirects to `/` with no token in the URL.
- Access tokens are refreshed 60 seconds before expiry. A failed refresh
  (`invalid_grant`) marks the connection as needing re-authorization.
- Logout revokes the refresh token at Google, deletes local tokens, and keeps
  all data (local and cloud) intact.

Token storage:

- **Windows:** DPAPI (`CryptProtectData`) blob in `data/google-tokens.bin`,
  bound to the current Windows user. Verified by a unit test that round-trips a
  sample token.
- **Other platforms:** `data/google-tokens.json` with mode `0600` and a warning
  in the log. On a headless VM this is the honest trade-off; it is not claimed
  to be OS-secure storage.

OAuth client types:

- Local/loopback: **Desktop app** client, redirect
  `http://127.0.0.1:<PORT>/api/auth/google/callback`.
- Hosted HTTPS: **Web application** client with the exact public redirect URI.

Google limitations that must be documented for the user: while the OAuth
consent screen is in **Testing**, refresh tokens expire after 7 days and only
listed test users can sign in. Publishing the app (even unverified, with the
"unverified app" warning) avoids the 7-day expiry but is subject to Google's
user cap and, for sensitive scopes, verification if the warning should be
removed. These are Google-side policies, not app behaviour.

## 4. Cloud document

One JSON file, `scout-data.json`, in `appDataFolder`:

```json
{
  "schema_version": 1,
  "app": "discord-community-scout",
  "workspace_id": "uuid",
  "generation": 12,
  "updated_at": "2026-11-19T10:00:00Z",
  "updated_by": "device-uuid",
  "communities": [
    {
      "id": "uuid",
      "name": "...", "description": "...", "tags": ["..."],
      "review_status": "suitable", "promotion_policy": "allowed",
      "policy_evidence": "...", "user_notes": "...",
      "created_at": "...", "updated_at": "...",
      "deleted": false,
      "sources": [{ "source_name": "...", "listing_url": "...", "first_seen_at": "...", "last_seen_at": "..." }]
    }
  ],
  "settings": { "key": "value" },
  "discovery_runs": [{ "id": "...", "query": "...", "status": "succeeded", "started_at": "..." }]
}
```

Included: community records, stable IDs, tags, review state, promotion policy
and evidence, notes, contact tracking, provenance, non-secret settings, the 50
most recent discovery runs, schema version, workspace ID, generation, device ID.

Excluded: tokens and secrets, HTML, images, search payloads, caches, logs,
duplicate records. Scores are stored but **recomputed locally after a merge**
from the merged fields, so two devices never fight over derived numbers.
Tombstones (`"deleted": true`) are kept for 90 days so deletions propagate and
records cannot be resurrected by a stale device.

## 5. Synchronization model

Local SQLite remains the operational store. New tables:

- `sync_state` — singleton: workspace ID, device ID, Drive file ID, base
  revision/etag, base document JSON, last sync time, dirty flag, status,
  last error.
- `sync_conflicts` — one row per unresolved field conflict.
- `sync_tombstones` — local deletions not yet uploaded.

Triggers mark `sync_state.dirty = 1` on community changes, so every write path
(manual, CSV import, discovery save, review) is covered without instrumenting
each handler.

Sync cycle (single-flight, one at a time):

1. Verify the token; refresh if needed.
2. Metadata request: `fields=id,version,etag`. If the remote version equals the
   base version and nothing is dirty, stop.
3. Download and validate the cloud document (schema, duplicate IDs, size cap).
   Invalid data is never overwritten; a copy is written to `data/recovery/`.
4. Build the local document from SQLite + tombstones.
5. Three-way merge: **base** (last synced document), **local**, **remote**.
6. If conflicts: apply non-conflicting values locally, store conflicts, set
   status `conflict`, and **do not upload**.
7. Otherwise apply the merged document locally, then upload with `If-Match` on
   the last known ETag. On `412` the remote changed underneath us: re-download,
   merge again (bounded retries).
8. On success: base = uploaded document, base revision/ETag updated, dirty
   cleared, tombstones cleared, `last_synced_at` set.

Triggers: startup, local change (2 s debounce), manual "Sync now", tab
returning to the foreground, and a 2-minute metadata check while signed in.
Retries use bounded exponential backoff; 401 triggers one refresh, 429/5xx back
off, network failures set `offline` and keep local changes.

## 6. Conflict resolution

Per community, comparing **base / local / remote** field by field:

| Situation | Result |
| --- | --- |
| Different fields changed | merged automatically |
| Same field, same value | accepted, no conflict |
| Same field, different values | conflict row; local value kept pending, both shown |
| One side changed, other equals base | changed value kept (stale value never wins) |
| Local deleted, remote modified | conflict, record preserved |
| Remote deleted, local modified | conflict, record preserved |
| Both deleted | deletion applied |

Every community has a UUID; names are never used as identity. Conflicts are
resolved in the UI one field at a time ("keep this device" / "keep other
device"). Resolution moves the base value to the remote value for that field so
the next merge uploads the chosen value without re-conflicting.

## 7. Local cache, offline and integrity

- SQLite serves the UI immediately, including offline, and stores pending
  changes. Nothing is discarded on logout, close, or network failure.
- The UI distinguishes "Synced", "Changes pending", "Syncing", "Offline",
  "Conflict", "Authentication expired", "Sync failed".
- Safety rails: validate before applying; never overwrite unparseable cloud
  data; refuse to upload an empty workspace over a non-empty cloud document
  unless the user actually deleted those records; snapshot the previous local
  document and the previous cloud document to `data/snapshots/` (5 kept);
  explicit confirmation for destructive operations.
- Import/export remain available (CSV export, cloud document export), and a
  restore from snapshot is exposed through `scripts/`-style file recovery and
  the API.

## 8. Migration of existing SQLite data

- Cloud missing + local records exist → `migration_required`; the UI asks
  before uploading. No silent upload.
- Cloud exists + local records exist + no base (first sync after upgrading, or
  a second device) → `migration_required`; the UI offers "merge" (three-way
  merge with an empty base, conflicts surfaced) or "use cloud" (local snapshot
  first). The cloud document is never replaced automatically.
- A snapshot of the local database's document is written before the first
  upload and before adopting cloud data.
- Migration is idempotent: it only changes `sync_state`, and re-running after a
  failure re-attempts with the current local data; record counts are compared
  and reported.

## 9. Risks and honest limitations

- **OAuth is unverified here.** No real Google client exists in this
  environment; the flow is implemented and unit-tested at the pure-logic level
  only. It must be tested with a real account before being called working.
- **Drive is not a real-time bus.** Two backends see each other's changes on
  the next poll (2 minutes) or manual sync, not instantly.
- **Drive version history is not an app backup.** The app keeps bounded local
  snapshots; off-device recovery relies on Drive's own file version history.
- **Tokens on non-Windows hosts** are file-protected, not OS-keychain-protected.
- **Phone access needs a reachable backend**; this design does not host one.
- Drive `appDataFolder` is hidden from the user's Drive UI; recovery must go
  through the app or the Drive API.
