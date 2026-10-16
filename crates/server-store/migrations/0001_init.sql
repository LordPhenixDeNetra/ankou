-- Schéma initial du serveur aNkou (spec §7.2).

CREATE TABLE plans (
    id          uuid PRIMARY KEY,
    name        text UNIQUE NOT NULL,
    max_devices int NULL                      -- NULL = illimité
);

INSERT INTO plans (id, name, max_devices)
VALUES ('01920000-0000-7000-8000-000000000001', 'famille', NULL);

CREATE TABLE users (
    id            uuid PRIMARY KEY,
    email         text UNIQUE NOT NULL,
    password_hash text NOT NULL,
    plan_id       uuid NOT NULL REFERENCES plans,
    created_at    timestamptz NOT NULL DEFAULT now(),
    disabled_at   timestamptz NULL
);

CREATE TABLE invitations (
    id         uuid PRIMARY KEY,
    code_hash  bytea UNIQUE NOT NULL,
    created_by uuid NULL REFERENCES users,
    expires_at timestamptz NOT NULL,
    used_by    uuid NULL REFERENCES users,
    used_at    timestamptz NULL
);

CREATE TABLE servers (
    id              uuid PRIMARY KEY,
    name            text UNIQUE NOT NULL,
    country         text NOT NULL,
    city            text NOT NULL,
    endpoint        text NOT NULL,
    public_key      text UNIQUE NOT NULL,
    tags            text[] NOT NULL DEFAULT '{}',
    capacity        int NOT NULL DEFAULT 0,
    active          bool NOT NULL DEFAULT true,
    node_token_hash bytea UNIQUE NOT NULL
);

CREATE TABLE devices (
    id             uuid PRIMARY KEY,
    user_id        uuid NOT NULL REFERENCES users,
    name           text NOT NULL,
    public_key     text NOT NULL,
    ipv4           inet NOT NULL,
    ipv6           inet NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    revoked_at     timestamptz NULL,
    last_handshake timestamptz NULL
);

CREATE UNIQUE INDEX devices_active_key ON devices (public_key) WHERE revoked_at IS NULL;
CREATE UNIQUE INDEX devices_active_ipv4 ON devices (ipv4) WHERE revoked_at IS NULL;

CREATE TABLE sessions (
    id           uuid PRIMARY KEY,
    refresh_hash bytea UNIQUE NOT NULL,
    user_id      uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    device_id    uuid NULL REFERENCES devices,
    expires_at   timestamptz NOT NULL
);
