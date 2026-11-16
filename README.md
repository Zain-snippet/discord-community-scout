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
| Server foundation, health endpoint, UI shell | working |
| Community records, dashboard, search | in progress |
| CSV import/export | in progress |
| Relevance scoring and review workflow | in progress |
| Discovery connector | in progress |
| Message drafts | in progress |
| Deployment | not deployed |

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

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/health` | Liveness plus database reachability |

More endpoints are documented in [`docs/api.md`](docs/api.md) as they land.

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

## Licence

MIT. The bundled source list is CC-BY-4.0 and is attributed in the UI.
