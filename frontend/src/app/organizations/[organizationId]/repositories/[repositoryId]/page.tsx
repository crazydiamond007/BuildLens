import Link from "next/link";

import { MetricCard, PageHeader, RecommendationCard, RunsTable, ScoreBadge } from "@/components/ui";
import { getRepositoryInsights } from "@/lib/api";
import { formatPercent, formatScore, formatSeconds, relativeTime } from "@/lib/format";

export default async function RepositoryPage({ params }: { params: Promise<{ organizationId: string; repositoryId: string }> }) {
  const { organizationId, repositoryId } = await params;
  const insights = await getRepositoryInsights(organizationId, repositoryId);
  const repo = insights.repository;
  const latest = insights.dora[0];
  const chronological = [...insights.dora].reverse();
  return <>
    <PageHeader eyebrow="Repository" title={repo.full_name} description={repo.description ?? `Tracking ${repo.default_branch} with ${repo.run_count} workflow runs.`} action={<div className="headerScore"><ScoreBadge value={repo.overall_score} grade={repo.grade} />{repo.html_url && <a className="secondaryButton" href={repo.html_url} target="_blank" rel="noreferrer">Open on GitHub</a>}</div>} />
    <div className="repoMeta"><span>{repo.primary_language ?? "Language unknown"}</span><span>{repo.is_private ? "Private" : "Public"}</span><span>{repo.default_branch}</span><span>Last run {relativeTime(repo.last_run_at)}</span></div>
    <section className="metricGrid">
      <MetricCard label="Overall score" value={formatScore(repo.overall_score)} detail={`Grade ${repo.grade ?? "pending"}`} values={[...insights.scores].reverse().map((score) => score.overall_score)} />
      <MetricCard label="Deployment frequency" value={latest?.deployment_frequency?.toFixed(1) ?? "No data"} detail={`${latest?.deployment_count ?? 0} deployments`} values={chronological.map((metric) => metric.deployment_frequency)} tone="success" />
      <MetricCard label="Lead time p50" value={formatSeconds(latest?.lead_time_p50_seconds ?? null)} detail={`p90 ${formatSeconds(latest?.lead_time_p90_seconds ?? null)}`} values={chronological.map((metric) => metric.lead_time_p50_seconds)} />
      <MetricCard label="Failure rate" value={formatPercent(latest?.change_failure_rate ?? null)} detail={`${repo.failure_count} failed runs`} values={chronological.map((metric) => metric.change_failure_rate)} tone="warning" />
    </section>
    <section className="sectionBlock"><div className="sectionHeading"><div><p className="eyebrow">Build history</p><h2>Recent runs</h2></div><Link href={`/organizations/${organizationId}/runs`}>All organization runs</Link></div><RunsTable runs={insights.recent_runs} organizationId={organizationId} /></section>
    <section className="splitGrid sectionBlock"><div><div className="sectionHeading"><div><p className="eyebrow">Grounded guidance</p><h2>Recommendations</h2></div></div><div className="stackList">{insights.recommendations.map((recommendation) => <RecommendationCard key={recommendation.id} recommendation={recommendation} />)}{!insights.recommendations.length && <div className="quietState">No recommendations for this repository.</div>}</div></div><div><div className="sectionHeading"><div><p className="eyebrow">Test stability</p><h2>Flaky tests</h2></div></div><div className="compactList">{insights.flaky_tests.map((test) => <div key={test.id}><span><strong>{test.name}</strong><small>{test.test_key}</small></span><span className="mono">{(test.flake_rate * 100).toFixed(1)}%</span></div>)}{!insights.flaky_tests.length && <div className="quietState">No flaky tests detected.</div>}</div></div></section>
  </>;
}
