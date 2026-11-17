# Roadmap

Status of the build milestones. A milestone is only marked done when its code is
committed, builds, and its tests pass.

| # | Milestone | Status |
| --- | --- | --- |
| 0 | Repository and source-policy audit | done — `docs/data-sources.md`, `docs/architecture.md` |
| 1 | Working application foundation (Axum, health, config, logging, CI) | done |
| 2 | Database, community CRUD, dashboard metrics, search/filter/sort | done — committed with milestone 4 |
| 3 | CSV import/export and manual entry | done |
| 4 | Relevance scoring and review workflow | done |
| 5 | Verified public discovery connector (Awesome Discord Communities) | done — enabled with an in-memory cache; other directories rejected in the audit |
| 6 | Message generator | done — deterministic templates with disclosure and forbidden-claim tests |
| 7 | Polish, security, usability | done — responsive light UI, accessible dialogs, Lighthouse 100/100/100 |
| 8 | Deployment readiness and final verification | done — Dockerfile, `.dockerignore`, `docs/deployment.md`, README updated |
| 9 | Oracle Cloud Always Free deployment packaging | done — `deploy/` compose + Caddyfile + env template, `scripts/` install/deploy/backup/restore/verify, HTTPS + bcrypt Basic Auth, firewall, DuckDNS updater, backup timer; Compose v2.29.7 and Caddy 2.8.4 validation passed; production image built and smoke-tested on a native ARM64 runner |
| 10 | Live deployment and restore proof | **blocked** — needs an Oracle Cloud account/VM and DuckDNS hostname owned by the user; no public URL exists yet |

## Deployment status (milestone 10)

Prepared and statically validated, **not live**:

- [x] Compose with app + Caddy + optional DuckDNS, internal network only
- [x] Caddy HTTPS, HTTP->HTTPS redirect, bcrypt Basic Auth on all routes
- [x] Persistent `/app/data`, Caddy certificate volumes, restart on boot
- [x] Backup + verify + restore scripts and a daily systemd timer
- [x] Oracle Always Free limits, idle reclamation and trial-vs-free documented
- [x] `docker compose config` (both profiles) and `caddy validate` pass
- [x] Production `Dockerfile` builds for ARM64 and a record survives a container restart (CI, `ubuntu-24.04-arm`)
- [ ] Oracle account, Always Free A1 VM, Security List rules
- [ ] DuckDNS hostname pointing at the VM
- [ ] Live HTTPS certificate issued
- [ ] `verify-deployment.sh --full` passing against the public URL
- [ ] Data persistence across restart/reboot proven on the VM
- [ ] Backup + actual restore test on the VM
- [ ] Desktop and mobile browser check against the public URL

## Known constraints recorded up front

- **No general-purpose Discord directory API is available for free.** Top.gg,
  DISBOARD, Discord.me, DiscordServers and Discord's own discovery were all
  rejected in the audit; see `docs/data-sources.md`. The one accepted source is a
  CC-BY-4.0 curated list read through the GitHub API, and it covers programming
  and technical communities rather than the "make money online" audiences.
- **CSV import and manual entry are therefore first-class features**, not
  afterthoughts, because the accepted source does not cover every audience.
- **Deployment is packaged but not live.** The repository now ships an Oracle
  Cloud Always Free Compose stack (Caddy HTTPS + Basic Auth, backups). Creating
  the account, VM, DNS record and running the live verification needs the
  owner's account and has not been done; no URL is claimed.
- **The opportunity is never described as verified.** The generator emits drafts
  with the $12 deposit disclosed; whether to post is a human decision.
