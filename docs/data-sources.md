# Data sources and platform policy audit

Initial audit date: **2026-11-16**. Follow-up review: **2026-11-16** (broader-audience discovery check). Every statement below reflects what was checked on those dates. Sources change; re-verify before enabling a connector.

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
| **Top.gg** — <https://docs.top.gg/> | Bot/project APIs only; no documented public server/community search endpoint for arbitrary discovery. Public API docs pages were not openly accessible during recheck (403/404). | No documented, permitted way to search *servers* for this app's use. | **Rejected** — no documented community-search API for this use. |
| **DISBOARD** — <https://disboard.org/> | None published. `https://disboard.org/site/tos` returned **HTTP 403** to a plain automated request; `robots.txt` allows general browsing but signals restrictions for many automated/training bots. | Undocumented, access-controlled. No permission to scrape/bulk-collect listings was established, and prior automated requests were blocked (403). | **Rejected** — no documented API, automated access blocked / not permitted. |
| **Discord.me** — <https://discord.me/> | None published. | ToS (<https://discord.me/tos>, v3 2025-10-07) explicitly prohibits scraping/harvesting user data without explicit permission and using automated tools/bots to interact with the platform without explicit permission. | **Rejected** — automated retrieval not permitted. |
| **DiscordServers.com** — <https://discordservers.com/> | `https://discordservers.com/api` returned **HTTP 404** in the initial audit; no public server-search API documentation found. | No documented, permitted API for community search was identified. | **Rejected** — cannot establish permitted automated access. |
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

## 6. Broader-audience discovery (follow-up, 2026-11-16)

Requested audiences reviewed: students, online earning, freelancing, remote work, side hustles, entrepreneurship, digital marketing, business/startups, programming/technology.

- The integrated, permitted source (**Awesome Discord Communities**, CC-BY-4.0, GitHub raw content) is primarily programming/technology-focused.
- Rechecked candidate directories above: DISBOARD, Discord.me, DiscordServers.com, Top.gg, and Discord official discovery remain **unusable/unpermitted** for automated community discovery in this app (no documented permitted server-search API, explicit ToS prohibitions on automated scraping/harvesting/tools, or access blocked).
- No additional Discord-community-specific curated dataset with a clear permissive license and verifiable permission for automated retrieval covering the requested non-programming audiences was identified during this review. No unverified connector was added.

**Result:** automated discovery coverage beyond the programming-focused Awesome list remains limited. **CSV import and manual entry are the supported, fully functional path** for students/freelancing/remote-work/side-hustle/entrepreneurship/marketing/business communities, using user-supplied or otherwise legitimately obtained listings. This limitation is stated in-app and in the README.

## 7. Re-verification checklist

Before adding or re-enabling a connector:

1. Open the source's current API documentation and terms.
2. Confirm automated querying and storage of listing data are permitted.
3. Confirm rate limits, quotas, attribution and caching requirements.
4. Record the date, URL and findings in this file.
5. Only then write the connector.
