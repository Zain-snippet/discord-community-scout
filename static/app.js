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
  if (view === "about") { loadSources(); loadHealth(); }
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
  loadOverview();
}

init();
