// Discord Community Scout — single-page frontend. No build step, no framework.
//
// Security note: every value that comes from the API is inserted with
// textContent (via the `el` helper) and never with innerHTML, so community
// names, notes and rule text cannot inject markup.

/* ----------------------------------------------------------------- utilities */

/**
 * Creates an element. Supported props: class, text, and any attribute.
 * Event handlers are passed as on<event>.
 */
function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on") && typeof value === "function") {
      node.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (key === "value") node.value = value;
    else if (key === "checked") node.checked = Boolean(value);
    else node.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function $(selector, root = document) {
  return root.querySelector(selector);
}

function toast(message, kind = "") {
  const stack = $("#toast-stack");
  const item = el("div", { class: `toast ${kind}`.trim(), role: "status", text: message });
  stack.append(item);
  window.setTimeout(() => item.remove(), 4200);
}

function formatDate(iso) {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, {
    year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

function isSafeHttpUrl(value) {
  if (typeof value !== "string") return false;
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

const STATUS_LABEL = {
  not_reviewed: "Not reviewed",
  suitable: "Suitable",
  unsuitable: "Unsuitable",
  needs_manual_check: "Needs manual check",
  contacted: "Contacted / posted",
};

const STATUS_TONE = {
  not_reviewed: "neutral",
  suitable: "ok",
  unsuitable: "danger",
  needs_manual_check: "warn",
  contacted: "info",
};

const POLICY_LABEL = { allowed: "Allowed", prohibited: "Prohibited", unknown: "Not verified" };
const POLICY_TONE = { allowed: "ok", prohibited: "danger", unknown: "warn" };

function pill(text, tone) {
  return el("span", { class: `pill pill-${tone}`, text });
}

function statusPill(status) {
  return pill(STATUS_LABEL[status] || status, STATUS_TONE[status] || "neutral");
}

function policyPill(policy) {
  return pill(POLICY_LABEL[policy] || policy, POLICY_TONE[policy] || "neutral");
}

function banner(text, tone = "info", title = "") {
  return el("div", { class: `banner banner-${tone}`, role: "status" },
    title ? el("strong", { text: title }) : null,
    text);
}

function emptyState(title, text, ...actions) {
  return el("div", { class: "empty" },
    el("h3", { text: title }),
    el("p", { class: "small", text }),
    actions.length ? el("div", { class: "row", style: "justify-content:center" }, actions) : null);
}

function loading(node, label = "Loading…") {
  clear(node);
  node.append(el("p", { class: "muted small" },
    el("span", { class: "spinner", "aria-hidden": "true" }), ` ${label}`));
}

/* ----------------------------------------------------------------------- api */

async function api(path, { method = "GET", body } = {}) {
  const headers = { Accept: "application/json" };
  if (body !== undefined) headers["Content-Type"] = "application/json";

  const response = await fetch(`/api${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  if (response.status === 204) return null;

  const raw = await response.text();
  let data = null;
  if (raw) {
    try { data = JSON.parse(raw); } catch { data = null; }
  }

  if (!response.ok) {
    const message = data?.error?.message || `Request failed (HTTP ${response.status}).`;
    const error = new Error(message);
    error.status = response.status;
    error.code = data?.error?.code;
    throw error;
  }
  return data;
}

/* --------------------------------------------------------------------- state */

const state = {
  view: "overview",
  meta: { review_statuses: [], promotion_policies: [], sources: [], tags: [] },
  filters: { q: "", status: "", source: "", tag: "", sort: "relevance", outreach_only: false, page: 1, per_page: 25 },
  editingId: null,
  reviewingId: null,
  draft: null,
  draftShowsAlternative: false,
  lastCsv: "",
};

/* -------------------------------------------------------------------- router */

const VIEWS = ["overview", "discover", "communities", "drafts", "about"];

function setView(view, { updateHash = true } = {}) {
  if (!VIEWS.includes(view)) view = "overview";
  state.view = view;

  for (const id of VIEWS) {
    const section = document.getElementById(`view-${id}`);
    if (section) section.hidden = id !== view;
  }
  document.querySelectorAll(".nav-link").forEach((link) => {
    if (link.dataset.view === view) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  });

  if (updateHash && location.hash !== `#${view}`) {
    history.replaceState(null, "", `#${view}`);
  }

  window.scrollTo({ top: 0 });
  activateView(view);
}

function activateView(view) {
  if (view === "overview") loadOverview();
  if (view === "discover") { loadSources(); loadRuns(); }
  if (view === "communities") { loadMeta(); loadCommunities(); }
  if (view === "drafts") loadDraftCommunities();
  if (view === "about") { loadSources(); loadHealth(); renderAboutSync(); }
}

/* ------------------------------------------------------------------ overview */

async function loadOverview() {
  const grid = $("#stat-grid");
  const activity = $("#last-activity");
  loading(grid, "Loading dashboard…");

  try {
    const d = await api("/dashboard");

    const stats = [
      { label: "Saved", value: d.total_communities, tone: "accent", view: "communities" },
      { label: "Awaiting review", value: d.awaiting_review, tone: "warn", view: "communities" },
      { label: "Reviewed", value: d.reviewed, tone: "", view: "communities" },
      { label: "Suitable", value: d.suitable, tone: "ok", view: "communities" },
      { label: "Unsuitable", value: d.unsuitable, tone: "danger", view: "communities" },
      { label: "Contacted / posted", value: d.contacted, tone: "ok", view: "communities" },
      { label: "Needs manual check", value: d.needs_manual_check, tone: "warn", view: "communities" },
      { label: "Promotion prohibited", value: d.promotion_prohibited, tone: "danger", view: "communities" },
    ];

    clear(grid);
    for (const stat of stats) {
      const card = el("button", {
        class: `stat ${stat.tone}`.trim(),
        type: "button",
        style: "text-align:left;cursor:pointer;font:inherit",
        onclick: () => setView(stat.view),
      },
        el("div", { class: "stat-value", text: String(stat.value) }),
        el("div", { class: "stat-label", text: stat.label }),
        // The visible label is the accessible name; this only adds context, so
        // screen readers never hear something different from what is shown.
        el("span", { class: "sr-only", text: ` — open ${stat.label.toLowerCase()}` }));
      grid.append(card);
    }

    clear(activity);
    if (d.last_activity) {
      activity.append(
        el("strong", { text: d.last_activity.kind }),
        ` — ${d.last_activity.detail} · ${formatDate(d.last_activity.at)}`);
    } else if (d.total_communities === 0) {
      activity.append("No communities saved yet. ",
        el("button", {
          class: "btn btn-sm btn-ghost", type: "button", text: "Discover one",
          onclick: () => setView("discover"),
        }));
    } else {
      activity.append("No recorded activity yet.");
    }
  } catch (error) {
    clear(grid);
    grid.append(banner(`Could not load the dashboard: ${error.message}`, "danger"));
    clear(activity);
    activity.textContent = "Unavailable.";
  }
}

/* ------------------------------------------------------------------ discover */

function sourceRow(source, { linkable = true } = {}) {
  const title = linkable && isSafeHttpUrl(source.homepage)
    ? el("a", { href: source.homepage, target: "_blank", rel: "noopener noreferrer", text: source.name })
    : el("span", { text: source.name });

  return el("div", { class: "result" },
    el("div", { class: "result-head" },
      el("h3", {}, title),
      source.enabled ? pill("Integrated", "ok") : pill("Not available", "neutral")),
    el("p", { class: "meta", text: `License: ${source.license}` }),
    el("p", { class: "small", text: source.notes }));
}

async function loadSources() {
  const list = $("#source-list");
  const about = $("#about-sources");
  if (list) loading(list);
  if (about) loading(about);

  try {
    const data = await api("/sources");

    if (list) {
      clear(list);
      if (!data.discovery_available) {
        list.append(banner(
          "No discovery source is currently available. CSV import and manual entry still work.",
          "warn", "Discovery is not configured"));
      }
      for (const source of data.sources) list.append(sourceRow(source));
    }

    if (about) {
      clear(about);
      for (const source of data.sources) about.append(sourceRow(source));
      if (!data.discovery_available) {
        about.prepend(banner(
          "Discovery is unavailable; use CSV import or manual entry.",
          "warn"));
      }
    }
  } catch (error) {
    if (list) { clear(list); list.append(banner(`Could not load sources: ${error.message}`, "danger")); }
    if (about) { clear(about); about.append(banner(`Could not load sources: ${error.message}`, "danger")); }
  }
}

async function loadRuns() {
  const node = $("#run-list");
  if (!node) return;
  loading(node, "Loading runs…");
  try {
    const runs = await api("/discovery/runs?limit=10");
    clear(node);
    if (!runs.length) {
      node.append(emptyState("No discovery runs yet", "Run a search above and it will be recorded here."));
      return;
    }
    const rows = runs.map((run) => el("tr", {},
      el("td", {}, el("div", { class: "cell-name", text: run.query })),
      el("td", {}, pill(run.status, run.status === "failed" ? "danger" : run.status === "partial" ? "warn" : "ok")),
      el("td", { text: String(run.result_count) }),
      el("td", { text: String(run.saved_count) }),
      el("td", {}, el("span", { class: "small muted", text: formatDate(run.started_at) })),
      el("td", {}, run.error ? el("span", { class: "small", text: run.error }) : el("span", { class: "muted", text: "—" }))));

    node.append(el("div", { class: "table-wrap" },
      el("table", {},
        el("thead", {}, el("tr", {},
          el("th", { text: "Query" }), el("th", { text: "Status" }),
          el("th", { text: "Matched" }), el("th", { text: "Saved" }),
          el("th", { text: "Started" }), el("th", { text: "Error" }))),
        el("tbody", {}, rows))));
  } catch (error) {
    clear(node);
    node.append(banner(`Could not load runs: ${error.message}`, "danger"));
  }
}

function scoreBadge(community) {
  const blocked = community.unsuitable_for_outreach;
  const low = community.relevance_score < 40;
  const cls = blocked ? "blocked" : low ? "low" : "";
  return el("span", { class: `score-badge ${cls}`.trim(), title: "Relevance score out of 100", text: `${community.relevance_score}/100` });
}

function breakdownList(factors) {
  return el("ul", { class: "breakdown-list" },
    factors.map((factor) => el("li", {},
      el("span", { class: "b-factor", text: factor.factor.replace(/_/g, " ") }),
      el("span", { class: "b-points", text: `${factor.points > 0 ? "+" : ""}${factor.points}${factor.max ? ` / ${factor.max}` : ""}` }),
      el("span", { class: "b-reason", text: factor.reason }))));
}

function discoverResult(hit) {
  const c = hit.community;
  const tags = el("div", { class: "tags" },
    c.tags.slice(0, 8).map((tag) => el("span", { class: "tag", text: tag })));

  const metaParts = [`Source: ${c.primary_source}`];
  if (c.member_count) metaParts.push(`${c.member_count.toLocaleString()} members (from source)`);
  if (c.language) metaParts.push(c.language);

  const links = el("div", { class: "row", style: "margin-top:.5rem" });
  if (isSafeHttpUrl(c.invite_url)) {
    links.append(el("a", { class: "btn btn-sm", href: c.invite_url, target: "_blank", rel: "noopener noreferrer", text: "Open invite" }));
  }
  if (isSafeHttpUrl(c.canonical_url)) {
    links.append(el("a", { class: "btn btn-sm", href: c.canonical_url, target: "_blank", rel: "noopener noreferrer", text: "Source listing" }));
  }
  links.append(el("button", {
    class: "btn btn-sm btn-primary", type: "button", text: "Review",
    onclick: () => openReview(c),
  }));

  return el("div", { class: "result" },
    el("div", { class: "result-head" },
      el("h3", { text: c.name }),
      el("div", { class: "row" },
        pill(hit.status === "new" ? "Newly saved" : "Already saved", hit.status === "new" ? "info" : "neutral"),
        scoreBadge(c))),
    el("p", { class: "meta", text: metaParts.join(" · ") }),
    el("p", { class: "small", text: c.description }),
    c.tags.length ? tags : null,
    c.unsuitable_for_outreach
      ? banner(c.outreach_note, "danger")
      : banner(c.outreach_note, c.promotion_policy === "allowed" ? "info" : "warn"),
    links,
    el("details", { class: "breakdown" },
      el("summary", { text: "Why this score" }),
      breakdownList(c.score_breakdown)));
}

async function runDiscovery(event) {
  event.preventDefault();
  const query = $("#discover-query").value.trim();
  const limit = Number($("#discover-limit").value);
  const status = $("#discover-status");
  const results = $("#discover-results");

  if (!query) {
    clear(status);
    status.append(banner("Enter at least one topic or keyword.", "warn"));
    $("#discover-query").focus();
    return;
  }

  clear(status);
  status.append(el("p", { class: "muted small" },
    el("span", { class: "spinner", "aria-hidden": "true" }),
    " Searching the permitted source…"));
  clear(results);
  $("#discover-submit").disabled = true;

  try {
    const data = await api("/discovery/search", { method: "POST", body: { query, limit } });

    clear(status);
    if (data.returned === 0) {
      status.append(banner(
        `No communities matched “${query}” in the permitted source. Try broader terms, or import a CSV of listings.`,
        "warn", "No matches"));
    } else {
      status.append(banner(
        `${data.returned} of ${data.total_matched} matches saved · ${data.duration_ms} ms · ${data.attribution}`,
        data.partial ? "warn" : "ok", data.partial ? "Partial results" : "Search complete"));
    }

    for (const hit of data.results) results.append(discoverResult(hit));

    loadRuns();
    loadOverview();
    refreshSyncBadge();
  } catch (error) {
    clear(status);
    status.append(banner(
      `${error.message}`,
      error.status === 502 ? "warn" : "danger",
      error.status === 502 ? "Discovery source unavailable" : "Search failed"));
    status.append(el("div", { class: "row" },
      el("button", { class: "btn btn-sm", type: "button", text: "Import a CSV instead", onclick: () => setView("communities") }),
      el("button", { class: "btn btn-sm", type: "button", text: "Add a community by hand", onclick: () => { setView("communities"); openCommunityDialog(); } })));
  } finally {
    $("#discover-submit").disabled = false;
  }
}

/* --------------------------------------------------------------- communities */

async function loadMeta() {
  try {
    const meta = await api("/meta");
    state.meta = meta;

    const statusSelect = $("#filter-status");
    const sourceSelect = $("#filter-source");
    const tagSelect = $("#filter-tag");

    fillSelect(statusSelect, "Any status",
      meta.review_statuses.map((value) => ({ value, label: STATUS_LABEL[value] || value })),
      state.filters.status);

    fillSelect(sourceSelect, "Any source",
      meta.sources.map((s) => ({ value: s.name, label: `${s.name} (${s.count})` })),
      state.filters.source);

    fillSelect(tagSelect, "Any tag",
      meta.tags.map((t) => ({ value: t.name, label: `${t.name} (${t.count})` })),
      state.filters.tag);
  } catch (error) {
    toast(`Could not load filter options: ${error.message}`, "error");
  }
}

function fillSelect(select, placeholder, options, selected) {
  if (!select) return;
  clear(select);
  select.append(el("option", { value: "", text: placeholder }));
  for (const option of options) {
    select.append(el("option", { value: option.value, text: option.label, selected: option.value === selected }));
  }
}

function communityQuery() {
  const params = new URLSearchParams();
  const f = state.filters;
  if (f.q) params.set("q", f.q);
  if (f.status) params.set("status", f.status);
  if (f.source) params.set("source", f.source);
  if (f.tag) params.set("tag", f.tag);
  if (f.sort) params.set("sort", f.sort);
  if (f.outreach_only) params.set("outreach_only", "true");
  params.set("page", String(f.page));
  params.set("per_page", String(f.per_page));
  return params.toString();
}

async function loadCommunities() {
  const tableNode = $("#communities-table");
  const statusNode = $("#communities-status");
  const pager = $("#communities-pagination");
  loading(tableNode, "Loading communities…");
  clear(statusNode);
  clear(pager);

  try {
    const page = await api(`/communities?${communityQuery()}`);
    clear(tableNode);

    if (page.total === 0) {
      const hasFilters = Object.values(state.filters).some((v, i) => i < 5 && v && v !== "relevance");
      tableNode.append(emptyState(
        hasFilters ? "No communities match these filters" : "No communities saved yet",
        hasFilters
          ? "Try clearing the filters, or add a community to get started."
          : "Discover from a permitted source, import a CSV, or add one by hand.",
        el("button", { class: "btn btn-primary btn-sm", type: "button", text: "Add community", onclick: () => openCommunityDialog() }),
        el("button", { class: "btn btn-sm", type: "button", text: "Discover", onclick: () => setView("discover") })));
      return;
    }

    const rows = page.items.map((c) => el("tr", {},
      el("td", {},
        el("div", { class: "cell-name", text: c.name }),
        c.description ? el("div", { class: "cell-sub truncate", text: c.description }) : null,
        c.tags.length ? el("div", { class: "tags" }, c.tags.slice(0, 3).map((tag) => el("span", { class: "tag", text: tag }))) : null),
      el("td", {}, el("span", { class: "small", text: c.primary_source })),
      el("td", {}, statusPill(c.review_status)),
      el("td", {}, policyPill(c.promotion_policy)),
      el("td", {}, scoreBadge(c)),
      el("td", {}, el("span", { class: "small muted", text: formatDate(c.updated_at) })),
      el("td", {}, el("div", { class: "actions" },
        el("button", { class: "btn btn-sm", type: "button", text: "Review", onclick: () => openReview(c) }),
        el("button", { class: "btn btn-sm", type: "button", text: "Edit", onclick: () => openCommunityDialog(c) })))));

    tableNode.append(el("div", { class: "table-wrap" },
      el("table", {},
        el("thead", {}, el("tr", {},
          el("th", { text: "Community" }), el("th", { text: "Source" }),
          el("th", { text: "Status" }), el("th", { text: "Policy" }),
          el("th", { text: "Score" }), el("th", { text: "Updated" }),
          el("th", { text: "Actions" }))),
        el("tbody", {}, rows))));

    const from = (page.page - 1) * page.per_page + 1;
    const to = Math.min(page.page * page.per_page, page.total);
    pager.append(
      el("span", { class: "small muted", text: `Showing ${from}–${to} of ${page.total} · page ${page.page} of ${page.total_pages}` }),
      el("div", { class: "row" },
        el("button", {
          class: "btn btn-sm", type: "button", text: "Previous",
          disabled: page.page <= 1,
          onclick: () => { state.filters.page = page.page - 1; loadCommunities(); },
        }),
        el("button", {
          class: "btn btn-sm", type: "button", text: "Next",
          disabled: page.page >= page.total_pages,
          onclick: () => { state.filters.page = page.page + 1; loadCommunities(); },
        })));
  } catch (error) {
    clear(tableNode);
    tableNode.append(banner(`Could not load communities: ${error.message}`, "danger"));
  }
}

function readFilters() {
  state.filters.q = $("#filter-q").value.trim();
  state.filters.status = $("#filter-status").value;
  state.filters.source = $("#filter-source").value;
  state.filters.tag = $("#filter-tag").value;
  state.filters.sort = $("#filter-sort").value;
  state.filters.outreach_only = $("#filter-outreach").checked;
  state.filters.page = 1;
}

/* ---------------------------------------------------------- community dialog */

function openCommunityDialog(community = null) {
  $(".dialog-head h2", $("#community-dialog")).textContent = community ? "Edit community" : "Add community";
  state.editingId = community ? community.id : null;

  const form = $("#community-form");
  form.reset();
  $("#community-form-error").replaceChildren();

  if (community) {
    $("#cf-name").value = community.name;
    $("#cf-description").value = community.description || "";
    $("#cf-invite").value = community.invite_url || "";
    $("#cf-canonical").value = community.canonical_url || "";
    $("#cf-tags").value = (community.tags || []).join(", ");
    $("#cf-language").value = community.language || "";
    $("#cf-policy").value = community.promotion_policy || "unknown";
    $("#cf-channel").value = community.channel_name || "";
    $("#cf-evidence").value = community.policy_evidence || "";
    $("#cf-notes").value = community.user_notes || "";
  }

  $("#community-dialog").showModal();
  $("#cf-name").focus();
}

async function saveCommunity(event) {
  event.preventDefault();
  const errorBox = $("#community-form-error");
  errorBox.replaceChildren();

  const tags = $("#cf-tags").value.split(",").map((t) => t.trim()).filter(Boolean);
  const payload = {
    name: $("#cf-name").value,
    description: $("#cf-description").value || null,
    invite_url: $("#cf-invite").value || null,
    canonical_url: $("#cf-canonical").value || null,
    tags,
    language: $("#cf-language").value || null,
    channel_name: $("#cf-channel").value || null,
    promotion_policy: $("#cf-policy").value,
    policy_evidence: $("#cf-evidence").value || null,
    user_notes: $("#cf-notes").value || null,
  };

  const button = $("#community-save");
  button.disabled = true;
  try {
    if (state.editingId) {
      await api(`/communities/${state.editingId}`, { method: "PATCH", body: payload });
      toast("Community updated.", "ok");
    } else {
      await api("/communities", { method: "POST", body: payload });
      toast("Community saved.", "ok");
    }
    $("#community-dialog").close();
    loadMeta();
    loadCommunities();
    loadOverview();
    refreshSyncBadge();
  } catch (error) {
    errorBox.append(banner(error.message, "danger"));
  } finally {
    button.disabled = false;
  }
}

/* ------------------------------------------------------------- review dialog */

function renderReviewSummary(community) {
  const node = $("#review-summary");
  clear(node);

  const facts = el("dl", { class: "facts" },
    el("dt", { text: "Source" }), el("dd", { text: community.primary_source }),
    el("dt", { text: "Tags" }), el("dd", { text: community.tags.length ? community.tags.join(", ") : "—" }),
    el("dt", { text: "Discovered" }), el("dd", { text: formatDate(community.created_at) }),
    el("dt", { text: "Last checked" }), el("dd", { text: formatDate(community.last_checked_at) }),
    el("dt", { text: "Contacted" }), el("dd", { text: formatDate(community.contacted_at) }));

  const links = el("div", { class: "row", style: "margin-top:.5rem" });
  if (isSafeHttpUrl(community.invite_url)) {
    links.append(el("a", { class: "btn btn-sm", href: community.invite_url, target: "_blank", rel: "noopener noreferrer", text: "Open invite" }));
  }
  if (isSafeHttpUrl(community.canonical_url)) {
    links.append(el("a", { class: "btn btn-sm", href: community.canonical_url, target: "_blank", rel: "noopener noreferrer", text: "Source listing" }));
  }

  node.append(
    el("h3", { text: community.name }),
    community.description ? el("p", { class: "small", text: community.description }) : null,
    facts,
    links);

  const warnings = $("#review-warnings");
  clear(warnings);
  if (community.promotion_policy === "prohibited") {
    warnings.append(banner(
      "This community prohibits promotion or recruitment. It is unsuitable for outreach. There is no override.",
      "danger", "Do not post here"));
  } else if (community.promotion_policy === "unknown") {
    warnings.append(banner(
      "The rules have not been verified. Read the community's posted rules before posting anything.",
      "warn", "Not verified"));
  } else {
    warnings.append(banner(
      "You recorded that promotion is allowed. Confirm again before posting.",
      "info", "Allowed (your record)"));
  }
  if (community.contacted_at) {
    warnings.append(banner(`You already contacted this community on ${formatDate(community.contacted_at)}. Avoid posting twice.`, "warn"));
  }
}

function renderReviewBreakdown(community) {
  $("#review-score-line").textContent =
    `Total ${community.relevance_score}/100 — each factor below explains its contribution. Scores above 100 are impossible; a negated cap appears as its own row.`;
  const list = $("#review-breakdown");
  clear(list);
  for (const factor of community.score_breakdown) {
    list.append(el("li", {},
      el("span", { class: "b-factor", text: factor.factor.replace(/_/g, " ") }),
      el("span", { class: "b-points", text: `${factor.points > 0 ? "+" : ""}${factor.points}${factor.max ? ` / ${factor.max}` : ""}` }),
      el("span", { class: "b-reason", text: factor.reason })));
  }
}

function openReview(community) {
  state.reviewingId = community.id;
  $(".dialog-head h2", $("#review-dialog")).textContent = `Review · ${community.name}`;

  $("#rf-status").value = community.review_status;
  $("#rf-policy").value = community.promotion_policy;
  $("#rf-channel").value = community.channel_name || "";
  $("#rf-evidence").value = community.policy_evidence || "";
  $("#rf-notes").value = community.user_notes || "";
  $("#rf-contacted").checked = Boolean(community.contacted_at);

  renderReviewSummary(community);
  renderReviewBreakdown(community);
  $("#review-dialog").showModal();
}

async function saveReview(event) {
  event.preventDefault();
  const id = state.reviewingId;
  if (!id) return;

  const payload = {
    review_status: $("#rf-status").value,
    promotion_policy: $("#rf-policy").value,
    policy_evidence: $("#rf-evidence").value || null,
    channel_name: $("#rf-channel").value || null,
    user_notes: $("#rf-notes").value || null,
    contacted: $("#rf-contacted").checked,
  };

  try {
    const updated = await api(`/communities/${id}/review`, { method: "PATCH", body: payload });
    toast("Review saved.", "ok");
    openReview(updated); // refresh the panel in place with the new score
    loadCommunities();
    loadOverview();
    refreshSyncBadge();
  } catch (error) {
    toast(`Could not save the review: ${error.message}`, "error");
  }
}

async function deleteReviewed() {
  const id = state.reviewingId;
  if (!id) return;
  const name = $(".dialog-head h2", $("#review-dialog")).textContent.replace("Review · ", "");
  if (!window.confirm(`Delete “${name}”? This removes the community and its saved source links. This cannot be undone.`)) {
    return;
  }
  try {
    await api(`/communities/${id}?confirm=true`, { method: "DELETE" });
    $("#review-dialog").close();
    toast("Community deleted.", "ok");
    loadMeta();
    loadCommunities();
    loadOverview();
    refreshSyncBadge();
  } catch (error) {
    toast(`Could not delete: ${error.message}`, "error");
  }
}

/* ------------------------------------------------------------- import dialog */

function openImport() {
  $("#import-text").value = "";
  $("#import-file").value = "";
  $("#import-result").replaceChildren();
  $("#import-commit").disabled = true;
  state.lastCsv = "";
  $("#import-dialog").showModal();
}

async function previewImport() {
  const csv = $("#import-text").value;
  const result = $("#import-result");
  clear(result);

  if (!csv.trim()) {
    result.append(banner("Choose a file or paste CSV text first.", "warn"));
    return;
  }
  state.lastCsv = csv;
  loading(result, "Validating rows…");

  try {
    const preview = await api("/csv/preview", { method: "POST", body: { csv } });
    clear(result);

    result.append(el("div", { class: "row", style: "margin-bottom:.6rem" },
      pill(`${preview.new_rows} new`, "ok"),
      pill(`${preview.merge_rows} merge`, "info"),
      pill(`${preview.invalid_rows} invalid`, preview.invalid_rows ? "danger" : "neutral"),
      el("span", { class: "small muted", text: `of ${preview.total_rows} rows` })));

    const rows = preview.rows.map((row) => el("tr", {},
      el("td", { text: String(row.line) }),
      el("td", {}, el("span", { class: "cell-name", text: row.name })),
      el("td", {}, pill(row.status, row.status === "new" ? "ok" : row.status === "merge" ? "info" : "danger")),
      el("td", {}, el("span", { class: "small", text:
        row.status === "invalid"
          ? row.errors.join(" ")
          : row.status === "merge"
            ? `Merges into ${row.merges_into}`
            : "Will be created" }))));

    result.append(el("div", { class: "table-wrap", style: "max-height:320px" },
      el("table", {},
        el("thead", {}, el("tr", {},
          el("th", { text: "Line" }), el("th", { text: "Name" }),
          el("th", { text: "Status" }), el("th", { text: "Detail" }))),
        el("tbody", {}, rows))));

    const savable = preview.new_rows + preview.merge_rows;
    $("#import-commit").disabled = savable === 0;
    if (savable === 0) {
      result.append(banner("No row can be imported. Fix the errors above and preview again.", "warn"));
    }
  } catch (error) {
    clear(result);
    result.append(banner(error.message, "danger", "Could not read the CSV"));
    $("#import-commit").disabled = true;
  }
}

async function commitImport() {
  const result = $("#import-result");
  const button = $("#import-commit");
  button.disabled = true;
  try {
    const data = await api("/csv/commit", { method: "POST", body: { csv: state.lastCsv } });
    clear(result);
    result.append(banner(
      `Imported ${data.imported}, merged ${data.merged}, skipped ${data.skipped_invalid} invalid row(s).`,
      "ok", "Import complete"));
    if (data.problems?.length) {
      result.append(el("ul", { class: "plain small" }, data.problems.map((p) => el("li", { text: p }))));
    }
    loadMeta();
    loadCommunities();
    loadOverview();
    refreshSyncBadge();
    toast(`Imported ${data.imported} and merged ${data.merged}.`, "ok");
  } catch (error) {
    clear(result);
    result.append(banner(`Import failed: ${error.message}`, "danger"));
  }
}

/* -------------------------------------------------------------------- drafts */

async function loadDraftCommunities() {
  const select = $("#draft-community");
  const hint = $("#draft-community-hint");
  loading(select);

  try {
    const page = await api("/communities?outreach_only=true&per_page=100&sort=name");
    clear(select);

    if (!page.items.length) {
      select.append(el("option", { value: "", text: "No eligible communities" }));
      select.disabled = true;
      $("#draft-generate").disabled = true;
      hint.textContent = page.total === 0
        ? "Save a community first. Communities whose rules prohibit promotion never appear here."
        : "All saved communities are currently flagged as prohibited.";
      return;
    }

    select.disabled = false;
    $("#draft-generate").disabled = false;
    select.append(el("option", { value: "", text: "Choose a community…" }));
    for (const c of page.items) {
      const label = `${c.name} — ${STATUS_LABEL[c.review_status] || c.review_status}`;
      select.append(el("option", { value: c.id, text: label }));
    }
    hint.textContent = "Only communities whose rules are not prohibited are listed.";
  } catch (error) {
    clear(select);
    select.append(el("option", { value: "", text: "Could not load communities" }));
    toast(error.message, "error");
  }
}

function renderDraftWarnings(response) {
  const node = $("#draft-warnings");
  clear(node);
  const { draft } = response;

  if (!draft.can_post) {
    node.append(banner(
      "This community's rules prohibit promotion. Do not post this draft here. There is no override.",
      "danger", "Do not post in this community"));
  }
  for (const warning of draft.warnings) {
    if (!draft.can_post && warning.toLowerCase().includes("prohibit")) continue;
    const tone = warning.toLowerCase().includes("not verified") || warning.toLowerCase().includes("re-check")
      ? "warn" : "info";
    node.append(banner(warning, tone));
  }
  if (draft.posting_target) {
    node.append(banner(`Where to post, according to your own recorded channel: ${draft.posting_target}`, "info"));
  }
}

function currentDraftText() {
  if (!state.draft) return "";
  return state.draftShowsAlternative ? state.draft.alternative.text : state.draft.draft.text;
}

function showDraft() {
  if (!state.draft) return;
  const node = $("#draft-output");
  node.hidden = false;

  const active = state.draftShowsAlternative ? state.draft.alternative : state.draft.draft;
  $("#draft-heading").textContent = state.draftShowsAlternative ? "Longer draft" : "Concise draft";
  $("#draft-text").value = active.text;
  $("#draft-swap").textContent = state.draftShowsAlternative ? "Use the concise draft" : "Use the longer draft";

  const list = $("#draft-disclosures");
  clear(list);
  for (const fact of active.disclosures) list.append(el("li", { text: fact }));

  renderDraftWarnings({ ...state.draft, draft: active });
}

async function generateDraft(event) {
  if (event) event.preventDefault();
  const communityId = $("#draft-community").value;
  if (!communityId) {
    toast("Choose a community first.", "error");
    $("#draft-community").focus();
    return;
  }

  const button = $("#draft-generate");
  button.disabled = true;
  try {
    const data = await api("/message/draft", {
      method: "POST",
      body: {
        community_id: communityId,
        tone: $("#draft-tone").value,
        variant: state.draftShowsAlternative ? "long" : "concise",
        notes: $("#draft-notes").value || null,
      },
    });
    state.draft = data;
    state.draftShowsAlternative = false;
    showDraft();
    $("#draft-regenerate").disabled = false;
    $("#draft-output").scrollIntoView({ behavior: "smooth", block: "start" });
  } catch (error) {
    toast(`Could not generate a draft: ${error.message}`, "error");
  } finally {
    button.disabled = false;
  }
}

async function copyDraft() {
  const text = $("#draft-text").value;
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    toast("Draft copied to the clipboard.", "ok");
  } catch {
    // Fallback for browsers without the async clipboard API.
    const area = $("#draft-text");
    area.select();
    try {
      document.execCommand("copy");
      toast("Draft copied.", "ok");
    } catch {
      toast("Could not copy automatically. Select the text and copy it manually.", "error");
    }
  }
}

/* --------------------------------------------------------------------- about */

async function loadHealth() {
  const node = $("#about-health");
  if (!node) return;
  try {
    const health = await api("/health");
    const ok = health.status === "ok";
    node.replaceChildren(
      pill(ok ? "Operational" : "Degraded", ok ? "ok" : "danger"),
      ` Version ${health.version} · database ${health.database}`);
  } catch (error) {
    node.replaceChildren(pill("Unreachable", "danger"), ` ${error.message}`);
  }
}

/* --------------------------------------------------------------------- sync */
//
// The sync state is advisory: the local database is always the source of truth
// for what this device shows. The pill reports what the backend last confirmed
// with Google Drive, and never says "Synced" until the backend actually did.

const SYNC_STATUS = {
  disconnected: { label: "Not signed in", tone: "neutral" },
  synced: { label: "Synced", tone: "ok" },
  syncing: { label: "Syncing…", tone: "info" },
  offline: { label: "Offline", tone: "warn" },
  pending: { label: "Pending changes", tone: "warn" },
  conflict: { label: "Conflict", tone: "danger" },
  auth_expired: { label: "Sign-in expired", tone: "danger" },
  failed: { label: "Sync failed", tone: "danger" },
  migration_required: { label: "Choose what to sync", tone: "warn" },
};

const FIELD_LABELS = {
  name: "Name",
  description: "Description",
  invite_url: "Invite URL",
  invite_code: "Invite code",
  canonical_url: "Listing URL",
  primary_source: "Source",
  tags: "Tags",
  member_count: "Member count",
  language: "Language",
  review_status: "Review status",
  promotion_policy: "Promotion policy",
  policy_evidence: "Rule evidence",
  policy_checked_at: "Rules checked at",
  channel_name: "Channel",
  user_notes: "Notes",
  contacted_at: "Contacted at",
  last_checked_at: "Last checked at",
  created_at: "Created",
  updated_at: "Updated",
  sources: "Sources",
  __deleted__: "Deletion",
};

const syncState = {
  auth: null,
  conflicts: [],
  busy: false,
  timer: null,
};

function syncLabel(snapshot) {
  if (snapshot?.conflicts > 0) {
    return `${snapshot.conflicts} conflict${snapshot.conflicts === 1 ? "" : "s"}`;
  }
  const meta = SYNC_STATUS[snapshot?.status] || SYNC_STATUS.disconnected;
  return meta.label;
}

function syncTone(snapshot) {
  if (snapshot?.conflicts > 0) return "danger";
  return (SYNC_STATUS[snapshot?.status] || SYNC_STATUS.disconnected).tone;
}

function fieldLabel(field) {
  if (field.startsWith("settings.")) return `Setting: ${field.slice("settings.".length)}`;
  return FIELD_LABELS[field] || field;
}

function formatFieldValue(field, raw) {
  if (raw === null || raw === undefined) return "nothing";
  if (raw === "") return "empty";
  if (field === "review_status") return STATUS_LABEL[raw] || raw;
  if (field === "promotion_policy") return POLICY_LABEL[raw] || raw;
  if (field === "__deleted__") return raw === "deleted" ? "deleted" : "still present";
  if (field === "tags" || field === "sources") {
    try {
      const value = JSON.parse(raw);
      if (!Array.isArray(value)) return raw;
      if (!value.length) return "none";
      return value
        .map((item) => (typeof item === "string" ? item : item?.source_name || JSON.stringify(item)))
        .join(", ");
    } catch {
      return raw;
    }
  }
  return raw;
}

async function loadSyncState() {
  try {
    const auth = await api("/auth/status");
    syncState.auth = auth;
  } catch {
    // A transient failure must not break the rest of the page. Keep the last
    // known state so the pill does not flap.
  }
  renderSyncPanel();
  renderAboutSync();
  return syncState.auth;
}

function renderSyncPanel() {
  const panel = $("#sync-panel");
  if (!panel) return;
  clear(panel);

  const auth = syncState.auth;
  const google = auth?.google;
  const snapshot = auth?.sync;
  const configured = Boolean(google?.configured);
  const signedIn = Boolean(google?.signed_in);

  let label = "Checking sync…";
  let tone = "neutral";
  if (auth?.google) {
    if (!configured) {
      label = "Sync off";
    } else {
      label = syncLabel(snapshot);
      tone = syncTone(snapshot);
    }
  }

  const status = el("button", {
    class: `sync-status sync-tone-${tone}`,
    type: "button",
    onclick: openSyncDialog,
    title: "Show sync details",
  },
    el("span", { class: "sync-dot", "aria-hidden": "true" }),
    el("span", { class: "sync-label", text: label }));

  panel.append(status);

  if (auth && !configured) {
    panel.append(el("p", { class: "sync-hint", text: "Google sync is not configured on this server. Everything works locally." }));
    return;
  }
  if (!auth) return;

  const actions = el("div", { class: "sync-actions" });
  if (!signedIn) {
    actions.append(el("button", {
      class: "btn btn-sm",
      type: "button",
      text: "Sign in with Google",
      onclick: signIn,
    }));
  } else {
    const syncButton = el("button", {
      class: "btn btn-sm",
      type: "button",
      text: syncState.busy || snapshot?.status === "syncing" ? "Syncing…" : "Sync now",
      disabled: syncState.busy || snapshot?.status === "syncing",
      onclick: syncNow,
    });
    actions.append(syncButton);
    actions.append(el("button", {
      class: "btn btn-sm btn-ghost",
      type: "button",
      text: "Sign out",
      onclick: signOut,
    }));
  }
  panel.append(actions);

  let hint = "";
  if (!signedIn) hint = "Your records sync to your own Google Drive app data folder.";
  else if (snapshot?.status === "conflict" || snapshot?.conflicts > 0) hint = "Two devices changed the same field. Open to choose a value.";
  else if (snapshot?.status === "migration_required") hint = "Both this device and Drive have records. Choose how to combine them.";
  else if (snapshot?.status === "auth_expired") hint = "Sign in again. Nothing was lost.";
  else if (snapshot?.status === "offline") hint = "Drive is unreachable. Changes are saved here and will sync later.";
  else if (snapshot?.status === "failed") hint = snapshot.last_error || "The last sync did not finish.";
  else if (snapshot?.dirty) hint = "Local changes are waiting to be sent.";
  if (hint) panel.append(el("p", { class: "sync-hint", text: hint }));
}

function renderAboutSync() {
  const node = $("#about-sync");
  if (!node) return;
  clear(node);

  const auth = syncState.auth;
  if (!auth?.google || !auth?.sync) {
    node.append(el("p", { class: "muted small", text: "Checking sync status…" }));
    return;
  }
  const google = auth.google;
  const snapshot = auth.sync;

  if (!google.configured) {
    node.append(
      el("p", { class: "small", text: "Google Drive sync is turned off because this server has no Google client configured. All features work with the local database only." }),
      el("p", { class: "muted small", text: "To enable it, set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET and restart. See docs/google-drive-setup.md." }),
    );
    return;
  }

  node.append(pill(syncLabel(snapshot), syncTone(snapshot)));

  const facts = el("dl", { class: "facts mt" },
    el("dt", { text: "Account" }), el("dd", { text: google.signed_in ? "Connected" : "Not connected" }),
    el("dt", { text: "Token storage" }), el("dd", { text: google.token_storage }),
    el("dt", { text: "Last confirmed sync" }), el("dd", { text: formatDate(snapshot.last_synced_at) }),
    el("dt", { text: "Local records" }), el("dd", { text: String(snapshot.records) }),
    el("dt", { text: "Pending changes" }), el("dd", { text: snapshot.dirty ? "Yes" : "No" }));
  node.append(facts);

  if (snapshot.last_error) {
    node.append(el("div", { class: "mt" }, banner(snapshot.last_error, snapshot.status === "failed" || snapshot.status === "auth_expired" ? "danger" : "warn")));
  }

  const row = el("div", { class: "row mt" });
  if (google.signed_in) {
    row.append(el("button", {
      class: "btn btn-sm",
      type: "button",
      text: syncState.busy ? "Syncing…" : "Sync now",
      disabled: syncState.busy,
      onclick: syncNow,
    }));
  } else {
    row.append(el("button", {
      class: "btn btn-sm btn-primary",
      type: "button",
      text: "Sign in with Google",
      onclick: signIn,
    }));
  }
  row.append(el("button", { class: "btn btn-sm", type: "button", text: "Sync details, conflicts and backups", onclick: openSyncDialog }));
  node.append(row);

  node.append(el("p", { class: "muted small mt mb0", text:
    "Google Drive stores only this app's own hidden file (drive.appdata). Scout cannot read your other Drive files, and your Discord data is never sent anywhere else. Signing out removes the credential on this server; it does not delete your records." }));
}

async function signIn() {
  // A full navigation is required: Google must set its own cookies.
  window.location.assign("/api/auth/google/start");
}

async function signOut() {
  if (!window.confirm("Sign out of Google Drive on this server? Local records stay, and syncing stops until you sign in again.")) {
    return;
  }
  try {
    const auth = await api("/auth/logout", { method: "POST" });
    syncState.auth = auth;
    renderSyncPanel();
    renderAboutSync();
    toast("Signed out of Google Drive.", "ok");
  } catch (error) {
    toast(`Could not sign out: ${error.message}`, "error");
  }
}

async function syncNow() {
  if (syncState.busy) return;
  syncState.busy = true;
  renderSyncPanel();
  renderAboutSync();
  try {
    const report = await api("/sync/run", { method: "POST" });
    syncState.busy = false;
    await loadSyncState();
    if (report.conflicts) {
      await openSyncDialog();
      toast(report.message, "error");
    } else if (report.status === "synced" || report.status === "pending") {
      toast(report.message, "ok");
      activateView(state.view);
    } else if (report.status === "syncing") {
      // Another sync was already running; the pill will update on the next poll.
    } else {
      toast(report.message, "error");
    }
  } catch (error) {
    syncState.busy = false;
    renderSyncPanel();
    renderAboutSync();
    toast(`Sync failed: ${error.message}`, "error");
  }
}

/* ------------------------------------------------------- sync / backup dialog */

async function openSyncDialog() {
  const dialog = $("#sync-dialog");
  if (!dialog) return;
  const body = $("#sync-dialog-body");
  const foot = $("#sync-dialog-foot");
  clear(body);
  clear(foot);
  body.append(el("p", { class: "muted small" }, el("span", { class: "spinner", "aria-hidden": "true" }), " Loading sync details…"));
  foot.append(el("button", { class: "btn", type: "button", text: "Close", onclick: () => dialog.close() }));
  if (!dialog.open) dialog.showModal();

  await loadSyncState();
  let conflicts = [];
  try {
    conflicts = await api("/sync/conflicts");
  } catch {
    conflicts = [];
  }
  syncState.conflicts = conflicts;

  clear(body);
  renderSyncSummary(body, conflicts);
  if (conflicts.length) renderConflictList(body, conflicts);
  await renderBackupSection(body);
}

function renderSyncSummary(body, conflicts) {
  const snapshot = syncState.auth?.sync;
  const google = syncState.auth?.google;

  body.append(el("div", { class: "row-between" },
    el("div", { class: "row" },
      pill(syncLabel(snapshot), syncTone(snapshot)),
      el("span", { class: "muted small", text: snapshot?.last_synced_at ? `Last confirmed ${formatDate(snapshot.last_synced_at)}` : "Not synced yet" })),
    google?.signed_in
      ? el("button", { class: "btn btn-sm", type: "button", text: syncState.busy ? "Syncing…" : "Sync now", disabled: syncState.busy, onclick: syncNow })
      : null));

  if (snapshot?.last_error) {
    body.append(el("div", { class: "mt" }, banner(snapshot.last_error, snapshot.status === "failed" || snapshot.status === "auth_expired" ? "danger" : "warn")));
  }

  if (snapshot?.status === "migration_required") {
    renderMigrationSection(body, snapshot);
  } else if (conflicts.length) {
    body.append(el("h3", { class: "mt", text: "Resolve conflicts" }));
    body.append(el("p", { class: "small muted", text: "Both values are kept until you choose. Nothing is uploaded while a conflict is open." }));
  }
}

function renderMigrationSection(body, snapshot) {
  const section = el("div", { class: "migration-section mt" },
    el("h3", { text: "Combine this device with Google Drive" }),
    el("p", { class: "small", text: `This device has ${snapshot.records} saved record(s). Drive may already have records too. Choose how to combine them. Nothing is deleted without a confirmation.` }));

  section.append(el("div", { class: "row mt" },
    el("button", {
      class: "btn btn-primary",
      type: "button",
      text: "Merge both (recommended)",
      onclick: () => runMigration("merge"),
    }),
    el("button", {
      class: "btn",
      type: "button",
      text: "Use the cloud copy",
      onclick: () => runMigration("use_cloud"),
    }),
    el("button", {
      class: "btn",
      type: "button",
      text: "Upload this device's data",
      onclick: () => runMigration("upload_local"),
    })));

  section.append(el("p", { class: "muted small", text:
    "Merge keeps records from both sides and asks you about any field two devices changed. \"Use the cloud copy\" and \"Upload this device's data\" replace one side, so they ask for confirmation and keep a snapshot first." }));
  body.append(section);
}

function renderConflictList(body, conflicts) {
  const list = el("div", { class: "conflict-list" });
  conflicts.forEach((conflict, index) => {
    const localRadio = el("input", { type: "radio", name: `conflict-${index}`, value: "local" });
    const remoteRadio = el("input", { type: "radio", name: `conflict-${index}`, value: "remote" });
    localRadio.checked = true;

    const isDeletion = conflict.field === "__deleted__";
    const localText = isDeletion
      ? (conflict.local_value === "deleted" ? "This device deleted the record" : "This device still has the record")
      : formatFieldValue(conflict.field, conflict.local_value);
    const remoteText = isDeletion
      ? (conflict.remote_value === "deleted" ? "The other device deleted the record" : "The other device still has the record")
      : formatFieldValue(conflict.field, conflict.remote_value);

    const item = el("div", { class: "conflict-item" },
      el("div", { class: "conflict-head" },
        el("strong", { text: conflict.community_name || "Record" }),
        el("span", { class: "muted small", text: fieldLabel(conflict.field) })),
      el("label", { class: "conflict-choice" },
        localRadio,
        el("span", { class: "conflict-tag", text: "This device" }),
        el("span", { class: "conflict-value", text: localText })),
      el("label", { class: "conflict-choice" },
        remoteRadio,
        el("span", { class: "conflict-tag", text: "Other device" }),
        el("span", { class: "conflict-value", text: remoteText })));

    if (conflict.base_value !== null && conflict.base_value !== undefined && !isDeletion) {
      item.append(el("p", { class: "muted small mb0", text: `Last synced value: ${formatFieldValue(conflict.field, conflict.base_value)}` }));
    }

    item.append(el("div", { class: "row mt" },
      el("button", {
        class: "btn btn-sm btn-primary",
        type: "button",
        text: "Keep selected value",
        onclick: (event) => applyConflict(conflict.id, remoteRadio.checked ? "remote" : "local", event.currentTarget),
      })));

    list.append(item);
  });
  body.append(list);
}

async function applyConflict(id, choice, button) {
  if (button) button.disabled = true;
  try {
    const report = await api(`/sync/conflicts/${encodeURIComponent(id)}/resolve`, {
      method: "POST",
      body: { choice },
    });
    await loadSyncState();
    activateView(state.view);
    if (report.conflicts) {
      toast(`${report.conflicts} conflict(s) still open.`, "warn");
    } else {
      toast(report.status === "synced" ? "Conflict resolved and synced." : report.message, report.status === "synced" ? "ok" : "warn");
    }
    await openSyncDialog();
  } catch (error) {
    if (button) button.disabled = false;
    toast(`Could not resolve: ${error.message}`, "error");
  }
}

async function runMigration(choice) {
  const messages = {
    merge: "Merge records from this device and Google Drive? Nothing is overwritten; conflicting fields will ask you to choose.",
    use_cloud: "Replace this device's records with the Google Drive copy? A snapshot of the current data is saved first.",
    upload_local: "Create or replace the Drive copy with this device's records? This is refused when Drive already has data, and a snapshot is saved first.",
  };
  if (!window.confirm(messages[choice])) return;
  const body = { choice, confirm: choice !== "merge" };
  try {
    const report = await api("/sync/migrate", { method: "POST", body });
    await loadSyncState();
    activateView(state.view);
    if (report.conflicts) {
      toast(report.message, "error");
    } else {
      toast(report.message, report.status === "synced" ? "ok" : "warn");
    }
    await openSyncDialog();
  } catch (error) {
    toast(`Migration failed: ${error.message}`, "error");
  }
}

async function renderBackupSection(body) {
  body.append(el("div", { class: "divider" }));
  body.append(el("h3", { text: "Backup and restore" }));
  body.append(el("p", { class: "small muted", text:
    "A copy is kept automatically before risky changes. Newer copies replace older ones, and the most recent five are listed here." }));

  body.append(el("div", { class: "row mt" },
    el("a", { class: "btn btn-sm", href: "/api/sync/export", download: "scout-data.json" }, "Download a copy")));

  const listNode = el("ul", { class: "snapshot-list" });
  body.append(listNode);

  let snapshots = [];
  try {
    snapshots = await api("/sync/snapshots");
  } catch (error) {
    listNode.append(el("li", { class: "muted small", text: `Could not list snapshots: ${error.message}` }));
    return;
  }
  if (!snapshots.length) {
    listNode.append(el("li", { class: "muted small", text: "No snapshots yet. One is created automatically before a risky change." }));
    return;
  }
  for (const snapshot of snapshots) {
    const size = snapshot.bytes < 1024 ? `${snapshot.bytes} B` : `${(snapshot.bytes / 1024).toFixed(1)} KiB`;
    listNode.append(el("li", { class: "snapshot-row" },
      el("span", { class: "snapshot-name", text: snapshot.name }),
      el("span", { class: "muted small", text: size }),
      el("button", {
        class: "btn btn-sm",
        type: "button",
        text: "Restore",
        onclick: () => restoreSnapshot(snapshot.name),
      })));
  }
}

async function restoreSnapshot(name) {
  if (!window.confirm(`Restore ${name}? This replaces the records on this device with that copy. A snapshot of the current data is saved first.`)) {
    return;
  }
  try {
    const report = await api(`/sync/snapshots/${encodeURIComponent(name)}/restore`, {
      method: "POST",
      body: { confirm: true },
    });
    await loadSyncState();
    activateView(state.view);
    toast(report.status === "synced" ? "Snapshot restored and synced." : report.message, report.status === "synced" ? "ok" : "warn");
    await openSyncDialog();
  } catch (error) {
    toast(`Restore failed: ${error.message}`, "error");
  }
}

function startSyncPolling() {
  if (syncState.timer) window.clearInterval(syncState.timer);
  syncState.timer = window.setInterval(() => {
    if (document.visibilityState === "visible") loadSyncState();
  }, 20000);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") loadSyncState();
  });
}

