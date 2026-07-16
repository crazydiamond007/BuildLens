export function formatDuration(milliseconds: number | null): string {
  if (milliseconds === null) return "No data";
  if (milliseconds < 1000) return `${milliseconds} ms`;
  const seconds = Math.round(milliseconds / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  if (minutes < 60) return `${minutes}m ${remainder}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export function formatSeconds(seconds: number | null): string {
  return seconds === null ? "No data" : formatDuration(seconds * 1000);
}

export function formatPercent(value: number | null): string {
  if (value === null) return "No data";
  return `${(value * 100).toFixed(value < 0.1 ? 1 : 0)}%`;
}

export function formatScore(value: number | null): string {
  return value === null ? "-" : value.toFixed(0);
}

export function formatDate(value: string | null): string {
  if (!value) return "No data";
  return new Intl.DateTimeFormat("en", {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(value));
}

export function formatDateTime(value: string | null): string {
  if (!value) return "No data";
  return new Intl.DateTimeFormat("en", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

export function relativeTime(value: string | null): string {
  if (!value) return "Never";
  const delta = Date.now() - new Date(value).getTime();
  const minutes = Math.round(delta / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

export function sentenceCase(value: string | null): string {
  if (!value) return "Unknown";
  return value.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
}
