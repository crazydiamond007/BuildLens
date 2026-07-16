import Link from "next/link";

import { formatDuration, formatScore, relativeTime, sentenceCase, shortSha } from "@/lib/format";
import type { AiReport, Recommendation, RepositorySummary, RunSummary } from "@/lib/types";

export function PageHeader({ eyebrow, title, description, action }: { eyebrow?: string; title: string; description?: string; action?: React.ReactNode }) {
  return (
    <div className="pageHeader">
      <div>
        {eyebrow && <p className="eyebrow">{eyebrow}</p>}
        <h1>{title}</h1>
        {description && <p className="pageDescription">{description}</p>}
      </div>
      {action && <div className="pageActions">{action}</div>}
    </div>
  );
}

export function EmptyState({ title, body }: { title: string; body: string }) {
  return <div className="emptyState"><span className="emptyGlyph">+</span><h3>{title}</h3><p>{body}</p></div>;
}

export function StatusPill({ value }: { value: string | null }) {
  const normalized = value?.toLowerCase() ?? "unknown";
  const tone = ["success", "passed", "completed"].includes(normalized)
    ? "success"
    : ["failure", "failed", "error", "cancelled", "critical", "high"].includes(normalized)
      ? "failure"
      : ["in_progress", "queued", "waiting", "medium"].includes(normalized)
        ? "warning"
        : "neutral";
  return <span className={`statusPill ${tone}`}>{sentenceCase(value)}</span>;
}

export function BandPill({ value }: { value: string | null }) {
  const band = value?.toLowerCase() ?? "unknown";
  return <span className={`statusPill band ${band}`}>{sentenceCase(value)}</span>;
}

export function Sparkline({ values, color = "var(--accent-line)" }: { values: Array<number | null>; color?: string }) {
  const clean = values.map((value) => value ?? 0);
  if (clean.length < 2 || clean.every((value) => value === clean[0])) return <div className="sparklineEmpty" />;
  const min = Math.min(...clean);
  const max = Math.max(...clean);
  const range = max - min || 1;
  const points = clean.map((value, index) => `${(index / (clean.length - 1)) * 100},${34 - ((value - min) / range) * 28}`).join(" ");
  return <svg className="sparkline" viewBox="0 0 100 40" preserveAspectRatio="none" aria-hidden="true"><polyline points={points} fill="none" stroke={color} strokeWidth="2" vectorEffect="non-scaling-stroke" /></svg>;
}

export function MetricCard({ label, value, detail, values, tone }: { label: string; value: string; detail: string; values: Array<number | null>; tone?: "success" | "warning" | "failure" }) {
  const color = tone ? `var(--${tone})` : "var(--accent-line)";
  return <article className="metricCard"><div className="metricTop"><span>{label}</span><i style={{ background: color }} /></div><strong>{value}</strong><p>{detail}</p><Sparkline values={values} color={color} /></article>;
}

export function ScoreBadge({ value, grade }: { value: number | null; grade?: string | null }) {
  const tone = value === null ? "neutral" : value >= 85 ? "success" : value >= 65 ? "warning" : "failure";
  return <span className={`scoreBadge ${tone}`}><strong>{formatScore(value)}</strong>{grade && <small>{grade}</small>}</span>;
}

export function RunsTable({ runs, organizationId, compact = false }: { runs: RunSummary[]; organizationId: string; compact?: boolean }) {
  if (!runs.length) return <EmptyState title="No workflow runs yet" body="Runs will appear after a tracked repository completes a GitHub Actions workflow." />;
  return (
    <div className="tableWrap">
      <table>
        <thead><tr><th>Workflow</th><th>Repository</th><th>Status</th><th>Branch / commit</th><th>Duration</th><th>Score</th><th>Finished</th></tr></thead>
        <tbody>
          {runs.map((run) => (
            <tr key={run.id}>
              <td><Link className="primaryLink" href={`/organizations/${organizationId}/runs/${run.id}`}>{run.name ?? `Run #${run.run_number}`}</Link><small>#{run.run_number}{run.run_attempt > 1 ? ` attempt ${run.run_attempt}` : ""}</small></td>
              <td>{run.repository}</td>
              <td><StatusPill value={run.conclusion ?? run.status} /></td>
              <td><span className="mono">{run.head_branch ?? "detached"}</span><small className="mono">{shortSha(run.head_sha)}</small></td>
              <td className="mono">{formatDuration(run.duration_ms)}</td>
              <td><ScoreBadge value={run.score} /></td>
              <td>{relativeTime(run.completed_at ?? run.started_at)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {compact && runs.length > 8 && <p className="tableNote">Showing the latest {runs.length} runs</p>}
    </div>
  );
}

export function RepositoryTable({ repositories, organizationId }: { repositories: RepositorySummary[]; organizationId: string }) {
  if (!repositories.length) return <EmptyState title="No tracked repositories" body="Track a GitHub repository from the gateway API to begin collecting workflow analytics." />;
  return (
    <div className="tableWrap"><table><thead><tr><th>Repository</th><th>Score</th><th>Reliability</th><th>Runs</th><th>Failures</th><th>Flaky</th><th>Recommendations</th><th>Last run</th></tr></thead><tbody>
      {repositories.map((repo) => <tr key={repo.id}>
        <td><Link className="primaryLink" href={`/organizations/${organizationId}/repositories/${repo.id}`}>{repo.full_name}</Link><small>{repo.primary_language ?? repo.default_branch}{repo.is_private ? " / private" : ""}</small></td>
        <td><ScoreBadge value={repo.overall_score} grade={repo.grade} /></td>
        <td className="mono">{formatScore(repo.reliability_score)}</td><td className="mono">{repo.run_count}</td><td className="mono">{repo.failure_count}</td><td className="mono">{repo.flaky_count}</td><td className="mono">{repo.open_recommendations}</td><td>{relativeTime(repo.last_run_at)}</td>
      </tr>)}
    </tbody></table></div>
  );
}

export function RecommendationCard({ recommendation }: { recommendation: Recommendation }) {
  return <article className="recommendationCard"><div className="recommendationMeta"><StatusPill value={recommendation.severity} /><span>{recommendation.repository}</span><span>{sentenceCase(recommendation.category)}</span><span>{relativeTime(recommendation.created_at)}</span></div><h3>{recommendation.title}</h3><p className="markdownText">{recommendation.body_md}</p></article>;
}

export function ReportCard({ report }: { report: AiReport }) {
  return <article className="reportCard"><div><p className="eyebrow">{sentenceCase(report.kind)}</p><h3>{report.title ?? "Analysis report"}</h3><p>{report.summary ?? report.error ?? "Report generation is still in progress."}</p></div><div className="reportMeta"><StatusPill value={report.status} /><span>{report.repository ?? "Organization"}</span><span>{relativeTime(report.completed_at ?? report.requested_at)}</span>{report.cost_usd !== null && <span className="mono">${report.cost_usd.toFixed(4)}</span>}</div></article>;
}
