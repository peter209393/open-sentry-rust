CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE projects (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    slug text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE issues (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    fingerprint text NOT NULL,
    title text NOT NULL,
    level text NOT NULL,
    status text NOT NULL DEFAULT 'unresolved' CHECK (status IN ('unresolved', 'resolved', 'ignored')),
    event_count bigint NOT NULL DEFAULT 1,
    first_seen timestamptz NOT NULL,
    last_seen timestamptz NOT NULL,
    UNIQUE (project_id, fingerprint)
);
CREATE INDEX issues_project_last_seen_idx ON issues(project_id, last_seen DESC);
CREATE INDEX issues_project_status_idx ON issues(project_id, status, last_seen DESC);

CREATE TABLE events (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    issue_id uuid NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    level text NOT NULL,
    message text NOT NULL,
    environment text,
    release text,
    tags jsonb NOT NULL DEFAULT '{}',
    contexts jsonb NOT NULL DEFAULT '{}',
    exception jsonb NOT NULL DEFAULT '{}',
    received_at timestamptz NOT NULL DEFAULT now(),
    occurred_at timestamptz NOT NULL
);
CREATE INDEX events_issue_occurred_idx ON events(issue_id, occurred_at DESC);
CREATE INDEX events_project_occurred_idx ON events(project_id, occurred_at DESC);
CREATE INDEX events_tags_gin_idx ON events USING gin(tags);

CREATE TABLE alert_rules (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name text NOT NULL,
    level text,
    message_contains text,
    environment text,
    cooldown_seconds integer NOT NULL DEFAULT 300 CHECK (cooldown_seconds >= 0),
    channel text NOT NULL CHECK (channel IN ('email', 'telegram')),
    target text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    last_triggered_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX alert_rules_project_idx ON alert_rules(project_id) WHERE enabled;

CREATE TABLE notification_outbox (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    rule_id uuid NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
    channel text NOT NULL,
    target text NOT NULL,
    payload jsonb NOT NULL,
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'processing', 'sent', 'failed')),
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL DEFAULT now(),
    claimed_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz
);
CREATE INDEX notification_outbox_pending_idx ON notification_outbox(available_at, created_at) WHERE status = 'pending';

INSERT INTO projects (id, name, slug)
VALUES ('00000000-0000-0000-0000-000000000001', 'Demo Project', 'demo');
