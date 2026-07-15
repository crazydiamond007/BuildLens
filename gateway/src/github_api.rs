use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use reqwest::{StatusCode, Url, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

const API_VERSION: &str = "2022-11-28";

pub async fn access_token(state: &AppState, user_id: Uuid) -> Result<String, AppError> {
    let encrypted = sqlx::query(
        "SELECT access_token_encrypted, token_expires_at
         FROM github_accounts WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::bad_request("the user has no connected GitHub account"))?;

    let expires_at: Option<DateTime<Utc>> = encrypted.try_get("token_expires_at")?;
    if expires_at.is_some_and(|expires_at| expires_at <= Utc::now()) {
        return Err(AppError::bad_request(
            "the connected GitHub credential has expired; sign in with GitHub again",
        ));
    }
    let ciphertext: Vec<u8> = encrypted.try_get("access_token_encrypted")?;
    state
        .token_cipher
        .decrypt(&ciphertext)
        .map_err(AppError::from)
}

/// Finds a usable GitHub token to act on a repository outside a user request —
/// specifically, the webhook-triggered log capture, which has no session. Picks
/// any organization member with a connected, unexpired GitHub account. This is a
/// deliberate simplification for a portfolio project: a production system would
/// use a GitHub App installation token scoped to the repository rather than
/// borrowing a member's credential. Returns `None` when no member has a valid
/// token, which the caller treats as "skip logs".
pub async fn repository_token(
    state: &AppState,
    repository_id: Uuid,
) -> Result<Option<String>, AppError> {
    let row = sqlx::query(
        "SELECT ga.access_token_encrypted
         FROM github_accounts ga
         JOIN organization_members m ON m.user_id = ga.user_id
         JOIN repositories r ON r.organization_id = m.organization_id
         JOIN users u ON u.id = ga.user_id
         WHERE r.id = $1
           AND u.is_active AND u.deleted_at IS NULL
           AND (ga.token_expires_at IS NULL OR ga.token_expires_at > now())
         ORDER BY CASE m.role
                    WHEN 'owner' THEN 0 WHEN 'admin' THEN 1
                    WHEN 'member' THEN 2 ELSE 3 END
         LIMIT 1",
    )
    .bind(repository_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let ciphertext: Vec<u8> = row.try_get("access_token_encrypted")?;
    Ok(Some(state.token_cipher.decrypt(&ciphertext)?))
}

pub struct GitHubApi<'a> {
    state: &'a AppState,
    token: &'a str,
}

impl<'a> GitHubApi<'a> {
    pub fn new(state: &'a AppState, token: &'a str) -> Self {
        Self { state, token }
    }

    pub fn endpoint(&self, segments: &[&str]) -> Result<Url, AppError> {
        let mut url = self.state.config.github_api_base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| AppError::internal("GitHub API base URL cannot be a base"))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }

    pub async fn repositories(&self) -> Result<Vec<GitHubRepository>, AppError> {
        let mut url = self.endpoint(&["user", "repos"])?;
        url.query_pairs_mut()
            .append_pair("affiliation", "owner,collaborator,organization_member")
            .append_pair("sort", "updated")
            .append_pair("per_page", "100");
        self.all_pages(url).await
    }

    pub async fn repository(&self, repository_id: i64) -> Result<GitHubRepository, AppError> {
        let url = self.endpoint(&["repositories", &repository_id.to_string()])?;
        self.get(url).await
    }

    /// A page of a list endpoint that returns a bare JSON array (`/branches`,
    /// `/commits`, `/pulls`). Delegates to `page_envelope`, which does the
    /// conditional-request and pagination work.
    pub async fn page<T>(&self, url: Url, etag: Option<&str>) -> Result<GitHubPage<T>, AppError>
    where
        T: DeserializeOwned,
    {
        let envelope = self.page_envelope::<Vec<T>>(url, etag).await?;
        Ok(GitHubPage {
            items: envelope.body.unwrap_or_default(),
            next: envelope.next,
            etag: envelope.etag,
            not_modified: envelope.not_modified,
        })
    }

    /// A page whose body is an arbitrary shape. The Actions endpoints
    /// (`/actions/runs`, `/actions/runs/{id}/jobs`) wrap their array in an
    /// object with a `total_count`, so they deserialize to a wrapper struct
    /// rather than a `Vec`. `body` is `None` on a `304 Not Modified`.
    pub async fn page_envelope<T>(
        &self,
        url: Url,
        etag: Option<&str>,
    ) -> Result<GitHubEnvelope<T>, AppError>
    where
        T: DeserializeOwned,
    {
        let mut request = self.request(self.state.http.get(url.clone()));
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|error| AppError::Upstream(error.to_string()))?;
        let status = response.status();
        let headers = response.headers().clone();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(GitHubEnvelope {
                body: None,
                next: None,
                etag: header_text(&headers, header::ETAG),
                not_modified: true,
            });
        }
        if !status.is_success() {
            return Err(api_error(status, &headers, response.text().await.ok()));
        }
        let next = header_text(&headers, header::LINK).and_then(|link| next_link(&link));
        let etag = header_text(&headers, header::ETAG);
        let body = response
            .json::<T>()
            .await
            .map_err(|error| AppError::Upstream(format!("invalid response from {url}: {error}")))?;
        Ok(GitHubEnvelope {
            body: Some(body),
            next,
            etag,
            not_modified: false,
        })
    }

    /// All jobs (with their steps) for one run attempt. Usually a handful, so
    /// pagination rarely fires, but it is honoured for the monorepo case.
    pub async fn run_jobs(
        &self,
        owner: &str,
        repository: &str,
        run_id: i64,
    ) -> Result<Vec<GitHubJob>, AppError> {
        let mut url = self.endpoint(&[
            "repos",
            owner,
            repository,
            "actions",
            "runs",
            &run_id.to_string(),
            "jobs",
        ])?;
        url.query_pairs_mut()
            .append_pair("filter", "latest")
            .append_pair("per_page", "100");
        let mut jobs = Vec::new();
        loop {
            let page = self.page_envelope::<GitHubJobsPage>(url, None).await?;
            if let Some(body) = page.body {
                jobs.extend(body.jobs);
            }
            match page.next {
                Some(next) => url = next,
                None => return Ok(jobs),
            }
        }
    }

    /// Downloads the zipped logs for a run. The endpoint answers `302` to a
    /// short-lived storage URL on a different host; reqwest follows it and drops
    /// the `Authorization` header on the host change, which is exactly what the
    /// signed URL wants. Returns `None` when logs are absent (run still going,
    /// or expired) rather than treating it as an error — logs are best-effort.
    pub async fn run_logs_zip(
        &self,
        owner: &str,
        repository: &str,
        run_id: i64,
    ) -> Result<Option<Vec<u8>>, AppError> {
        let url = self.endpoint(&[
            "repos",
            owner,
            repository,
            "actions",
            "runs",
            &run_id.to_string(),
            "logs",
        ])?;
        let response = self
            .request(self.state.http.get(url))
            .send()
            .await
            .map_err(|error| AppError::Upstream(error.to_string()))?;
        if response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::GONE {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            return Err(api_error(status, &headers, response.text().await.ok()));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| AppError::Upstream(error.to_string()))?;
        Ok(Some(bytes.to_vec()))
    }

    pub async fn first_review_at(
        &self,
        owner: &str,
        repository: &str,
        number: i32,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        let mut url = self.endpoint(&[
            "repos",
            owner,
            repository,
            "pulls",
            &number.to_string(),
            "reviews",
        ])?;
        url.query_pairs_mut().append_pair("per_page", "100");
        let reviews = self.all_pages::<GitHubReview>(url).await?;
        Ok(reviews
            .into_iter()
            .filter_map(|review| review.submitted_at)
            .min())
    }

    pub async fn pull_request(
        &self,
        owner: &str,
        repository: &str,
        number: i32,
    ) -> Result<GitHubPullRequest, AppError> {
        let url = self.endpoint(&["repos", owner, repository, "pulls", &number.to_string()])?;
        self.get(url).await
    }

    pub async fn ensure_webhook(&self, owner: &str, repository: &str) -> Result<(), AppError> {
        let mut hooks_url = self.endpoint(&["repos", owner, repository, "hooks"])?;
        hooks_url.query_pairs_mut().append_pair("per_page", "100");
        let hooks = self.all_pages::<GitHubHook>(hooks_url.clone()).await?;
        let callback = self.state.config.github_webhook_url.as_str();
        if let Some(hook) = hooks
            .iter()
            .find(|hook| hook.config.url.as_deref() == Some(callback))
        {
            let url =
                self.endpoint(&["repos", owner, repository, "hooks", &hook.id.to_string()])?;
            let response = self
                .request(self.state.http.patch(url))
                .json(&UpdateWebhook {
                    active: true,
                    events: &[
                        "push",
                        "pull_request",
                        "pull_request_review",
                        "workflow_run",
                        "workflow_job",
                        "deployment_status",
                    ],
                    config: CreateWebhookConfig {
                        url: callback,
                        content_type: "json",
                        secret: &self.state.config.github_webhook_secret,
                        insecure_ssl: "0",
                    },
                })
                .send()
                .await
                .map_err(|error| AppError::Upstream(error.to_string()))?;
            if !response.status().is_success() {
                let status = response.status();
                let headers = response.headers().clone();
                return Err(api_error(status, &headers, response.text().await.ok()));
            }
            return Ok(());
        }

        let response = self
            .request(self.state.http.post(hooks_url))
            .json(&CreateWebhook {
                name: "web",
                active: true,
                events: &[
                    "push",
                    "pull_request",
                    "pull_request_review",
                    "workflow_run",
                    "workflow_job",
                    "deployment_status",
                ],
                config: CreateWebhookConfig {
                    url: callback,
                    content_type: "json",
                    secret: &self.state.config.github_webhook_secret,
                    insecure_ssl: "0",
                },
            })
            .send()
            .await
            .map_err(|error| AppError::Upstream(error.to_string()))?;
        if response.status() != StatusCode::CREATED {
            let status = response.status();
            let headers = response.headers().clone();
            return Err(api_error(status, &headers, response.text().await.ok()));
        }
        Ok(())
    }

    pub async fn remove_webhook(&self, owner: &str, repository: &str) -> Result<(), AppError> {
        let mut hooks_url = self.endpoint(&["repos", owner, repository, "hooks"])?;
        hooks_url.query_pairs_mut().append_pair("per_page", "100");
        let hooks = self.all_pages::<GitHubHook>(hooks_url).await?;
        let callback = self.state.config.github_webhook_url.as_str();
        for hook in hooks
            .into_iter()
            .filter(|hook| hook.config.url.as_deref() == Some(callback))
        {
            let url =
                self.endpoint(&["repos", owner, repository, "hooks", &hook.id.to_string()])?;
            let response = self
                .request(self.state.http.delete(url))
                .send()
                .await
                .map_err(|error| AppError::Upstream(error.to_string()))?;
            if response.status() != StatusCode::NO_CONTENT {
                let status = response.status();
                let headers = response.headers().clone();
                return Err(api_error(status, &headers, response.text().await.ok()));
            }
        }
        Ok(())
    }

    async fn get<T>(&self, url: Url) -> Result<T, AppError>
    where
        T: DeserializeOwned,
    {
        let response = self
            .request(self.state.http.get(url.clone()))
            .send()
            .await
            .map_err(|error| AppError::Upstream(error.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            return Err(api_error(status, &headers, response.text().await.ok()));
        }
        response
            .json::<T>()
            .await
            .map_err(|error| AppError::Upstream(format!("invalid response from {url}: {error}")))
    }

    async fn all_pages<T>(&self, mut url: Url) -> Result<Vec<T>, AppError>
    where
        T: DeserializeOwned,
    {
        let mut output = Vec::new();
        loop {
            let page = self.page(url, None).await?;
            output.extend(page.items);
            match page.next {
                Some(next) => url = next,
                None => return Ok(output),
            }
        }
    }

    fn request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .bearer_auth(self.token)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
    }
}