/** Handles the ?google=… result of the OAuth redirect without leaving it in the URL. */
function readGoogleRedirect() {
  const params = new URLSearchParams(location.search);
  const result = params.get("google");
  if (!result) return;
  if (result === "connected") toast("Connected to Google Drive.", "ok");
  else if (result === "denied") toast("Google sign-in was cancelled. Nothing changed.", "warn");
  else toast("Google sign-in did not complete. Try again.", "error");
  params.delete("google");
  const query = params.toString();
  history.replaceState(null, "", `${location.pathname}${query ? `?${query}` : ""}${location.hash}`);
}

/** Cheap badge refresh after a local write; does not trigger a network sync. */
function refreshSyncBadge() {
  loadSyncState();
}

/* --------------------------------------------------------------- wiring / init */

function wireEvents() {
  document.querySelectorAll(".nav-link").forEach((link) => {
    link.addEventListener("click", () => setView(link.dataset.view));
  });
  document.querySelectorAll("[data-goto]").forEach((button) => {
    button.addEventListener("click", () => setView(button.dataset.goto));
  });
  document.querySelectorAll("[data-close]").forEach((button) => {
    button.addEventListener("click", () => document.getElementById(button.dataset.close)?.close());
  });

  $("#refresh-dashboard").addEventListener("click", loadOverview);
  $("#refresh-sources").addEventListener("click", loadSources);

  $("#discover-form").addEventListener("submit", runDiscovery);
  $("#filter-form").addEventListener("submit", (event) => { event.preventDefault(); readFilters(); loadCommunities(); });
  $("#filter-form").addEventListener("change", () => { readFilters(); loadCommunities(); });
  $("#filter-q").addEventListener("input", debounce(() => { readFilters(); loadCommunities(); }, 350));
  $("#reset-filters").addEventListener("click", () => {
    $("#filter-form").reset();
    readFilters();
    loadCommunities();
  });

  $("#open-add").addEventListener("click", () => openCommunityDialog());
  $("#open-import").addEventListener("click", openImport);
  $("#community-form").addEventListener("submit", saveCommunity);

  $("#review-form").addEventListener("submit", saveReview);
  $("#review-delete").addEventListener("click", deleteReviewed);
  $("#rf-policy").addEventListener("change", () => {
    // Surface the consequence immediately, before the user saves.
    const node = $("#review-warnings");
    clear(node);
    const policy = $("#rf-policy").value;
    if (policy === "prohibited") {
      node.append(banner("This community prohibits promotion. Saving keeps it unsuitable for outreach with no override.", "danger", "Do not post here"));
    } else if (policy === "unknown") {
      node.append(banner("Unknown rules are never treated as permission. Check the posted rules manually.", "warn", "Not verified"));
    } else {
      node.append(banner("Only mark allowed if you quoted the rule as evidence.", "info"));
    }
  });

  $("#import-preview").addEventListener("click", previewImport);
  $("#import-commit").addEventListener("click", commitImport);
  $("#import-file").addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file) return;
    if (file.size > 1024 * 1024) {
      $("#import-result").replaceChildren(banner("That file is larger than 1 MiB. Split it into smaller files.", "danger"));
      return;
    }
    const text = await file.text();
    $("#import-text").value = text;
    previewImport();
  });

  $("#draft-form").addEventListener("submit", generateDraft);
  $("#draft-regenerate").addEventListener("click", () => generateDraft(null));
  $("#draft-copy").addEventListener("click", copyDraft);
  $("#draft-swap").addEventListener("click", () => {
    state.draftShowsAlternative = !state.draftShowsAlternative;
    showDraft();
  });
  $("#draft-community").addEventListener("change", () => {
    state.draft = null;
    $("#draft-output").hidden = true;
    $("#draft-regenerate").disabled = true;
  });

  window.addEventListener("hashchange", () => {
    const view = location.hash.replace("#", "");
    if (view !== state.view) setView(view, { updateHash: false });
  });
}

function debounce(fn, delay) {
  let timer;
  return (...args) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), delay);
  };
}

function init() {
  wireEvents();
  const initial = VIEWS.includes(location.hash.replace("#", "")) ? location.hash.replace("#", "") : "overview";
  setView(initial);
  readGoogleRedirect();
  loadOverview();
  loadSyncState();
  startSyncPolling();
}

init();
