CREATE TABLE services (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name text NOT NULL,
    environment text NOT NULL DEFAULT 'default',
    latest_release text,
    sdk_name text,
    sdk_version text,
    runtime jsonb NOT NULL DEFAULT '{}',
    event_count bigint NOT NULL DEFAULT 0,
    log_count bigint NOT NULL DEFAULT 0,
    first_seen timestamptz NOT NULL,
    last_seen timestamptz NOT NULL,
    UNIQUE (project_id, name, environment)
);
CREATE INDEX services_project_last_seen_idx ON services(project_id, last_seen DESC);

ALTER TABLE events
    ADD COLUMN service_id uuid REFERENCES services(id) ON DELETE SET NULL,
    ADD COLUMN trace_id text;
CREATE INDEX events_service_occurred_idx ON events(service_id, occurred_at DESC);
CREATE INDEX events_trace_id_idx ON events(trace_id) WHERE trace_id IS NOT NULL;

CREATE TABLE logs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source_item_id uuid NOT NULL REFERENCES envelope_items(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    service_id uuid REFERENCES services(id) ON DELETE SET NULL,
    occurred_at timestamptz NOT NULL,
    level text NOT NULL,
    body text NOT NULL,
    trace_id text,
    attributes jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX logs_project_occurred_idx ON logs(project_id, occurred_at DESC);
CREATE INDEX logs_service_occurred_idx ON logs(service_id, occurred_at DESC);
CREATE INDEX logs_trace_id_idx ON logs(trace_id) WHERE trace_id IS NOT NULL;

ALTER TABLE issues DROP CONSTRAINT issues_status_check;
ALTER TABLE issues ADD CONSTRAINT issues_status_check
    CHECK (status IN ('unresolved', 'in_progress', 'resolved', 'ignored'));
ALTER TABLE issues
    ADD COLUMN assigned_to text,
    ADD COLUMN fix_context jsonb,
    ADD COLUMN fixed_at timestamptz;
