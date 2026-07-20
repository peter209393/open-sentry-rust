CREATE TABLE organizations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    slug text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO organizations (id, name, slug)
VALUES ('00000000-0000-0000-0000-000000000001', 'Demo Organization', 'demo');

ALTER TABLE projects
    ADD COLUMN organization_id uuid REFERENCES organizations(id) ON DELETE CASCADE,
    ADD COLUMN external_id bigint;

UPDATE projects
SET organization_id = '00000000-0000-0000-0000-000000000001', external_id = 1
WHERE id = '00000000-0000-0000-0000-000000000001';

ALTER TABLE projects
    ALTER COLUMN organization_id SET NOT NULL,
    ALTER COLUMN external_id SET NOT NULL,
    ADD CONSTRAINT projects_external_id_unique UNIQUE (external_id);

CREATE TABLE project_keys (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name text NOT NULL,
    key_hash text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);

CREATE INDEX project_keys_active_idx ON project_keys(project_id) WHERE revoked_at IS NULL;

INSERT INTO project_keys (project_id, name, key_hash)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'Development DSN',
    '298754db2dbab6ec62605ceb0379eb7ee376580359449efe0caa3aa06cd56736'
);
