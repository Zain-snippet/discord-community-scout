# Discord Community Scout

A research assistant for public Discord communities. It collects publicly listed
communities, scores how relevant they are to a topic, records your review
decisions, and drafts an outreach message that **you** review and post yourself.

It is not a bot. There is no code path that joins a server, sends a DM, posts a
message, or automates a Discord account.

## Status

| Area | State |
| --- | --- |
| Source-policy audit | done — see [`docs/data-sources.md`](docs/data-sources.md) |
| Server foundation, health endpoint, CI | done |
| Community records, dashboard, search/filter/sort/pagination | done |
| CSV preview/commit/export and manual entry | done |
| Relevance scoring and review workflow | done — explainable factors, prohibited = unsuitable (no override) |
| Discovery connector | done — only the permitted CC-BY-4.0 source enabled; others rejected/unavailable |
| Message drafts | done — deterministic templates, no AI/key, all required disclosures + forbidden-claim guard |
| UI polish/accessibility | done — responsive SPA, Lighthouse 100/100/100 (accessibility/best practices/SEO) on overview |
| Deployment | **not deployed** — readiness notes in [`docs/deployment.md`](docs/deployment.md) |

See [`ROADMAP.md`](ROADMAP.md) for the milestone table.

## Requirements

- Rust **1.82+** (developed on 1.100).
- On Windows, the MSVC or GNU toolchain both work. With `*-windows-gnu` you need
  MinGW-w64 (`gcc`) on `PATH`; with `*-windows-msvc` you need the Visual C++
  build tools.
- No database server. SQLite is embedded.

## Run locally

```bash
cp .env.example .env      # optional; defaults work without it
cargo run
```

Open <http://127.0.0.1:8080>.

The SQLite file is created automatically at `data/scout.db` (ignored by Git).
Migrations are applied at boot.

## Features

- **Discovery**: searches the audited, permitted public source (Awesome Discord Communities, CC-BY-4.0, attributed). Results are scored against the query, deduplicated by invite code, and stored with provenance and run history. Unavailable/rejected sources are listed honestly in-app.
- **Communities**: create/edit/review/delete records; search, filter (status/source/tag/outreach-only), sort and paginate. Promotion policy is `allowed` / `prohibited` / `unknown` (unknown is never treated as permission); prohibited communities are flagged unsuitable with no override.
- **Scoring**: explainable 0–100 breakdown (topic, audience, description, policy evidence, and a policy cap factor) shown in the UI.
- **CSV**: preview with per-row status/errors, commit valid rows (dedup/merge provenance), and export all saved communities.
- **Drafts**: deterministic, copy-only message drafts (concise/long, tones) that always disclose remote commission-based work, the **$12 initial deposit**, and that it is **recoverable after your first payment, subject to the company's terms**, and invite questions. A runtime guard refuses drafts containing unsupported claims. Nothing is posted automatically.
- **Fallback**: when discovery is unavailable, CSV import and manual entry remain fully usable.

### Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `PORT` | `8080` | HTTP port |
| `DATABASE_URL` | `sqlite://data/scout.db?mode=rwc` | Must be a `sqlite:` URL |
| `RUST_LOG` | `info` | Log filter |
| `SCOUT_CACHE_TTL_SECS` | `3600` | Discovery response cache lifetime |

Invalid configuration is reported at startup with a clear message rather than a
panic.

## Develop

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks plus a health-endpoint smoke test
(`.github/workflows/ci.yml`).

## API

Base path: `/api`. Errors are `{"error":{"code","message"}}`.

Key endpoints (see [`docs/api.md`](docs/api.md) for full request/response shapes): `GET /api/health`, `GET /api/dashboard`, `GET /api/meta`, CRUD `/api/communities` + `/api/communities/:id/review`, `POST /api/csv/preview|commit`, `GET /api/csv/export`, `GET /api/sources`, `GET /api/discovery/runs`, `POST /api/discovery/search`, `POST /api/message/draft`. A `robots.txt` disallows `/api/`.

## Data sources

Discovery is limited to public sources whose terms permit automated access. The
audit of Top.gg, DISBOARD, Discord.me, DiscordServers and Discord's own discovery
is in [`docs/data-sources.md`](docs/data-sources.md); all were rejected. The one
accepted source is the CC-BY-4.0
[Awesome Discord Communities](https://github.com/mhxion/awesome-discord-communities)
list, read through the GitHub REST API with attribution.

Because that list is programming-focused, **CSV import and manual entry are the
primary way to work with other audiences.**

## What this app will not do

- Automate a Discord account, self-bot, DM, join, or post.
- Scrape Discord or any directory that blocks automated access.
- Invent channel names, rules, member counts, or invite links.
- Claim the opportunity is verified, salaried, or risk-free.

## Deployment

**Not deployed.** Build/run locally or via the included `Dockerfile`; see [`docs/deployment.md`](docs/deployment.md) for readiness notes, health checks, and the (unverified) caveats around free-tier hosting and SQLite persistence.

## Licence

MIT. The bundled source list is CC-BY-4.0 and is attributed in the UI.