pub struct GitHubPage<T> {
    pub items: Vec<T>,
    pub next: Option<Url>,
    pub etag: Option<String>,
    pub not_modified: bool,
}

pub struct GitHubEnvelope<T> {
    pub body: Option<T>,
    pub next: Option<Url>,
    pub etag: Option<String>,
    pub not_modified: bool,
}

/// The `workflow_run` object, shared by the REST list endpoint and the
/// `workflow_run` webhook — they carry the same shape. There is no
/// `completed_at`; when `status == "completed"` the run's `updated_at` is the
/// completion time, which is how the ingest maps it.
#[derive(Clone, Debug, Deserialize)]
pub struct GitHubWorkflowRun {
    pub id: i64,
    pub name: Option<String>,
    pub workflow_id: i64,
    pub run_number: i32,
    #[serde(default = "one")]
    pub run_attempt: i32,
    pub event: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_sha: String,
    pub head_branch: Option<String>,
    pub actor: Option<GitHubUserSummary>,
    pub triggering_actor: Option<GitHubUserSummary>,
    pub created_at: Option<DateTime<Utc>>,
    pub run_started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pull_requests: Vec<GitHubRunPullRequest>,
}

fn one() -> i32 {
    1
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubRunPullRequest {
    pub number: i32,
}

/// The `workflow` object nested in a `workflow_run` webhook and returned by the
/// workflows REST endpoint. Enough to upsert the `workflows` row a run points at.
#[derive(Clone, Debug, Deserialize)]
pub struct GitHubWorkflow {
    pub id: i64,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Deserialize)]
