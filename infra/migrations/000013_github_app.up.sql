-- GitHub App installations.
--
-- The move from a user OAuth token to a GitHub App means repository access no
-- longer rides on a person's credential. It rides on an *installation*: the user
-- installs BuildLens on an account (their own or an org's) and grants it a set
-- of repositories. GitHub gives that installation a stable numeric id, and the
-- gateway mints short-lived access tokens against it (see gateway/src/github_app.rs).
--
-- This table is the durable record of an installation. `organizations` already
-- carried a `github_installation_id` column in anticipation of this; the foreign
-- key below finally gives it something to point at, and ON DELETE SET NULL means
-- uninstalling on GitHub cleanly unlinks the workspace without deleting it.
CREATE TABLE github_installations (
    -- GitHub's installation id, not a surrogate: it is the value every
    -- /app/installations/{id}/* call is keyed by, so it is the natural key.
    installation_id      bigint PRIMARY KEY,
    -- The account the App is installed on (a user or an organization login).
    account_login        text        NOT NULL,
    account_id           bigint      NOT NULL,
    -- GitHub's own vocabulary, so it stays free text rather than a CHECK: 'User'
    -- or 'Organization' today, but not ours to constrain.
    target_type          text        NOT NULL,
    -- 'all' or 'selected' - whether the App can see every repo on the account or
    -- only the chosen ones. Informational; the token's scope is enforced by GitHub.
    repository_selection text        NOT NULL DEFAULT 'selected',
    -- Set while an installation is suspended on GitHub; tokens will not mint.
    suspended_at         timestamptz,
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER github_installations_set_updated_at
    BEFORE UPDATE ON github_installations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

ALTER TABLE organizations
    ADD CONSTRAINT organizations_github_installation_fkey
    FOREIGN KEY (github_installation_id)
    REFERENCES github_installations (installation_id)
    ON DELETE SET NULL;

-- The gateway owns every observed GitHub fact, installations included.
GRANT INSERT, UPDATE, DELETE ON github_installations TO buildlens_gateway;
