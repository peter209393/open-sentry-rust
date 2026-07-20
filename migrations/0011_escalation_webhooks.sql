ALTER TABLE alert_rules DROP CONSTRAINT alert_rules_channel_check;
ALTER TABLE alert_rules ADD CONSTRAINT alert_rules_channel_check CHECK(channel IN ('email','telegram','voice_call','webhook'));
CREATE TABLE on_call_schedules (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 name text NOT NULL, timezone text NOT NULL DEFAULT 'UTC', created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE on_call_members (
 schedule_id uuid NOT NULL REFERENCES on_call_schedules(id) ON DELETE CASCADE, user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 rotation_order integer NOT NULL, PRIMARY KEY(schedule_id,user_id), UNIQUE(schedule_id,rotation_order)
);
CREATE TABLE escalation_policies (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 name text NOT NULL, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE escalation_steps (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), policy_id uuid NOT NULL REFERENCES escalation_policies(id) ON DELETE CASCADE,
 step_order integer NOT NULL, delay_seconds integer NOT NULL DEFAULT 0 CHECK(delay_seconds>=0),
 channel text NOT NULL CHECK(channel IN ('email','telegram','voice_call','webhook')), target text,
 schedule_id uuid REFERENCES on_call_schedules(id) ON DELETE SET NULL, UNIQUE(policy_id,step_order),
 CHECK(target IS NOT NULL OR schedule_id IS NOT NULL)
);
CREATE TABLE webhook_endpoints (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 name text NOT NULL, url text NOT NULL, signing_secret text NOT NULL, enabled boolean NOT NULL DEFAULT true,
 created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE alert_rules ADD COLUMN escalation_policy_id uuid REFERENCES escalation_policies(id) ON DELETE SET NULL;
ALTER TABLE notification_outbox ADD COLUMN escalation_policy_id uuid REFERENCES escalation_policies(id) ON DELETE SET NULL,
 ADD COLUMN escalation_step integer NOT NULL DEFAULT 0;
CREATE TABLE webhook_deliveries (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), endpoint_id uuid NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
 notification_id uuid REFERENCES notification_outbox(id) ON DELETE SET NULL, status_code integer, error text,
 delivered_at timestamptz NOT NULL DEFAULT now()
);
