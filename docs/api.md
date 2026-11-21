# API

Base path: `/api`. All responses are JSON.

Errors use a single envelope and never include stack traces or database detail:

```json
{ "error": { "code": "bad_request", "message": "Name is required." } }
```

| `code` | HTTP status |
| --- | --- |
| `bad_request` | 400 |
| `not_found` | 404 |
| `conflict` | 409 |
| `upstream_error` | 502 |
| `database_error` | 500 |
| `internal_error` | 500 |

---

## GET /api/health

Liveness **and** database reachability.

```json
{ "status": "ok", "version": "0.1.0", "database": "ok" }
```

Returns `503` with `"status":"degraded"` if the database cannot be queried.

---

## GET /api/dashboard

Counters computed with SQL from `communities`. Never hard-coded.

```json
{
  "total_communities": 12,
  "awaiting_review": 7,
  "reviewed": 5,
  "suitable": 3,
  "unsuitable": 1,
  "contacted": 1,
  "needs_manual_check": 0,
  "promotion_prohibited": 2,
  "promotion_allowed": 4,
  "policy_unknown": 6,
  "last_activity": { "kind": "Community updated", "detail": "Students Hub", "at": "2026-11-16T13:20:00Z" }
}
```

`last_activity` is `null` when nothing has happened yet.

---

## GET /api/meta

Review vocabulary plus the sources and tags that currently exist, so the UI
never hard-codes filter options.

```json
{
  "review_statuses": ["not_reviewed", "suitable", "unsuitable", "needs_manual_check", "contacted"],
  "promotion_policies": ["allowed", "prohibited", "unknown"],
  "sources": [{ "name": "Manual entry", "count": 3 }],
  "tags": [{ "name": "students", "count": 2 }]
}
```

---

## GET /api/communities

Query parameters (all optional):

| Parameter | Values | Notes |
| --- | --- | --- |
| `q` | text | Matches name, description or tags. Wildcards are escaped |
| `status` | `not_reviewed`, `suitable`, `unsuitable`, `needs_manual_check`, `contacted` | Invalid values are a 400 |
| `source` | exact source name | |
| `tag` | exact tag | |
| `sort` | `relevance` (default), `name`, `newest`, `oldest` | Unknown values are a 400 |
| `page` | integer ≥ 1 | Default 1 |
| `per_page` | 1–100 | Default 25 |
| `outreach_only` | `true`/`false` | Excludes communities whose rules prohibit promotion |

```json
{ "items": [ /* Community */ ], "page": 1, "per_page": 25, "total": 1, "total_pages": 1 }
```

Each community carries its provenance:

```json
{
  "id": "2f0c…",
  "name": "Students Making Money",
  "description": "…",
  "invite_url": "https://discord.gg/testcode1",
  "invite_code": "testcode1",
  "canonical_url": null,
  "primary_source": "Manual entry",
  "tags": ["students", "side hustle"],
  "member_count": null,
  "language": null,
  "relevance_score": 20,
  "score_breakdown": [
    { "factor": "topic", "points": 39, "max": 50, "reason": "Matched search terms: students." },
    { "factor": "audience", "points": 20, "max": 20, "reason": "Audience keywords: students, remote." },
    { "factor": "description", "points": 15, "max": 15, "reason": "Description mentions: work, remote." },
    { "factor": "policy_evidence", "points": 0, "max": 15, "reason": "The community's rules prohibit promotion, so no points are awarded." },
    { "factor": "policy_cap", "points": -54, "max": 0, "reason": "The community prohibits promotion, so the score is capped at 20 (subtotal 74)." }
  ],
  "review_status": "not_reviewed",
  "promotion_policy": "prohibited",
  "policy_evidence": "No advertising or referral links",
  "policy_checked_at": "2026-11-16T13:19:58Z",
  "channel_name": null,
  "user_notes": "rules say no promo",
  "contacted_at": null,
  "last_checked_at": null,
  "created_at": "2026-11-16T13:19:58Z",
  "updated_at": "2026-11-16T13:19:58Z",
  "sources": [
    { "source_name": "Manual entry", "source_external_id": null, "listing_url": null,
      "first_seen_at": "…", "last_seen_at": "…" }
  ],
  "unsuitable_for_outreach": true,
  "outreach_note": "This community's rules prohibit promotion or recruitment. Do not post here."
}
```

The factor list always sums to `relevance_score`, including the negative
`policy_cap` entry when a prohibition caps the score.

---

## POST /api/communities

Creates a record entered by hand. Returns `201` with the created community.

```json
{
  "name": "Students Making Money",
  "description": "optional, ≤ 2000 chars",
  "invite_url": "https://discord.gg/abc123",
  "canonical_url": "https://example.com/listing",
  "tags": ["students", "side hustle"],
  "language": "English",
  "channel_name": "#promotions",
  "user_notes": "checked the rules on 16 Nov",
  "promotion_policy": "allowed",
  "policy_evidence": "Quoted rule text",
  "review_status": "not_reviewed"
}
```

