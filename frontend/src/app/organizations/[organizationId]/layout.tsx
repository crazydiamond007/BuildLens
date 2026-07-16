import { notFound, redirect } from "next/navigation";

import { AppShell } from "@/components/app-shell";
import { ApiError, getMe } from "@/lib/api";

export default async function OrganizationLayout({ children, params }: { children: React.ReactNode; params: Promise<{ organizationId: string }> }) {
  const { organizationId } = await params;
  let me;
  try {
    me = await getMe();
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) redirect("/");
    throw error;
  }
  const membership = me.memberships.find((item) => item.id === organizationId);
  if (!membership) notFound();
  const gatewayPublicUrl = process.env.GATEWAY_PUBLIC_URL ?? "http://localhost:8080";
  return (
    <AppShell
      membership={membership}
      memberships={me.memberships}
      user={{ name: me.name, email: me.email, avatarUrl: me.avatar_url }}
      logoutUrl={`${gatewayPublicUrl}/auth/logout`}
    >
      {children}
    </AppShell>
  );
}