pub struct GitHubRunsPage {
    #[serde(default)]
    pub workflow_runs: Vec<GitHubWorkflowRun>,
}

#[derive(Deserialize)]
pub struct GitHubWorkflowsPage {
    #[serde(default)]
    pub workflows: Vec<GitHubWorkflow>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubJob {
    pub id: i64,
    pub run_id: i64,
    #[serde(default = "one")]
    pub run_attempt: i32,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub runner_id: Option<i64>,
    pub runner_name: Option<String>,
    pub runner_group_name: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub steps: Vec<GitHubStep>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubStep {
    pub number: i32,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct GitHubJobsPage {
    #[serde(default)]
    pub jobs: Vec<GitHubJob>,
}

fn api_error(
    status: StatusCode,
    headers: &header::HeaderMap,
    response_body: Option<String>,
) -> AppError {
    let remaining = header_text(headers, "x-ratelimit-remaining");
    let reset = header_text(headers, "x-ratelimit-reset")
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
        .and_then(|time| time.duration_since(SystemTime::now()).ok())
        .map(|duration| duration.as_secs());
    let retry_after = header_text(headers, header::RETRY_AFTER);
    let rate_detail =
        if remaining.as_deref() == Some("0") || status == StatusCode::TOO_MANY_REQUESTS {
            format!(
                "; rate limited, retry after {} seconds",
                retry_after
                    .or_else(|| reset.map(|seconds| seconds.to_string()))
                    .unwrap_or_else(|| "an unspecified delay".to_string())
            )
        } else if status == StatusCode::FORBIDDEN && retry_after.is_some() {
            format!(
                "; secondary rate limit, retry after {} seconds",
                retry_after.unwrap_or_default()
            )
        } else {
            String::new()
        };
    let message = response_body
        .and_then(|body| serde_json::from_str::<GitHubErrorBody>(&body).ok())
        .map(|body| body.message)
        .unwrap_or_else(|| "request failed".to_string());
    AppError::Upstream(format!("GitHub returned {status}: {message}{rate_detail}"))
}

fn header_text(headers: &header::HeaderMap, name: impl header::AsHeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn next_link(value: &str) -> Option<Url> {
    value.split(',').find_map(|part| {
        let mut pieces = part.trim().split(';');
        let target = pieces.next()?.trim();
        let is_next = pieces.any(|piece| piece.trim() == "rel=\"next\"");
        if !is_next {
            return None;
        }
        Url::parse(target.strip_prefix('<')?.strip_suffix('>')?).ok()
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitHubRepository {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub owner: GitHubUserSummary,
    pub description: Option<String>,
    pub default_branch: String,
    #[serde(rename = "private")]
    pub is_private: bool,
    pub archived: bool,
    pub fork: bool,
    pub language: Option<String>,
    pub html_url: String,
    pub created_at: Option<DateTime<Utc>>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub permissions: Option<GitHubPermissions>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitHubPermissions {
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub maintain: bool,
    #[serde(default)]
    pub push: bool,
    #[serde(default)]
    pub pull: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitHubUserSummary {
    pub id: i64,
    pub login: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubBranch {
    pub name: String,
    pub commit: GitHubBranchHead,
    #[serde(default)]
    pub protected: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubBranchHead {
    pub sha: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubCommit {
    pub sha: String,
    pub commit: GitHubCommitDetail,
    pub author: Option<GitHubUserSummary>,
    #[serde(default)]
    pub parents: Vec<GitHubBranchHead>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubCommitDetail {
    pub message: String,
    pub author: Option<GitHubSignature>,
    pub committer: Option<GitHubSignature>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubSignature {
    pub name: Option<String>,
    pub email: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubPullRequest {
    pub id: i64,
    pub number: i32,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    pub user: Option<GitHubUserSummary>,
    pub head: GitHubPullRef,
    pub base: GitHubPullRef,
    pub merge_commit_sha: Option<String>,
    pub merged_by: Option<GitHubUserSummary>,
    pub additions: Option<i32>,
    pub deletions: Option<i32>,
    pub changed_files: Option<i32>,
    pub commits: Option<i32>,
    #[serde(default)]
    pub comments: i32,
    #[serde(default)]
    pub review_comments: i32,
    #[serde(alias = "created_at")]
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubPullRef {
    #[serde(rename = "ref")]
    pub name: String,
    pub sha: String,
}

#[derive(Deserialize)]
struct GitHubReview {
    submitted_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct GitHubHook {
    id: i64,
    config: GitHubHookConfig,
}

#[derive(Deserialize)]
struct GitHubHookConfig {
    url: Option<String>,
}

#[derive(Serialize)]
struct CreateWebhook<'a> {
    name: &'static str,
    active: bool,
    events: &'a [&'a str],
    config: CreateWebhookConfig<'a>,
}

#[derive(Serialize)]
struct CreateWebhookConfig<'a> {
    url: &'a str,
    content_type: &'static str,
    secret: &'a str,
    insecure_ssl: &'static str,
}

#[derive(Serialize)]
struct UpdateWebhook<'a> {
    active: bool,
    events: &'a [&'a str],
    config: CreateWebhookConfig<'a>,
}

#[derive(Deserialize)]
struct GitHubErrorBody {
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_next_pagination_link() {
        let link = "<https://api.github.test/items?page=2>; rel=\"next\", <https://api.github.test/items?page=4>; rel=\"last\"";
        assert_eq!(
            next_link(link).map(|url| url.to_string()),
            Some("https://api.github.test/items?page=2".to_string())
        );
    }

    #[test]
    fn ignores_link_without_next_relation() {
        assert!(next_link("<https://api.github.test/items?page=1>; rel=\"prev\"").is_none());
    }
}
