ALTER TABLE alert_rules DROP CONSTRAINT alert_rules_channel_check;
ALTER TABLE alert_rules ADD CONSTRAINT alert_rules_channel_check
    CHECK (channel IN ('email', 'telegram', 'voice_call'));
