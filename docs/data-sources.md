# Data sources and platform policy audit

Audit date: **2026-11-16**. Every statement below was checked against the linked
document on that date. Sources change; re-verify before enabling a connector.

## 1. Discord platform rules

Checked: <https://discord.com/developers/docs/policies-and-agreements/developer-policy>
(Discord Developer Policy, effective 2024-07-08, last updated 2024-06-06).

Clauses that constrain this application:

| Clause | Text (abridged) | Consequence for this app |
| --- | --- | --- |
| §1, §2 | Do not modify a user's account or initiate processes on a user's or server's behalf without explicit permission | No account automation of any kind |
| §5 | Do not contact users on Discord without their explicit permission | No DMs, no auto-posting |
| §6 | Do not target users with advertisements or marketing | Drafts are for human review only |
| §13 | Do not misrepresent or fraudulently manipulate engagement, including inflating server membership | No fake members, no engagement inflation |
| §20 | **Do not mine or scrape any data, content, or information available on or through Discord services** | Discord itself is never fetched, crawled, or queried outside documented APIs |

Hard consequences adopted in code:

- No self-bot, no user-token automation, no join/DM/post/react automation.
- No reading of guild member lists, message history, or private channels.
- No undocumented Discord endpoints (`/api/v9/discoverable-guilds` and similar are
  explicitly out of scope).
- The message generator produces **drafts only**. There is no send path in the
  codebase at all.
- Discovery never contacts Discord. It reads independent public directories and
  user-supplied data.

## 2. Candidate community directories

| Source | Documented public API? | Result | Verdict |
| --- | --- | --- | --- |
| **Top.gg** — <https://docs.top.gg/> | No server/community search endpoint. v0 documents `GET /bots`, `GET /bots/:id/stats`, `GET /bots/:id/check`, `POST /bots/:id/stats`; v1 documents projects, votes, webhooks, OAuth, users. All bot endpoints require a Top.gg token and are limited to 60 req/min. | No documented way to search *servers*. A token would have to be registered to a project the user owns, and the data may only be used for that project's stated functionality (§15 of Discord's policy is about Discord API data, but Top.gg's own terms restrict token use to the registered project). | **Rejected** — no documented community-search API; token cannot be legitimately obtained for this use. |
| **DISBOARD** — <https://disboard.org/> | None published. `https://disboard.org/terms` returned **HTTP 403** to a plain automated request, indicating bot mitigation. | Undocumented, access-controlled. Scraping is not permitted by any statement we could read, and we will not reverse-engineer. | **Rejected** — no documented API, automated access blocked. |
| **Discord.me** — <https://discord.me/> | None published. `https://discord.me/terms` returned **HTTP 404**. | No documented API and no retrievable terms. | **Rejected** — cannot establish permission. |
| **DiscordServers.com** — <https://discordservers.com/> | `https://discordservers.com/api` returned **HTTP 404**. No API documentation found. | No documented API. | **Rejected** — cannot establish permission. |
| **Discord official Server Discovery** — <https://discord.com/servers> | No public API. Requires an authenticated browser session. | Fetching it would be scraping Discord services, prohibited by Developer Policy §20. | **Rejected** — prohibited. |

No candidate directory offers a free, documented, publicly queryable API for
community search. Nothing was scraped to reach this conclusion: the only
requests made were to published documentation and `/terms` pages, and the
blocked ones (403) were not retried or worked around.

## 3. Accepted source: Awesome Discord Communities

- Repository: <https://github.com/mhxion/awesome-discord-communities>
- License: **CC-BY-4.0** (`license.spdx_id`, verified via the GitHub API)
- Accessed through the **GitHub REST API** (<https://docs.github.com/en/rest>),
  which is documented public infrastructure, not the directory's own site.
- Data used: `README.md`, a generated Markdown document listing ~270 public
  communities with name, invite URL, category and subcategory.
- Authentication: none required for public repositories. Unauthenticated rate
  limit is 60 requests/hour per IP; this app performs at most one fetch per hour
  thanks to a cache, so it stays far below the limit.
- Attribution: CC-BY-4.0 requires credit. The UI labels every record from this
  source and links back to the repository; `docs/api.md` documents it too.
- Restrictions honoured: the list is *not* a Discord listing or promotional
  site, so records are never described as "verified" or "endorsed".

Why this is acceptable where the directories are not: the content is explicitly
published under a licence that permits reuse with attribution, and the transport
is GitHub's documented API rather than an unofficial endpoint.

## 4. Always-available fallback

Two paths never depend on any third party:

- **CSV import** of community listings (documented header format, preview,
  row-level validation).
- **Manual entry** of a single community.

Imported and manual records are stored with `source = "CSV import"` or
`source = "Manual entry"` and are labelled as such everywhere in the UI. They
are never shown as live discovery results.

## 5. Provenance rules

- Every community row has at least one `community_sources` row recording the
  source name, external id, canonical listing URL and first/last observed time.
- A community found twice is merged on normalized invite code, and the second
  source is attached as an additional provenance row rather than overwriting the
  first.
- Deleting a community deletes its provenance rows (cascade).

## 6. Re-verification checklist

Before adding or re-enabling a connector:

1. Open the source's current API documentation and terms.
2. Confirm automated querying and storage of listing data are permitted.
3. Confirm rate limits, quotas, attribution and caching requirements.
4. Record the date, URL and findings in this file.
5. Only then write the connector.
