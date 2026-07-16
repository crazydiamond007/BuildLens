import Link from "next/link";

import { EmptyState, PageHeader, StatusPill } from "@/components/ui";
import { getDashboard } from "@/lib/api";
import { formatDateTime } from "@/lib/format";

export default async function FlakyTestsPage({ params }: { params: Promise<{ organizationId: string }> }) {
  const { organizationId } = await params;
  const dashboard = await getDashboard(organizationId);
  const tests = dashboard.flaky_tests;
  return <><PageHeader eyebrow="Reliability" title="Flaky tests" description={`${tests.length} tests have unstable outcomes across comparable retries on unchanged commits.`} />
    <section className="sectionBlock noTop">{tests.length ? <div className="tableWrap"><table><thead><tr><th>Test</th><th>Repository</th><th>Flake rate</th><th>Flips</th><th>Pass / fail</th><th>State</th><th>Last failure</th></tr></thead><tbody>{tests.map((test) => <tr key={test.id}><td><strong>{test.name}</strong><small className="mono">{test.test_key}</small></td><td><Link className="primaryLink" href={`/organizations/${organizationId}/repositories/${test.repository_id}`}>{test.repository}</Link></td><td><span className="rateBar"><i style={{ width: `${Math.min(test.flake_rate * 100, 100)}%` }} /></span><span className="mono">{(test.flake_rate * 100).toFixed(1)}%</span></td><td className="mono">{test.flip_count}</td><td className="mono">{test.passed_runs} / {test.failed_runs}</td><td><StatusPill value={test.is_quarantined ? "quarantined" : "active"} /></td><td>{formatDateTime(test.last_failed_at)}</td></tr>)}</tbody></table></div> : <EmptyState title="No flaky tests detected" body="Analytics needs comparable workflow retries on the same commit before it can identify outcome flips." />}</section>
  </>;
}