Validation:

- `name` is required, ≤ 200 characters, whitespace collapsed.
- URLs must be `http`/`https`; `javascript:`, `data:`, `file:` are rejected.
- Tags are lowercased, deduplicated, ≤ 40 chars each, ≤ 12 tags.
- A Discord invite is normalised to `https://discord.gg/<code>` and the code is
  stored lowercase, so the same server cannot be added twice (`409`).
- `promotion_policy` must be `allowed`, `prohibited` or `unknown`.

---

## GET /api/communities/{id}

Returns one community or `404`.

## PATCH /api/communities/{id}

Same body as create. Fields not supplied keep their current value, except the
plain text fields which are replaced. Recomputes the score.

## DELETE /api/communities/{id}?confirm=true

`confirm=true` is mandatory; without it the request is a `400` rather than a
silent deletion. Returns `204`. Provenance rows are removed by cascade.

## PATCH /api/communities/{id}/review

Records the review decision and recomputes the score.

```json
{
  "review_status": "suitable",
  "promotion_policy": "allowed",
  "policy_evidence": "Promotion allowed in #promotions",
  "channel_name": "#promotions",
  "user_notes": "posted 16 Nov",
  "contacted": true
}
```

- `review_status` is required and must be one of the five values.
- `policy_checked_at` is set to now unless the policy is `unknown`, in which case
  it is cleared.
- `contacted: true` (or status `contacted`) stamps `contacted_at` once and keeps
  the original timestamp on later edits.

---

## POST /api/csv/preview

Validates a CSV document and reports what committing it would do. Format is
documented in [`csv-format.md`](csv-format.md).

```json
{ "csv": "name,description,...\nStudents Hub,..." }
```

```json
{
  "total_rows": 4, "new_rows": 2, "merge_rows": 0, "invalid_rows": 2,
  "rows": [
    { "line": 2, "name": "Students Hub", "status": "new", "errors": [] },
    { "line": 5, "name": "Duplicate", "status": "invalid",
      "errors": ["Duplicate invite link within this file."] }
  ]
}
```

Header problems are a `400` with a message. Row problems are reported per row and
never dropped silently.

## POST /api/csv/commit

Same body as preview. Saves every valid row.

```json
{ "imported": 2, "merged": 0, "skipped_invalid": 2,
  "problems": ["Line 4: not saved — invite_url/canonical_url: Only http and https URLs are allowed."] }
```

`merge` rows attach a `CSV import` provenance row to the existing record; they do
not overwrite it. Imported records carry `primary_source = "CSV import"`.

## GET /api/csv/export

Returns `text/csv` as an attachment with every saved community and its review
fields. Values starting with `=`, `+`, `-`, `@`, tab or carriage return are
prefixed with an apostrophe so spreadsheets treat them as text.

---

## GET /api/sources

Every source considered in the audit, with an honest availability flag.

```json
{
  "discovery_available": true,
  "sources": [
    { "id": "awesome-discord-communities", "name": "Awesome Discord Communities",
      "homepage": "https://github.com/mhxion/awesome-discord-communities",
      "license": "CC-BY-4.0", "enabled": true,
      "notes": "Curated public list read through the GitHub REST API…" },
    { "id": "disboard", "name": "DISBOARD", "enabled": false,
      "notes": "No public API; automated requests are blocked (HTTP 403). Not scraped." }
  ]
}
```

Disabled sources are listed so the UI states plainly that they are not
integrated, instead of hiding them or pretending search covers them.

## GET /api/discovery/runs?limit=20

Most recent runs, newest first. `limit` is clamped to 1–50.

```json
[{ "id": "…", "query": "rust", "requests_limit": 5, "status": "succeeded",
   "result_count": 66, "saved_count": 5, "error": null,
   "started_at": "…", "finished_at": "…" }]
```

## POST /api/discovery/search

Runs one search against the permitted source, scores and saves the results.

```json
{ "query": "rust programming", "limit": 5 }
```

`limit` is clamped to 1–50 (default 10). A blank or over-200-character query is a
`400`.

```json
{
  "run_id": "…", "query": "rust programming", "requested_limit": 5,
  "status": "succeeded", "total_matched": 66, "returned": 5,
  "saved_new": 5, "merged_existing": 0, "partial": false, "duration_ms": 748,
  "attribution": "Source: Awesome Discord Communities (…), licensed CC-BY-4.0.",
  "results": [ { "community": { /* Community */ }, "status": "new" } ]
}
```

- Results are deduplicated by invite code. A community already saved is returned
  with `status: "existing"` and gains a provenance row; its data is not
  overwritten.
