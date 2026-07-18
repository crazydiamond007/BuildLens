"use client";

import { useMemo, useState, useTransition } from "react";

import { trackRepository, untrackRepository } from "@/lib/actions";
import { relativeTime } from "@/lib/format";
import type { DiscoveredRepository } from "@/lib/types";

import { EmptyState, StatusPill } from "./ui";

type Availability =
  | { kind: "tracked"; repositoryId: string }
  | { kind: "trackable" }
  | { kind: "blocked"; reason: string };

// A repository belongs to exactly one workspace - a gateway rule, decided here
// too so a claimed repository shows a disabled row that explains itself rather
// than a button that fails only after being pressed. Everything discovery
// returns is inside the workspace's installation, so access itself is no longer
// a per-repository question.
function availability(
  repository: DiscoveredRepository,
  organizationId: string,
): Availability {
  const tracking = repository.tracking;
  if (tracking && tracking.organization_id !== organizationId) {
    return { kind: "blocked", reason: "Tracked by another workspace" };
  }
  if (tracking?.tracking_enabled) {
    return { kind: "tracked", repositoryId: tracking.repository_id };
  }
  return { kind: "trackable" };
}

// Before the App is installed there is nothing to list: send the admin to GitHub
// to install it and pick repositories. This is the middle step of the
// sign-in -> install -> track flow. Kept hook-free and separate so the tracking
// list's own hooks always run in the same order.
export function InstallPanel({ installUrl }: { installUrl: string }) {
  return (
    <div className="installPanel">
      <h3>Install BuildLens on GitHub</h3>
      <p>
        BuildLens reads workflow runs and logs through a GitHub App you install on
        the repositories you choose. Nothing is tracked until you install it and
        pick which repositories it can see.
      </p>
      <a className="primaryButton" href={installUrl}>
        Install on GitHub
      </a>
      <p className="installNote">
        You&apos;ll pick repositories on GitHub, then land back here to start
        tracking them.
      </p>
    </div>
  );
}

export function RepositoryTracking({
  organizationId,
  repositories,
}: {
  organizationId: string;
  repositories: DiscoveredRepository[];
}) {

  const [query, setQuery] = useState("");
  const [onlyTrackable, setOnlyTrackable] = useState(false);
  const [pendingId, setPendingId] = useState<number | null>(null);
  const [errors, setErrors] = useState<Record<number, string>>({});
  const [justTracked, setJustTracked] = useState<number[]>([]);
  const [isPending, startTransition] = useTransition();

  const rows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return repositories
      .map((repository) => ({
        repository,
        state: availability(repository, organizationId),
      }))
      .filter(({ repository, state }) => {
        if (onlyTrackable && state.kind !== "trackable") return false;
        if (!needle) return true;
        return (
          repository.full_name.toLowerCase().includes(needle) ||
          (repository.language ?? "").toLowerCase().includes(needle)
        );
      })
      .sort((a, b) => {
        const rank = (kind: Availability["kind"]) =>
          kind === "tracked" ? 0 : kind === "trackable" ? 1 : 2;
        return (
          rank(a.state.kind) - rank(b.state.kind) ||
          a.repository.full_name.localeCompare(b.repository.full_name)
        );
      });
  }, [repositories, organizationId, query, onlyTrackable]);

  function run(githubId: number, action: () => Promise<{ ok: true } | { ok: false; message: string }>) {
    setPendingId(githubId);
    startTransition(async () => {
      const result = await action();
      setPendingId(null);
      setErrors((current) => {
        const next = { ...current };
        if (result.ok) delete next[githubId];
        else next[githubId] = result.message;
        return next;
      });
    });
  }

  const trackedCount = repositories.filter(
    (repository) =>
      repository.tracking?.tracking_enabled &&
      repository.tracking.organization_id === organizationId,
  ).length;

  return (
    <>
      <div className="trackToolbar">
        <input
          className="searchInput"
          type="search"
          value={query}
          placeholder="Filter by name or language"
          aria-label="Filter repositories"
          onChange={(event) => setQuery(event.target.value)}
        />
        <label className="trackFilter">
          <input
            type="checkbox"
            checked={onlyTrackable}
            onChange={(event) => setOnlyTrackable(event.target.checked)}
          />
          Only untracked
        </label>
        <span className="trackCount mono">
          {trackedCount} / {repositories.length} tracked
        </span>
      </div>

      {!rows.length ? (
        <EmptyState
          title="No repositories match"
          body="Nothing here matches the current filter. Clear it to see every repository the BuildLens app can reach, or add more repositories to the app on GitHub."
        />
      ) : (
        <div className="trackList">
          {rows.map(({ repository, state }) => {
            const busy = pendingId === repository.id && isPending;
            const error = errors[repository.id];
            const syncing = justTracked.includes(repository.id);
            return (
              <div className="trackRow" key={repository.id}>
                <div className="trackMain">
                  <div className="trackName">
                    <strong>{repository.full_name}</strong>
                    {repository.private && <span className="trackTag">Private</span>}
                    {repository.fork && <span className="trackTag">Fork</span>}
                    {repository.archived && <span className="trackTag">Archived</span>}
                    {state.kind === "tracked" && <StatusPill value="tracked" />}
                  </div>
                  <small>
                    {repository.description ??
                      `${repository.language ?? "No language"} / ${repository.default_branch}`}
                    {repository.pushed_at && ` / pushed ${relativeTime(repository.pushed_at)}`}
                  </small>
                </div>

                <div className="trackSide">
                  {error ? (
                    <span className="trackNote error">{error}</span>
                  ) : syncing && state.kind === "tracked" ? (
                    <span className="trackNote">Backfilling history, data appears shortly</span>
                  ) : state.kind === "blocked" ? (
                    <span className="trackNote">{state.reason}</span>
                  ) : null}

                  {state.kind === "tracked" ? (
                    <button
                      className="trackButton"
                      disabled={busy}
                      onClick={() => {
                        setJustTracked((current) =>
                          current.filter((id) => id !== repository.id),
                        );
                        run(repository.id, () =>
                          untrackRepository(organizationId, state.repositoryId),
                        );
                      }}
                    >
                      {busy ? "Removing" : "Untrack"}
                    </button>
                  ) : (
                    <button
                      className="trackButton primary"
                      disabled={busy || state.kind === "blocked"}
                      onClick={() => {
                        setJustTracked((current) => [...current, repository.id]);
                        run(repository.id, () =>
                          trackRepository(organizationId, repository.id),
                        );
                      }}
                    >
                      {busy ? "Tracking" : "Track"}
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}
