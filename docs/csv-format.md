# CSV format

Used by **Import** (`POST /api/csv/preview`, `POST /api/csv/commit`) and produced
by **Export** (`GET /api/csv/export`).

## Rules

- UTF-8 text, RFC 4180 quoting. A quoted field may contain commas and newlines.
- First row must be a header row and **must contain `name`**.
- Column order does not matter; unknown columns are ignored.
- Maximum size **1 MiB** and **2,000 data rows**.
- Blank lines are skipped silently. A row with an empty `name` is reported as an
  error, never dropped silently.

## Columns

| Column | Required | Notes |
| --- | --- | --- |
| `name` | yes | ≤ 200 chars |
| `description` | no | ≤ 2000 chars |
| `invite_url` | no | `http`/`https` only. Discord invites are normalised to `https://discord.gg/<code>` and lowercased for de-duplication |
| `canonical_url` | no | `http`/`https` only |
| `tags` | no | separated by `;` or `\|`, max 12, lowercased, ≤ 40 chars each |
| `language` | no | free text, ≤ 60 chars |
| `channel_name` | no | only if you actually verified it |
| `user_notes` | no | ≤ 5000 chars |
| `promotion_policy` | no | `allowed`, `prohibited`, `unknown` (default `unknown`) |
| `policy_evidence` | no | quote the rule you read, ≤ 2000 chars |
| `review_status` | no | `not_reviewed`, `suitable`, `unsuitable`, `needs_manual_check`, `contacted` |
| `member_count` | no | informational only; never used as a quality signal |

## Example

```csv
name,description,invite_url,tags,language,promotion_policy,policy_evidence,review_status
Students Hub,A place for students and side hustles,https://discord.gg/stud1,school;students,English,allowed,Promo allowed in #promos,suitable
Dev Lounge,General programming chat,https://discord.gg/dev1,programming,,unknown,,
```

## What the preview reports

Every row gets one of three statuses:

| Status | Meaning |
| --- | --- |
| `new` | Will be created |
| `merge` | Invite already saved; provenance is attached to the existing record instead of overwriting it |
| `invalid` | Will not be saved, with the reasons listed |

Counting rules: `total_rows = new_rows + merge_rows + invalid_rows`.

## Imported records are labelled

Rows saved by import are stored with `primary_source = "CSV import"` and a
provenance row of the same name. They are never presented as live discovery
results, and the UI shows the source on every record.

## Formula-like text

A cell such as `=cmd|'/C calc'!A0` is stored as ordinary text. Nothing in this
application evaluates spreadsheet content. On **export** any value beginning with
`=`, `+`, `-`, `@`, a tab or a carriage return is prefixed with an apostrophe so
spreadsheet software keeps it as text.
