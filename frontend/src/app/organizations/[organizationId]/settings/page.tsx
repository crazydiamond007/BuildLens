import { InstallPanel, RepositoryTracking } from "@/components/repository-tracking";
import { PageHeader } from "@/components/ui";
import { getDiscoveredRepositories } from "@/lib/api";

export default async function SettingsPage({
  params,
}: {
  params: Promise<{ organizationId: string }>;
}) {
  const { organizationId } = await params;
  const discovery = await getDiscoveredRepositories(organizationId);
  return (
    <>
      <PageHeader
        eyebrow="Settings"
        title="Repository tracking"
        description="Choose which GitHub repositories this workspace collects analytics for. Tracking backfills recent history, so numbers take a few minutes to appear."
      />
      <section className="sectionBlock noTop">
        {discovery.installed ? (
          <RepositoryTracking
            organizationId={organizationId}
            repositories={discovery.repositories}
          />
        ) : (
          <InstallPanel installUrl={discovery.install_url} />
        )}
      </section>
    </>
  );
}
