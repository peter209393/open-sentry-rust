CREATE TABLE user_invitations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(), organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    email text NOT NULL, display_name text NOT NULL, role text NOT NULL CHECK(role IN ('admin','member')),
    token_hash text NOT NULL UNIQUE, invited_by uuid NOT NULL REFERENCES users(id), expires_at timestamptz NOT NULL,
    accepted_at timestamptz, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX user_invitations_pending_email ON user_invitations(organization_id,lower(email)) WHERE accepted_at IS NULL;

CREATE TABLE project_deletion_requests (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(), project_id uuid NOT NULL UNIQUE REFERENCES projects(id) ON DELETE CASCADE,
    requested_by uuid NOT NULL REFERENCES users(id), confirmation text NOT NULL, execute_after timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE alert_rules ADD COLUMN threshold_count integer CHECK(threshold_count IS NULL OR threshold_count > 0),
    ADD COLUMN window_seconds integer CHECK(window_seconds IS NULL OR window_seconds >= 60),
    ADD COLUMN notify_recovery boolean NOT NULL DEFAULT false;
ALTER TABLE notification_outbox ADD COLUMN dedup_key text;
CREATE UNIQUE INDEX notification_outbox_dedup_key ON notification_outbox(dedup_key) WHERE dedup_key IS NOT NULL;
