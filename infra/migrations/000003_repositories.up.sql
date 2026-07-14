-- Repositories and version control.
--
-- Note what is NOT here: the commit graph. There are no parent/child edges
-- between commits. We store the commits we encounter through pushes, pull
-- requests and workflow runs, and nothing more. Every metric in this product,
-- including lead time, needs "when was this authored" and "when did it ship",
-- both of which we get without mirroring the DAG.

CREATE TABLE repositories (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    organization_id  uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    -- Stable across renames AND transfers between owners. full_name is not.
    github_repo_id   bigint      NOT NULL UNIQUE,
    owner_login      text        NOT NULL,
    name             text        NOT NULL,
    full_name        text GENERATED ALWAYS AS (owner_login || '/' || name) STORED,
    description      text,
    default_branch   text        NOT NULL DEFAULT 'main',
    is_private       boolean     NOT NULL DEFAULT false,
    is_archived      boolean     NOT NULL DEFAULT false,
    is_fork          boolean     NOT NULL DEFAULT false,
    primary_language text,
    html_url         text,
    -- The user opted this repo in. Syncing every repo a user can see would be
    -- rude to GitHub's rate limiter and to their dashboard.
    tracking_enabled boolean     NOT NULL DEFAULT false,
    github_created_at timestamptz,
    github_pushed_at  timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    -- Soft delete only. Analytics rows point here; a hard delete would either
    -- cascade away months of metrics or fail on the foreign key.
    deleted_at       timestamptz
);

CREATE TRIGGER repositories_set_updated_at
    BEFORE UPDATE ON repositories
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX repositories_org_idx       ON repositories (organization_id) WHERE deleted_at IS NULL;
CREATE INDEX repositories_full_name_idx ON repositories (full_name);
CREATE INDEX repositories_tracked_idx   ON repositories (organization_id)
    WHERE tracking_enabled AND deleted_at IS NULL;

-- Where each sync left off, per resource. Without this, every sync re-walks the
-- full GitHub history and burns the hourly rate limit on data we already have.
-- The ETag lets us re-poll for free: GitHub returns 304 and does not charge us.
CREATE TABLE repository_sync_state (
    repository_id   uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    resource        text        NOT NULL
                      CHECK (resource IN ('branches', 'commits', 'pull_requests',
                                          'workflows', 'workflow_runs', 'deployments')),
    etag            text,
    cursor          text,
    sync_status     text        NOT NULL DEFAULT 'idle'
                      CHECK (sync_status IN ('idle', 'syncing', 'error')),
    last_synced_at  timestamptz,
    last_success_at timestamptz,
    last_error      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (repository_id, resource)
);

CREATE TRIGGER repository_sync_state_set_updated_at
    BEFORE UPDATE ON repository_sync_state
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE branches (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id  uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name           text        NOT NULL,
    head_sha       text        NOT NULL,
    is_default     boolean     NOT NULL DEFAULT false,
    is_protected   boolean     NOT NULL DEFAULT false,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    deleted_at     timestamptz,

    CONSTRAINT branches_repo_name_key UNIQUE (repository_id, name)
);

CREATE TRIGGER branches_set_updated_at
    BEFORE UPDATE ON branches
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE commits (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id         uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    -- Not the primary key: the same SHA can legitimately exist in two repos
    -- (forks), and we want repository_id on the leading edge of every index.
    sha                   text        NOT NULL,
    message               text,
    author_name           text,
    author_email          citext,
    author_login          text,
    author_github_user_id bigint,
    committer_name        text,
    committer_email       citext,
    -- authored_at is when the developer wrote it; committed_at is when it
    -- landed. A rebase moves the second and not the first. Lead time measures
    -- from authored_at, so we need both.
    authored_at           timestamptz,
    committed_at          timestamptz NOT NULL,
    additions             integer,
    deletions             integer,
    changed_files         integer,
    is_merge_commit       boolean     NOT NULL DEFAULT false,
    created_at            timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT commits_repo_sha_key UNIQUE (repository_id, sha)
);

CREATE INDEX commits_repo_committed_at_idx ON commits (repository_id, committed_at DESC);

CREATE TABLE pull_requests (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id         uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    github_pr_id          bigint      NOT NULL UNIQUE,
    number                integer     NOT NULL,
    title                 text        NOT NULL,
    state                 text        NOT NULL CHECK (state IN ('open', 'closed')),
    is_draft              boolean     NOT NULL DEFAULT false,
    author_login          text,
    author_github_user_id bigint,
    head_ref              text,
    head_sha              text,
    base_ref              text,
    base_sha              text,
    merge_commit_sha      text,
    merged_by_login       text,
    additions             integer,
    deletions             integer,
    changed_files         integer,
    commits_count         integer,
    comments_count        integer     NOT NULL DEFAULT 0,
    review_comments_count integer     NOT NULL DEFAULT 0,
    opened_at             timestamptz NOT NULL,
    -- Populated only if we ingest pull_request_review events. Left nullable and
    -- unused for now; review latency is the most interesting slice of lead time
    -- and this is where it lands when we want it.
    first_review_at       timestamptz,
    closed_at             timestamptz,
    merged_at             timestamptz,
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT pull_requests_repo_number_key UNIQUE (repository_id, number)
);

CREATE TRIGGER pull_requests_set_updated_at
    BEFORE UPDATE ON pull_requests
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX pull_requests_repo_merged_idx ON pull_requests (repository_id, merged_at DESC)
    WHERE merged_at IS NOT NULL;
CREATE INDEX pull_requests_repo_state_idx  ON pull_requests (repository_id, state);
CREATE INDEX pull_requests_head_sha_idx    ON pull_requests (repository_id, head_sha);
