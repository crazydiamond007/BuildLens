import "server-only";

import { cookies } from "next/headers";

import type {
  Dashboard,
  DiscoveredRepository,
  Me,
  RepositoryInsights,
  RunDetail,
} from "./types";

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

const gatewayUrl = process.env.GATEWAY_INTERNAL_URL ?? "http://localhost:8080";

async function call(path: string, method: string): Promise<Response> {
  const cookieStore = await cookies();
  const session = cookieStore.get("buildlens_session");
  const headers = new Headers({ Accept: "application/json" });
  if (session) {
    headers.set("Cookie", `buildlens_session=${session.value}`);
  }

  return fetch(`${gatewayUrl}${path}`, {
    method,
    headers,
    cache: "no-store",
  });
}

async function apiError(response: Response): Promise<ApiError> {
  const body = (await response.json().catch(() => null)) as
    | { error?: { message?: string } }
    | null;
  return new ApiError(
    response.status,
    body?.error?.message ?? `Gateway returned ${response.status}`,
  );
}

async function request<T>(path: string): Promise<T> {
  const response = await call(path, "GET");
  if (!response.ok) {
    throw await apiError(response);
  }
  return (await response.json()) as T;
}

// The gateway is the only authority on whether the caller may perform this, so
// nothing is checked here beyond forwarding the session cookie it authenticates
// with. Both tracking mutations answer with an empty or ignorable body.
export async function send(path: string, method: "POST" | "PUT" | "DELETE"): Promise<void> {
  const response = await call(path, method);
  if (!response.ok) {
    throw await apiError(response);
  }
}

export function getMe(): Promise<Me> {
  return request<Me>("/me");
}

export function getDiscoveredRepositories(): Promise<DiscoveredRepository[]> {
  return request<DiscoveredRepository[]>("/github/repositories");
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
