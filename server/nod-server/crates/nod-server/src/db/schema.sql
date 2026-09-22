PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    emoji TEXT NOT NULL DEFAULT '🔔',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
);

CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    native_app_id TEXT,
    token_hash TEXT NOT NULL UNIQUE,
    push_provider TEXT,
    push_token TEXT,
    signing_key_id TEXT,
    signing_key_algorithm TEXT,
    signing_public_key TEXT,
    revoked_at TEXT,
    notification_sound TEXT NOT NULL DEFAULT 'default',
    notification_preferences_json TEXT NOT NULL DEFAULT '{}',
    last_seen_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK (
        push_token IS NULL
        OR (
            native_app_id IS NOT NULL
            AND TRIM(native_app_id) != ''
        )
    )
);

CREATE INDEX IF NOT EXISTS idx_devices_user_id
ON devices(user_id);

CREATE TABLE IF NOT EXISTS device_attestations (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    status TEXT NOT NULL,
    key_id TEXT,
    team_id TEXT,
    bundle_id TEXT,
    environment TEXT,
    public_key TEXT,
    counter INTEGER,
    receipt_hash TEXT,
    verified_at TEXT,
    failure_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (device_id, provider)
);

CREATE INDEX IF NOT EXISTS idx_device_attestations_status
ON device_attestations(status, provider);

CREATE TABLE IF NOT EXISTS issuer_tokens (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    scopes_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS user_enrollment_codes (
    code_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_user_enrollment_codes_user
ON user_enrollment_codes(user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS user_channel_subscriptions (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    subscribed INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, channel_id)
);

CREATE TABLE IF NOT EXISTS user_channel_clears (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    cleared_at TEXT NOT NULL,
    PRIMARY KEY (user_id, channel_id)
);

CREATE TABLE IF NOT EXISTS requests (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES channels(id),
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    body_markdown TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    links_json TEXT NOT NULL,
    image_url TEXT,
    notification_json TEXT NOT NULL DEFAULT '{}',
    dedupe_key TEXT,
    expires_at TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    resolved_at TEXT,
    decision_json TEXT,
    callback_url TEXT,
    decision_resolution TEXT NOT NULL DEFAULT 'shared',
    recipient_salt TEXT NOT NULL DEFAULT '',
    explicit_recipients INTEGER NOT NULL DEFAULT 0,
    created_by_issuer_token_id TEXT REFERENCES issuer_tokens(id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_requests_pending_dedupe
ON requests(channel_id, dedupe_key)
WHERE dedupe_key IS NOT NULL AND status = 'pending';

CREATE INDEX IF NOT EXISTS idx_requests_channel_created
ON requests(channel_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_requests_status_expires
ON requests(status, expires_at);

CREATE INDEX IF NOT EXISTS idx_requests_created_by_issuer_token
ON requests(created_by_issuer_token_id);

CREATE TABLE IF NOT EXISTS request_options (
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    option_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    style TEXT NOT NULL,
    requires_text INTEGER NOT NULL DEFAULT 0,
    text_placeholder TEXT,
    destructive INTEGER NOT NULL DEFAULT 0,
    foreground INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (request_id, option_id)
);

CREATE TABLE IF NOT EXISTS request_recipients (
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (request_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_request_recipients_user
ON request_recipients(user_id, request_id);

CREATE TABLE IF NOT EXISTS request_user_decisions (
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    decision_json TEXT NOT NULL,
    resolved_at TEXT NOT NULL,
    PRIMARY KEY (request_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_request_user_decisions_user
ON request_user_decisions(user_id, resolved_at DESC);

CREATE TABLE IF NOT EXISTS decision_nonces (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    key_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    used_at TEXT NOT NULL,
    PRIMARY KEY (device_id, key_id, nonce)
);

CREATE TABLE IF NOT EXISTS push_deliveries (
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    error TEXT,
    PRIMARY KEY(request_id,device_id)
);
CREATE INDEX IF NOT EXISTS idx_push_deliveries_status_updated
ON push_deliveries(status,updated_at);

CREATE TABLE IF NOT EXISTS request_idempotency (
    scope TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    PRIMARY KEY(scope,channel_id,key)
);
