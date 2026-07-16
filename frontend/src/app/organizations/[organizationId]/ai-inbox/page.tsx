import { EmptyState, PageHeader, RecommendationCard, ReportCard } from "@/components/ui";
import { getDashboard } from "@/lib/api";

export default async function AiInboxPage({ params }: { params: Promise<{ organizationId: string }> }) {
  const { organizationId } = await params;
  const dashboard = await getDashboard(organizationId);
  return <><PageHeader eyebrow="Grounded intelligence" title="AI inbox" description="Failure analysis and recommendations generated from bounded logs, workflow facts, tests, and analytics evidence." />
    <div className="boundaryNotice"><strong>Read-only evidence boundary</strong><span>Recommendation status changes remain disabled because the gateway does not own AI worker tables. That grant boundary is intentionally preserved.</span></div>
    <section className="splitGrid sectionBlock"><div><div className="sectionHeading"><div><p className="eyebrow">Action queue</p><h2>Recommendations</h2></div><span className="subtleTag">{dashboard.recommendations.length} open</span></div><div className="stackList">{dashboard.recommendations.map((recommendation) => <RecommendationCard key={recommendation.id} recommendation={recommendation} />)}{!dashboard.recommendations.length && <EmptyState title="Inbox is clear" body="Open recommendations will appear after the AI worker completes a grounded report." />}</div></div><div><div className="sectionHeading"><div><p className="eyebrow">Analysis history</p><h2>Reports</h2></div><span className="subtleTag">{dashboard.reports.length} recent</span></div><div className="stackList">{dashboard.reports.map((report) => <ReportCard key={report.id} report={report} />)}{!dashboard.reports.length && <EmptyState title="No AI reports yet" body="Failed runs can trigger reports when the AI worker is configured with a provider key." />}</div></div></section>
  </>;
}
