CREATE TABLE releases (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(), project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version text NOT NULL, description text, created_at timestamptz NOT NULL DEFAULT now(),
    first_event_at timestamptz, last_event_at timestamptz, event_count bigint NOT NULL DEFAULT 0,
    UNIQUE(project_id, version)
);
CREATE TABLE deployments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(), release_id uuid NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    environment text NOT NULL, name text, url text, deployed_by uuid REFERENCES users(id), deployed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX deployments_release_time_idx ON deployments(release_id, deployed_at DESC);
CREATE TABLE debug_files (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(), project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    release_id uuid REFERENCES releases(id) ON DELETE CASCADE, kind text NOT NULL CHECK(kind IN ('source_map','native_symbol')),
    name text NOT NULL, debug_id text, checksum text NOT NULL, payload bytea NOT NULL,
    status text NOT NULL DEFAULT 'ready' CHECK(status IN ('ready','invalid','processing')),
    error text, created_at timestamptz NOT NULL DEFAULT now(), UNIQUE(project_id, checksum)
);
CREATE INDEX debug_files_lookup_idx ON debug_files(project_id, release_id, name);
ALTER TABLE events ADD COLUMN symbolicated_exception jsonb, ADD COLUMN symbolication_status text NOT NULL DEFAULT 'pending';
CREATE TABLE symbolication_jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(), event_id uuid NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','complete','failed')),
    attempts integer NOT NULL DEFAULT 0, last_error text, available_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(), completed_at timestamptz, UNIQUE(event_id)
);
