import "server-only";

import { cookies } from "next/headers";

import type { Dashboard, Me, RepositoryInsights, RunDetail } from "./types";

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

const gatewayUrl = process.env.GATEWAY_INTERNAL_URL ?? "http://localhost:8080";

async function request<T>(path: string): Promise<T> {
  const cookieStore = await cookies();
  const session = cookieStore.get("buildlens_session");
  const headers = new Headers({ Accept: "application/json" });
  if (session) {
    headers.set("Cookie", `buildlens_session=${session.value}`);
  }

  const response = await fetch(`${gatewayUrl}${path}`, {
    headers,
    cache: "no-store",
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new ApiError(
      response.status,
      body?.error?.message ?? `Gateway returned ${response.status}`,
    );
  }
  return (await response.json()) as T;
}

export function getMe(): Promise<Me> {
  return request<Me>("/me");
}

export function getDashboard(organizationId: string): Promise<Dashboard> {
  return request<Dashboard>(`/organizations/${organizationId}/dashboard`);
}

export function getRepositoryInsights(
  organizationId: string,
  repositoryId: string,
): Promise<RepositoryInsights> {
  return request<RepositoryInsights>(
    `/organizations/${organizationId}/repositories/${repositoryId}/insights`,
  );
}

export function getRun(
  organizationId: string,
  runId: string,
): Promise<RunDetail> {
  return request<RunDetail>(`/organizations/${organizationId}/runs/${runId}`);
}

export function githubLoginUrl(): string {
  const publicUrl = process.env.GATEWAY_PUBLIC_URL ?? "http://localhost:8080";
  return `${publicUrl}/auth/github/login`;
}
