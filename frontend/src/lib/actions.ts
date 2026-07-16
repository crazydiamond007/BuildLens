"use server";

import { refresh } from "next/cache";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";

import { ApiError, send } from "./api";

export type MutationResult = { ok: true } | { ok: false; message: string };

// A rejected tracking attempt is an ordinary outcome, not a crash: the gateway
// refuses repositories the user lacks GitHub admin on, and repositories another
// workspace already claimed. Both need to reach the row that triggered them, so
// they are returned rather than thrown into the error boundary.
async function mutate(path: string, method: "PUT" | "DELETE"): Promise<MutationResult> {
  try {
    await send(path, method);
  } catch (error) {
    if (error instanceof ApiError) {
      return { ok: false, message: error.message };
    }
    throw error;
  }
  refresh();
  return { ok: true };
}

export async function trackRepository(
  organizationId: string,
  githubRepositoryId: number,
): Promise<MutationResult> {
  return mutate(
    `/organizations/${organizationId}/github-repositories/${githubRepositoryId}/tracking`,
    "PUT",
  );
}

export async function untrackRepository(
  organizationId: string,
  repositoryId: string,
): Promise<MutationResult> {
  return mutate(
    `/organizations/${organizationId}/repositories/${repositoryId}/tracking`,
    "DELETE",
  );
}

// Signing out must always end at the sign-in page, so a gateway that refuses
// the call cannot leave someone stuck on a page they can no longer load. The
// gateway clears the cookie on its own response, which a server-to-server fetch
// never shows the browser, so the cookie is dropped here as well.
export async function signOut(): Promise<never> {
  try {
    await send("/auth/logout", "POST");
  } catch (error) {
    if (!(error instanceof ApiError)) throw error;
  }
  (await cookies()).delete("buildlens_session");
  redirect("/");
}