- If the upstream source is unreachable the response is `502` with
  `{"error":{"code":"upstream_error"}}` and a message pointing to CSV import and
  manual entry, which never depend on a third party.

## POST /api/message/draft

Builds deterministic drafts — no AI service, no API key.

```json
{ "community_id": "…", "tone": "conversational", "variant": "concise",
  "notes": "optional extra context you want appended" }
```

`tone`: `conversational` (default) or `direct`. `variant`: `concise` (default) or
`long`. The other length is always returned as `alternative`.

```json
{
  "community_id": "…", "community_name": "Students Hub",
  "draft": {
    "variant": "concise",
    "text": "Hey everyone! … remote, commission-based … $12 initial deposit … recoverable after your first payment, subject to the company's terms … happy to share the details …",
    "can_post": true,
    "warnings": [
      "This community's rules are not verified. Read its posted rules before posting.",
      "This is a draft. Review it yourself, confirm it matches the community's rules, and post it manually if appropriate."
    ],
    "disclosures": ["Remote", "Commission-based, not salaried", "$12 initial deposit disclosed",
      "Deposit recoverable after the first payment, subject to the company's terms",
      "Invitation to ask for details"],
    "posting_target": null
  },
  "alternative": { /* the other variant */ }
}
```

- `can_post` is `false` when the community's rules prohibit promotion, and
  `posting_target` is then always `null`. There is no override.
- `posting_target` is only set from a channel name the user recorded themselves.
- A runtime guard re-checks every draft against a forbidden-claims list and refuses
  to return one that contains an unsupported claim.

---

## Google auth and sync

These endpoints are always present. When `GOOGLE_CLIENT_ID` is unset, sync is
`disconnected`, `GET /api/auth/google/start` returns `400`, and no Google request
is made. See [`google-drive-setup.md`](google-drive-setup.md) for setup.

### GET /api/auth/status and GET /api/sync/status

Both return the same shape:

```json
{
  "google": {
    "configured": true,
    "signed_in": true,
    "token_storage": "Windows DPAPI"
  },
  "sync": {
    "status": "synced",
    "raw_status": "synced",
    "last_synced_at": "2026-11-21T16:00:00Z",
    "last_error": null,
    "dirty": false,
    "conflicts": 0,
    "records": 12,
    "workspace_id": "uuid",
    "has_base": true,
    "generation": 4
  }
}
```

`status` is one of `disconnected`, `synced`, `pending`, `syncing`, `offline`,
`conflict`, `auth_expired`, `failed`, `migration_required`. Never render
"Synced" from `dirty: false` alone; use `status`.

### GET /api/auth/google/start

Redirects to Google with PKCE, a single-use state (10 minutes) and the
`drive.appdata` scope. `400` when sync is not configured.

### GET /api/auth/google/callback

Google's redirect target. Always redirects to `/?google=connected`,
`/?google=denied` or `/?google=error`; tokens never appear in the URL or body.

### POST /api/auth/logout

Revokes the refresh token at Google, deletes the local credential and returns
the status shape above. Local records and the Drive document are untouched.

### POST /api/sync/run

Runs one single-flight cycle and returns a report. A cycle already in progress
returns `status: "syncing"` rather than queueing.

```json
{ "status": "synced", "message": "Changes uploaded.", "conflicts": 0,
  "records": 12, "uploaded": true }
```

### GET /api/sync/conflicts

One row per unresolved field conflict.

```json
[{
  "id": "uuid::user_notes", "community_id": "uuid", "community_name": "Students Hub",
  "field": "user_notes", "base_value": "", "local_value": "local note",
  "remote_value": "other note", "created_at": "…"
}]
```

Deletion conflicts use `field: "__deleted__"` with values `deleted`/`present`.
No upload happens while a conflict is open.

### POST /api/sync/conflicts/{id}/resolve

```json
{ "choice": "local" }
```

`choice` is `local` or `remote`. Only the named field is resolved; the rest of
the merge stays. Returns a sync report.

### POST /api/sync/migrate

Used when `status` is `migration_required`.

```json
{ "choice": "merge", "confirm": false }
```

`choice` is `merge`, `use_cloud` or `upload_local`. `merge` preserves both
sides. The other two replace one side and return `400` unless `confirm` is
`true`; the UI shows a snapshot notice first. `upload_local` is refused when a
cloud document already exists.

### GET /api/sync/export

Downloads the local copy of the cloud document as
`application/json` with `Content-Disposition: attachment; filename="scout-data.json"`.
Contains no tokens or secrets.

### GET /api/sync/snapshots

Lists up to five snapshots, newest first.

```json
[{ "name": "20261121T150000Z-before-first-merge.json", "bytes": 8123 }]
```

### POST /api/sync/snapshots/{name}/restore

```json
{ "confirm": true }
```

Replaces the records present in the snapshot; `confirm` is mandatory. A snapshot
of the current state is taken first where applicable.

