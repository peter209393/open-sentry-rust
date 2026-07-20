ALTER TABLE projects
    ADD COLUMN archived_at timestamptz,
    ADD COLUMN retention_days integer CHECK (retention_days IS NULL OR retention_days > 0),
    ADD COLUMN scrub_fields jsonb NOT NULL DEFAULT '["password","passwd","authorization","cookie","set-cookie","token","secret","api_key","apikey"]'::jsonb;

ALTER TABLE project_keys
    ADD COLUMN last_used_at timestamptz;

ALTER TABLE users
    ADD COLUMN active boolean NOT NULL DEFAULT true,
    ADD COLUMN last_login_at timestamptz;

ALTER TABLE issues
    ADD COLUMN assigned_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN regressed_at timestamptz;

CREATE TABLE issue_comments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    issue_id uuid NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author_user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    body text NOT NULL CHECK (length(body) BETWEEN 1 AND 5000),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX issue_comments_issue_created_idx ON issue_comments(issue_id, created_at);

CREATE INDEX projects_organization_active_idx
    ON projects(organization_id, created_at DESC) WHERE archived_at IS NULL;
