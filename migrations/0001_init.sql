-- Initial schema.

CREATE TABLE communities (
    id                TEXT PRIMARY KEY NOT NULL,
    name              TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    invite_url        TEXT,
    invite_code       TEXT,
    canonical_url     TEXT,
    primary_source    TEXT NOT NULL DEFAULT 'Manual entry',
    tags              TEXT NOT NULL DEFAULT '[]',
    member_count      INTEGER,
    language          TEXT,
    relevance_score   INTEGER NOT NULL DEFAULT 0,
    score_breakdown   TEXT NOT NULL DEFAULT '[]',
    review_status     TEXT NOT NULL DEFAULT 'not_reviewed'
        CHECK (review_status IN ('not_reviewed', 'suitable', 'unsuitable', 'needs_manual_check', 'contacted')),
    promotion_policy  TEXT NOT NULL DEFAULT 'unknown'
        CHECK (promotion_policy IN ('allowed', 'prohibited', 'unknown')),
    policy_evidence   TEXT,
    policy_checked_at TEXT,
    channel_name      TEXT,
    user_notes        TEXT NOT NULL DEFAULT '',
    contacted_at      TEXT,
    last_checked_at   TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE UNIQUE INDEX ux_communities_invite_code
    ON communities (invite_code) WHERE invite_code IS NOT NULL;
CREATE INDEX ix_communities_review_status ON communities (review_status);
CREATE INDEX ix_communities_relevance ON communities (relevance_score DESC);
CREATE INDEX ix_communities_created_at ON communities (created_at DESC);

-- One row per place a community was seen. Never overwrite; append.
CREATE TABLE community_sources (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    community_id       TEXT NOT NULL REFERENCES communities (id) ON DELETE CASCADE,
    source_name        TEXT NOT NULL,
    source_external_id TEXT,
    listing_url        TEXT,
    first_seen_at      TEXT NOT NULL,
    last_seen_at       TEXT NOT NULL
);

CREATE UNIQUE INDEX ux_community_sources_ref
    ON community_sources (community_id, source_name, ifnull(source_external_id, ''));
CREATE INDEX ix_community_sources_source ON community_sources (source_name);

CREATE TABLE discovery_runs (
    id              TEXT PRIMARY KEY NOT NULL,
    query           TEXT NOT NULL,
    sources         TEXT NOT NULL DEFAULT '[]',
    requested_limit INTEGER NOT NULL,
    status          TEXT NOT NULL
        CHECK (status IN ('running', 'succeeded', 'partial', 'failed')),
    result_count    INTEGER NOT NULL DEFAULT 0,
    saved_count     INTEGER NOT NULL DEFAULT 0,
    partial         INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    started_at      TEXT NOT NULL,
    finished_at     TEXT
);

CREATE INDEX ix_discovery_runs_started ON discovery_runs (started_at DESC);

-- Non-secret preferences only.
CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
