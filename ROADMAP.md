# Roadmap

Status of the eight milestones in the build brief. A milestone is only marked
done when its code is committed, builds, and its tests pass.

| # | Milestone | Status |
| --- | --- | --- |
| 0 | Repository and source-policy audit | done — `docs/data-sources.md`, `docs/architecture.md` |
| 1 | Working application foundation (Axum, health, config, logging, CI) | done |
| 2 | Database, community CRUD, dashboard metrics, search/filter/sort | done — committed with milestone 4 |
| 3 | CSV import/export and manual entry | done |
| 4 | Relevance scoring and review workflow | done |
| 5 | Verified public discovery connector (Awesome Discord Communities) | done — enabled with an in-memory cache; other directories rejected in the audit |
| 6 | Message generator | done — deterministic templates with disclosure and forbidden-claim tests |
| 7 | Polish, security, usability | in progress |
| 8 | Deployment readiness and final verification | planned |

## Known constraints recorded up front

- **No general-purpose Discord directory API is available for free.** Top.gg,
  DISBOARD, Discord.me, DiscordServers and Discord's own discovery were all
  rejected in the audit; see `docs/data-sources.md`. The one accepted source is a
  CC-BY-4.0 curated list read through the GitHub API, and it covers programming
  and technical communities rather than the "make money online" audiences.
- **CSV import and manual entry are therefore first-class features**, not
  afterthoughts, because the accepted source does not cover every audience.
- **Deployment is not claimed.** No free host's current terms could be verified
  offline, and deployment needs the owner's account. The repo ships a Dockerfile
  and a deployment guide instead.
- **The opportunity is never described as verified.** The generator emits drafts
  with the $12 deposit disclosed; whether to post is a human decision.
