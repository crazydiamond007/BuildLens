import { PageHeader, RunsTable } from "@/components/ui";
import { getDashboard } from "@/lib/api";

export default async function RunsPage({ params }: { params: Promise<{ organizationId: string }> }) {
  const { organizationId } = await params;
  const dashboard = await getDashboard(organizationId);
  const failed = dashboard.recent_runs.filter((run) => ["failure", "failed"].includes(run.conclusion ?? "")).length;
  return <><PageHeader eyebrow="GitHub Actions" title="Workflow runs" description={`Latest ${dashboard.recent_runs.length} runs across ${dashboard.repositories.length} repositories. ${failed} failed.`} /><section className="sectionBlock noTop"><RunsTable runs={dashboard.recent_runs} organizationId={organizationId} /></section></>;
}
