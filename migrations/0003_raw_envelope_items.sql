CREATE TABLE envelope_items (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    item_type text NOT NULL,
    payload bytea NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX envelope_items_project_received_idx
    ON envelope_items(project_id, received_at DESC);
