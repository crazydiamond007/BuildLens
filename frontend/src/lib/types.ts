export type Membership = {
  id: string;
  slug: string;
  name: string;
  kind: string;
  role: string;
};

export type Me = {
  id: string;
  email: string;
  name: string | null;
  avatar_url: string | null;
  last_login_at: string | null;
  created_at: string;
  memberships: Membership[];
};

export type GitHubPermissions = {
  admin: boolean;
  maintain: boolean;
  push: boolean;
  pull: boolean;
};

export type TrackingSummary = {
  repository_id: string;
  organization_id: string;
  tracking_enabled: boolean;
};

// `tracking` is null unless some organization the user belongs to has already
// tracked the repository, and its organization_id is not necessarily the one
// currently being viewed: GitHub repositories are claimed by a single workspace.
export type DiscoveredRepository = {
  id: number;
  name: string;
  full_name: string;
  owner: { id: number; login: string };
  description: string | null;
  default_branch: string;
  private: boolean;
  archived: boolean;
  fork: boolean;
  language: string | null;
  html_url: string;
  created_at: string | null;
  pushed_at: string | null;
  permissions: GitHubPermissions | null;
  tracking: TrackingSummary | null;
};

// Discovery's answer. When `installed` is false the workspace has not installed
// the GitHub App yet, so `repositories` is empty and `install_url` is where the
// browser goes to install it and pick repositories.
export type DiscoverResponse = {
  installed: boolean;
  install_url: string;
  repositories: DiscoveredRepository[];
};

export type DoraMetric = {
  id: string;
  repository_id: string | null;
  granularity: string;
  period_start: string;
  period_end: string;
  deployment_count: number;
  deployment_frequency: number | null;
  lead_time_p50_seconds: number | null;
  lead_time_p90_seconds: number | null;
  change_failure_rate: number | null;
  failed_deployment_count: number;
  mttr_p50_seconds: number | null;
  mttr_p90_seconds: number | null;
  performance_band: string | null;
  sample_size: number;
  computed_at: string;
};

export type RepositorySummary = {
  id: string;
  full_name: string;
  description: string | null;
  default_branch: string;
  primary_language: string | null;
  tracking_enabled: boolean;
  is_private: boolean;
  html_url: string | null;
  overall_score: number | null;
  reliability_score: number | null;
  velocity_score: number | null;
  quality_score: number | null;
  efficiency_score: number | null;
  grade: string | null;
  score_computed_at: string | null;
  run_count: number;
  failure_count: number;
  last_run_at: string | null;
  flaky_count: number;
  open_recommendations: number;
};

export type RunSummary = {
  id: string;
  repository_id: string;
  repository: string;
  run_number: number;
  run_attempt: number;
  name: string | null;
  event: string;
  status: string;
  conclusion: string | null;
  head_sha: string;
  head_branch: string | null;
  actor_login: string | null;
  created_at: string | null;
  started_at: string | null;
  completed_at: string | null;
  queued_duration_ms: number | null;
  duration_ms: number | null;
  score: number | null;
};

export type FlakyTest = {
  id: string;
  repository_id: string;
  repository: string;
  test_key: string;
  suite: string | null;
  classname: string | null;
  name: string;
  window_days: number;
  total_runs: number;
  passed_runs: number;
  failed_runs: number;
  flip_count: number;
  flake_rate: number;
  is_flaky: boolean;
  is_quarantined: boolean;
  last_seen_at: string | null;
  last_failed_at: string | null;
  computed_at: string;
};

export type Recommendation = {
  id: string;
  report_id: string;
  repository_id: string;
  repository: string;
  category: string;
  severity: string;
  title: string;
  body_md: string;
  evidence: unknown;
  status: string;
  created_at: string;
};

export type AiReport = {
  id: string;
  repository_id: string | null;
  repository: string | null;
  workflow_run_id: string | null;
  kind: string;
  status: string;
  title: string | null;
  summary: string | null;
  content_md: string | null;
  content: unknown;
  model: string | null;
  prompt_version: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  latency_ms: number | null;
  error: string | null;
  requested_at: string;
  completed_at: string | null;
};

export type Dashboard = {
  organization: Membership;
  dora: DoraMetric[];
  repositories: RepositorySummary[];
  recent_runs: RunSummary[];
  flaky_tests: FlakyTest[];
  recommendations: Recommendation[];
  reports: AiReport[];
};

export type ScorePoint = {
  overall_score: number;
  reliability_score: number | null;
  velocity_score: number | null;
  quality_score: number | null;
  efficiency_score: number | null;
  grade: string | null;
  computed_at: string;
};

export type RepositoryInsights = {
  repository: RepositorySummary;
  dora: DoraMetric[];
  scores: ScorePoint[];
  recent_runs: RunSummary[];
  flaky_tests: FlakyTest[];
  recommendations: Recommendation[];
  reports: AiReport[];
};

export type RunHeader = Omit<RunSummary, "created_at"> & {
  github_run_id: number;
  triggering_actor_login: string | null;
  is_default_branch: boolean;
  created_at: string | null;
  duration_score: number | null;
  reliability_score: number | null;
  flakiness_score: number | null;
  score_breakdown: unknown;
};

export type StepDetail = {
  id: string;
  number: number;
  name: string;
  status: string;
  conclusion: string | null;
  started_at: string | null;
  completed_at: string | null;
  duration_ms: number | null;
};

export type JobDetail = {
  id: string;
  name: string;
  status: string;
  conclusion: string | null;
  runner_name: string | null;
  runner_group_name: string | null;
  labels: string[];
  started_at: string | null;
  completed_at: string | null;
  duration_ms: number | null;
  steps: StepDetail[];
};

export type TestResult = {
  id: string;
  workflow_job_id: string | null;
  test_key: string;
  suite: string | null;
  classname: string | null;
  name: string;
  status: string;
  duration_ms: number | null;
  failure_type: string | null;
  failure_message: string | null;
  executed_at: string;
};

export type BuildLog = {
  id: string;
  size_bytes: number;
  content_type: string;
  content_encoding: string | null;
  line_count: number | null;
  expires_at: string | null;
  created_at: string;
};

export type RunDetail = {
  run: RunHeader;
  jobs: JobDetail[];
  tests: TestResult[];
  report: AiReport | null;
  recommendations: Recommendation[];
  log: BuildLog | null;
};
