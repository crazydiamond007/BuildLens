-- Identity and tenancy.
--
-- The Organization is BuildLens's tenancy boundary, not GitHub's. Every user
-- gets a personal organization at signup, and every repository belongs to
-- exactly one organization. Authorization is therefore one rule everywhere:
-- "is this user a member of the org that owns this row?"
--
-- Mirroring GitHub's org membership instead would mean inheriting GitHub's
-- permission model, where read access to code implies read access to metrics.
-- Those are not the same permission.

CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    email         citext      NOT NULL UNIQUE,
    name          text,
    avatar_url    text,
    is_active     boolean     NOT NULL DEFAULT true,
    last_login_at timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz
);

CREATE TRIGGER users_set_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The GitHub OAuth identity behind a user.
--
-- Tokens are encrypted by the gateway (AES-GCM, key from the environment) and
-- stored as ciphertext. Postgres never holds a usable GitHub credential, so a
-- database dump is not a credential leak.
CREATE TABLE github_accounts (
    id                     uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id                uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- The numeric ID, not the login: logins are renameable, IDs are not.
    github_user_id         bigint      NOT NULL,
    login                  text        NOT NULL,
    avatar_url             text,
    access_token_encrypted bytea       NOT NULL,
    refresh_token_encrypted bytea,
    token_expires_at       timestamptz,
    scopes                 text[]      NOT NULL DEFAULT '{}',
    connected_at           timestamptz NOT NULL DEFAULT now(),
    created_at             timestamptz NOT NULL DEFAULT now(),
    updated_at             timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT github_accounts_github_user_id_key UNIQUE (github_user_id),
    -- One GitHub identity per user for now. Dropping this is how you'd later
    -- support linking several GitHub accounts to one BuildLens login.
    CONSTRAINT github_accounts_user_id_key UNIQUE (user_id)
);

CREATE TRIGGER github_accounts_set_updated_at
    BEFORE UPDATE ON github_accounts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE organizations (
    id                     uuid PRIMARY KEY DEFAULT uuidv7(),
    slug                   citext      NOT NULL UNIQUE,
    name                   text        NOT NULL,
    -- 'personal' orgs are created implicitly at signup and hold a user's own
    -- repositories. 'team' orgs are created explicitly and have real members.
    kind                   text        NOT NULL CHECK (kind IN ('personal', 'team')),
    -- Set when the org is linked to a GitHub organization. Null for a personal
    -- workspace tracking repos the user owns individually.
    github_org_id          bigint UNIQUE,
    github_org_login       text,
    github_installation_id bigint UNIQUE,
    created_by             uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at             timestamptz NOT NULL DEFAULT now(),
    updated_at             timestamptz NOT NULL DEFAULT now(),
    deleted_at             timestamptz
);

CREATE TRIGGER organizations_set_updated_at
    BEFORE UPDATE ON organizations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE organization_members (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    organization_id uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id         uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- We own this vocabulary, so it gets a CHECK constraint. Contrast with the
    -- GitHub-sourced status columns later on, which are deliberately free text.
    role            text        NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    invited_by      uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT organization_members_org_user_key UNIQUE (organization_id, user_id)
);

CREATE TRIGGER organization_members_set_updated_at
    BEFORE UPDATE ON organization_members
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX organization_members_user_id_idx ON organization_members (user_id);

-- Programmatic access tokens (the `blq_...` kind you paste into CI).
--
-- We store a SHA-256 hash, never the token. `token_prefix` is the first few
-- characters, kept in the clear so the UI can show "blq_a1b2…" and so lookup is
-- an index hit rather than a scan of every hash.
CREATE TABLE api_tokens (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id         uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Null means the token is scoped to the user across all their orgs.
    organization_id uuid REFERENCES organizations(id) ON DELETE CASCADE,
    name            text        NOT NULL,
    token_prefix    text        NOT NULL,
    token_hash      bytea       NOT NULL UNIQUE,
    scopes          text[]      NOT NULL DEFAULT '{}',
    last_used_at    timestamptz,
    expires_at      timestamptz,
    revoked_at      timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER api_tokens_set_updated_at
    BEFORE UPDATE ON api_tokens
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX api_tokens_prefix_idx  ON api_tokens (token_prefix);
CREATE INDEX api_tokens_user_id_idx ON api_tokens (user_id) WHERE revoked_at IS NULL;
