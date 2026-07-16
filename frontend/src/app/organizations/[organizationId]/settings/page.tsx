import { RepositoryTracking } from "@/components/repository-tracking";
import { PageHeader } from "@/components/ui";
import { getDiscoveredRepositories } from "@/lib/api";

export default async function SettingsPage({
  params,
}: {
  params: Promise<{ organizationId: string }>;
}) {
  const { organizationId } = await params;
  const repositories = await getDiscoveredRepositories();
  return (
    <>
      <PageHeader
        eyebrow="Settings"
        title="Repository tracking"
        description="Choose which GitHub repositories this workspace collects analytics for. Tracking installs a webhook and backfills recent history, so numbers take a few minutes to appear."
      />
      <section className="sectionBlock noTop">
        <RepositoryTracking organizationId={organizationId} repositories={repositories} />
      </section>
    </>
  );
}
