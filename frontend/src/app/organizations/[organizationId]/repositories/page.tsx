import Link from "next/link";

import { PageHeader, RepositoryTable } from "@/components/ui";
import { getDashboard } from "@/lib/api";

export default async function RepositoriesPage({ params }: { params: Promise<{ organizationId: string }> }) {
  const { organizationId } = await params;
  const dashboard = await getDashboard(organizationId);
  const scored = dashboard.repositories.filter((repository) => repository.overall_score !== null).length;
  return <><PageHeader eyebrow="Inventory" title="Repositories" description={`${dashboard.repositories.length} tracked repositories, ${scored} with current analytics scores.`} action={<Link className="primaryButton" href={`/organizations/${organizationId}/settings`}>Track a repository</Link>} /><section className="sectionBlock noTop"><RepositoryTable repositories={dashboard.repositories} organizationId={organizationId} /></section></>;
}
