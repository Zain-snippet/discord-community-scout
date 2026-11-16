# Deployment readiness

**Status: not deployed.** No hosted instance is running, and no free-tier account or platform has been verified as part of this work. The steps below are readiness notes only.

## Cost ($0 goal)

- Application: Rust/Axum binary, no license fee.
- Database: embedded SQLite file on local disk (`data/scout.db` by default).
- Discovery: reads the CC-BY-4.0 [Awesome Discord Communities](https://github.com/mhxion/awesome-discord-communities) list via GitHub's public raw-content endpoint; no API key required. Requests are cached in memory and bounded by timeouts.
- Message drafts: deterministic templates in-process; **no AI API and no key required**.
- External paid services: **none**.

## Local / self-host run (verified path)

```bash
cp .env.example .env   # optional
cargo build --release
./target/release/discord-community-scout
```

Windows (PowerShell):

```powershell
$env:PORT='8080'
$env:DATABASE_URL='sqlite://data/scout.db?mode=rwc'
.\target\release\discord-community-scout.exe
```

Open `http://127.0.0.1:8080`. The SQLite file and parent directory are created automatically; migrations run at startup.

## Container image (buildable, not published)

A multi-stage [`Dockerfile`](../Dockerfile) is included. Building/publishing it was **not** verified in a hosted environment in this run.

```bash
docker build -t discord-community-scout:local .
docker run --rm -p 8080:8080 -v scout_data:/app/data discord-community-scout:local
```

Persist `/app/data` so the SQLite database survives container restarts.

## Free-tier hosting notes (unverified)

Common $0 tiers (for example Fly.io, Render, Railway free/hobby plans, or a small VPS credit) **change frequently** and were **not** tested or claimed here. Before deploying:

- Confirm the platform still offers a genuinely free tier and its limits (sleeping, CPU/RAM, egress).
- Ensure a **persistent disk/volume** is attached for `/app/data`; SQLite on ephemeral storage loses data on restart.
- Run a single instance (SQLite is not a multi-writer network database).
- Set `PORT` to the platform-provided port if required, and keep `DATABASE_URL` pointing at the mounted volume.
- Do not expose internal error detail; the app already returns a stable error envelope.

Because none of the above was executed against a live host, deployment remains **not completed**.

## Health check

`GET /api/health` returns `200 {"status":"ok","database":"ok",...}` when the app and database are reachable, or `503` with `"status":"degraded"` if the database cannot be queried. Use this for container/platform health probes.

## Operational caveats

- Discovery depends on GitHub's public availability of the curated list; if it is unreachable the app returns a user-facing `upstream_error` and CSV/manual entry continue to work.
- Only the audited, permitted discovery source is enabled; rejected directories remain unavailable by design (see [`data-sources.md`](data-sources.md)).
- The app never automates a Discord account and never posts messages. Drafts are copy/paste artifacts for human review.
