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

## Planned endpoints

Implemented in later milestones and documented here when they land:

- `GET /api/sources`, `GET /api/discovery/runs`, `POST /api/discovery/search`
- `POST /api/csv/preview`, `POST /api/csv/commit`, `GET /api/csv/export`
- `POST /api/message/draft`
