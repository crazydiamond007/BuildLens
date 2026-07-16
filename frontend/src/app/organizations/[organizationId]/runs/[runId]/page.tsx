import Link from "next/link";

import { EmptyState, PageHeader, RecommendationCard, ScoreBadge, StatusPill } from "@/components/ui";
import { getRun } from "@/lib/api";
import { formatDateTime, formatDuration, shortSha } from "@/lib/format";

export default async function RunPage({ params }: { params: Promise<{ organizationId: string; runId: string }> }) {
  const { organizationId, runId } = await params;
  const detail = await getRun(organizationId, runId);
  const run = detail.run;
  const failedTests = detail.tests.filter((test) => ["failed", "error", "failure"].includes(test.status));
  return <>
    <PageHeader eyebrow={`${run.repository} / run #${run.run_number}`} title={run.name ?? `Workflow run #${run.run_number}`} description={`${run.event} on ${run.head_branch ?? "detached head"} at ${shortSha(run.head_sha)}`} action={<div className="headerScore"><StatusPill value={run.conclusion ?? run.status} /><ScoreBadge value={run.score} /></div>} />
    <div className="runFacts"><div><small>Duration</small><strong>{formatDuration(run.duration_ms)}</strong></div><div><small>Queued</small><strong>{formatDuration(run.queued_duration_ms)}</strong></div><div><small>Actor</small><strong>{run.actor_login ?? "Unknown"}</strong></div><div><small>Completed</small><strong>{formatDateTime(run.completed_at)}</strong></div><div><small>Attempt</small><strong>{run.run_attempt}</strong></div></div>

    {detail.report && <section className="analysisPanel"><div className="analysisHeader"><div><p className="eyebrow">Grounded AI analysis</p><h2>{detail.report.title ?? "Build analysis"}</h2></div><StatusPill value={detail.report.status} /></div><p className="analysisSummary">{detail.report.summary ?? detail.report.error ?? "The report is processing."}</p>{detail.report.content_md && <div className="reportContent">{detail.report.content_md}</div>}<div className="analysisFooter"><span>{detail.report.model ?? "Model pending"}</span><span>Prompt {detail.report.prompt_version ?? "pending"}</span>{detail.report.cost_usd !== null && <span className="mono">${detail.report.cost_usd.toFixed(4)}</span>}</div></section>}

    <section className="sectionBlock"><div className="sectionHeading"><div><p className="eyebrow">Execution trace</p><h2>Jobs and steps</h2></div>{detail.log && <span className="subtleTag">Log archive {Math.round(detail.log.size_bytes / 1024)} KB</span>}</div>{detail.jobs.length ? <div className="jobList">{detail.jobs.map((job) => <details key={job.id} open={job.conclusion === "failure"}><summary><span><StatusPill value={job.conclusion ?? job.status} /><strong>{job.name}</strong></span><span className="mono">{formatDuration(job.duration_ms)}</span></summary><div className="stepList">{job.steps.map((step) => <div key={step.id}><span className="stepNumber">{step.number}</span><StatusPill value={step.conclusion ?? step.status} /><span>{step.name}</span><span className="mono">{formatDuration(step.duration_ms)}</span></div>)}</div></details>)}</div> : <EmptyState title="No job details" body="GitHub did not return jobs for this workflow run." />}</section>

    <section className="splitGrid sectionBlock"><div><div className="sectionHeading"><div><p className="eyebrow">Test evidence</p><h2>Failing tests</h2></div><span className="subtleTag">{failedTests.length} failures</span></div>{failedTests.length ? <div className="testList">{failedTests.map((test) => <article key={test.id}><div><StatusPill value={test.status} /><strong>{test.name}</strong></div><p className="mono">{test.test_key}</p>{test.failure_message && <pre>{test.failure_message}</pre>}</article>)}</div> : <div className="quietState">No failing JUnit results were captured for this run.</div>}</div><div><div className="sectionHeading"><div><p className="eyebrow">Recommended actions</p><h2>Next steps</h2></div><Link href={`/organizations/${organizationId}/ai-inbox`}>AI inbox</Link></div><div className="stackList">{detail.recommendations.map((recommendation) => <RecommendationCard key={recommendation.id} recommendation={recommendation} />)}{!detail.recommendations.length && <div className="quietState">No recommendations are attached to this run.</div>}</div></div></section>
  </>;
}
