import Link from "next/link";

import { MetricCard, PageHeader, RecommendationCard, RepositoryTable, RunsTable } from "@/components/ui";
import { getDashboard } from "@/lib/api";
import { formatPercent, formatSeconds } from "@/lib/format";

export default async function OverviewPage({ params }: { params: Promise<{ organizationId: string }> }) {
  const { organizationId } = await params;
  const dashboard = await getDashboard(organizationId);
  const latest = dashboard.dora[0];
  const chronological = [...dashboard.dora].reverse();
  return (
    <>
      <PageHeader eyebrow="Organization overview" title={dashboard.organization.name} description="Delivery performance, build health, and grounded recommendations in one view." action={<span className="roleBadge">{dashboard.organization.role}</span>} />
      <section className="metricGrid">
        <MetricCard label="Deployment frequency" value={latest?.deployment_frequency?.toFixed(1) ?? "No data"} detail={`${latest?.deployment_count ?? 0} deployments in latest week`} values={chronological.map((metric) => metric.deployment_frequency)} tone="success" />
        <MetricCard label="Lead time p50" value={formatSeconds(latest?.lead_time_p50_seconds ?? null)} detail={`p90 ${formatSeconds(latest?.lead_time_p90_seconds ?? null)}`} values={chronological.map((metric) => metric.lead_time_p50_seconds)} />
        <MetricCard label="Change failure rate" value={formatPercent(latest?.change_failure_rate ?? null)} detail={`${latest?.failed_deployment_count ?? 0} failed deployments`} values={chronological.map((metric) => metric.change_failure_rate)} tone={(latest?.change_failure_rate ?? 0) > 0.15 ? "warning" : "success"} />
        <MetricCard label="Recovery time p50" value={formatSeconds(latest?.mttr_p50_seconds ?? null)} detail={`Sample size ${latest?.sample_size ?? 0}`} values={chronological.map((metric) => metric.mttr_p50_seconds)} tone="warning" />
      </section>

      <section className="sectionBlock">
        <div className="sectionHeading"><div><p className="eyebrow">Repository health</p><h2>Scoreboard</h2></div><Link href={`/organizations/${organizationId}/repositories`}>View all repositories</Link></div>
        <RepositoryTable repositories={dashboard.repositories.slice(0, 6)} organizationId={organizationId} />
      </section>

      <section className="sectionBlock">
        <div className="sectionHeading"><div><p className="eyebrow">Workflow activity</p><h2>Recent runs</h2></div><Link href={`/organizations/${organizationId}/runs`}>View all runs</Link></div>
        <RunsTable runs={dashboard.recent_runs.slice(0, 8)} organizationId={organizationId} compact />
      </section>

      <section className="splitGrid sectionBlock">
        <div>
          <div className="sectionHeading"><div><p className="eyebrow">AI inbox</p><h2>Open recommendations</h2></div><Link href={`/organizations/${organizationId}/ai-inbox`}>Open inbox</Link></div>
          <div className="stackList">{dashboard.recommendations.slice(0, 3).map((recommendation) => <RecommendationCard key={recommendation.id} recommendation={recommendation} />)}{!dashboard.recommendations.length && <div className="quietState">No open recommendations.</div>}</div>
        </div>
        <div>
          <div className="sectionHeading"><div><p className="eyebrow">Reliability signal</p><h2>Flaky tests</h2></div><Link href={`/organizations/${organizationId}/flaky-tests`}>Inspect tests</Link></div>
          <div className="compactList">{dashboard.flaky_tests.slice(0, 6).map((test) => <div key={test.id}><span><strong>{test.name}</strong><small>{test.repository}</small></span><span className="mono failureText">{(test.flake_rate * 100).toFixed(1)}%</span></div>)}{!dashboard.flaky_tests.length && <div className="quietState">No flaky tests detected.</div>}</div>
        </div>
      </section>
    </>
  );
}
