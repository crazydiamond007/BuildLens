"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";

import { signOut } from "@/lib/actions";
import type { Membership } from "@/lib/types";

type Props = {
  membership: Membership;
  memberships: Membership[];
  user: { name: string | null; email: string; avatarUrl: string | null };
  children: React.ReactNode;
};

const navItems = [
  ["Overview", "overview", "O"],
  ["Repositories", "repositories", "R"],
  ["Workflow runs", "runs", "W"],
  ["DORA metrics", "dora", "D"],
  ["Flaky tests", "flaky-tests", "F"],
  ["AI inbox", "ai-inbox", "A"],
] as const;

const workspaceItems = [["Repository tracking", "settings", "S"]] as const;

// The mobile bottom bar carries the four most-visited destinations; everything
// else (Repositories, AI inbox, Repository tracking) lives behind "More", which
// reuses the same slide-in drawer the desktop sidebar becomes on narrow screens.
const bottomTabs = [
  ["Overview", "overview", "grid"],
  ["Runs", "runs", "activity"],
  ["DORA", "dora", "gauge"],
  ["Flaky", "flaky-tests", "alert"],
] as const;

const TAB_ICONS: Record<string, React.ReactNode> = {
  grid: (
    <>
      <rect x="3" y="3" width="7" height="7" rx="1.5" />
      <rect x="14" y="3" width="7" height="7" rx="1.5" />
      <rect x="3" y="14" width="7" height="7" rx="1.5" />
      <rect x="14" y="14" width="7" height="7" rx="1.5" />
    </>
  ),
  activity: <polyline points="3 12 7 12 10 4 14 20 17 12 21 12" />,
  gauge: (
    <>
      <path d="M4 18a8 8 0 0 1 16 0" />
      <line x1="12" y1="18" x2="15.5" y2="12.5" />
    </>
  ),
  alert: (
    <>
      <path d="M12 3 2.5 20h19L12 3Z" />
      <line x1="12" y1="10" x2="12" y2="14" />
      <line x1="12" y1="16.7" x2="12" y2="16.8" />
    </>
  ),
  more: (
    <>
      <circle cx="5" cy="12" r="1.6" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
      <circle cx="19" cy="12" r="1.6" fill="currentColor" stroke="none" />
    </>
  ),
};

function TabIcon({ name }: { name: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {TAB_ICONS[name]}
    </svg>
  );
}

export function AppShell({ membership, memberships, user, children }: Props) {
  const pathname = usePathname();
  const router = useRouter();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const base = `/organizations/${membership.id}`;
  const routes = useMemo(
    () => navItems.map(([label, segment, icon]) => ({ label, icon, href: `${base}/${segment}` })),
    [base],
  );
  const workspaceRoutes = useMemo(
    () => workspaceItems.map(([label, segment, icon]) => ({ label, icon, href: `${base}/${segment}` })),
    [base],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
      if (event.key === "Escape") setPaletteOpen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  function toggleTheme() {
    const next = document.documentElement.dataset.theme === "light" ? "dark" : "light";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("buildlens-theme", next);
  }

  function renderLink(route: { label: string; icon: string; href: string }) {
    const active = pathname === route.href || pathname.startsWith(`${route.href}/`);
    return (
      <Link
        key={route.href}
        href={route.href}
        className={active ? "active" : ""}
        onClick={() => setSidebarOpen(false)}
      >
        <span className="navIcon">{route.icon}</span>
        {route.label}
      </Link>
    );
  }

  const initials = (user.name ?? user.email)
    .split(/[ @._-]/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");

  return (
    <div className="appShell">
      <button
        className={`sidebarBackdrop ${sidebarOpen ? "visible" : ""}`}
        aria-label="Close navigation"
        onClick={() => setSidebarOpen(false)}
      />
      <aside className={`sidebar ${sidebarOpen ? "open" : ""}`}>
        <Link href={`${base}/overview`} className="brand" onClick={() => setSidebarOpen(false)}>
          <span className="brandMark"><i /><i /><i /></span>
          <span>BuildLens</span>
        </Link>

        <button className="commandButton" onClick={() => setPaletteOpen(true)}>
          <span className="mono">&gt;</span>
          Search
          <kbd>Ctrl K</kbd>
        </button>

        <p className="navLabel">Analytics</p>
        <nav className="navList" aria-label="Primary navigation">
          {routes.map(renderLink)}
        </nav>

        <p className="navLabel spaced">Workspace</p>
        <nav className="navList" aria-label="Workspace navigation">
          {workspaceRoutes.map(renderLink)}
        </nav>

        <div className="sidebarUser">
          <span className="avatar">{initials || "BL"}</span>
          <span className="sidebarUserText">
            <strong>{user.name ?? user.email.split("@")[0]}</strong>
            <small>{membership.role}</small>
          </span>
        </div>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <button className="mobileMenu" aria-label="Open navigation" onClick={() => setSidebarOpen(true)}>
            Menu
          </button>
          <select
            className="organizationSelect"
            value={membership.id}
            aria-label="Organization"
            onChange={(event) => router.push(`/organizations/${event.target.value}/overview`)}
          >
            {memberships.map((item) => (
              <option key={item.id} value={item.id}>{item.name}</option>
            ))}
          </select>
          <span className="topbarDivider" />
          <span className="topbarContext">{membership.kind} organization</span>
          <div className="topbarActions">
            <button className="iconButton" onClick={toggleTheme} aria-label="Toggle color theme">Theme</button>
            <form action={signOut}>
              <button className="iconButton" type="submit">Sign out</button>
            </form>
          </div>
        </header>
        <main className="mainContent">{children}</main>
      </div>

      <nav className="bottomNav" aria-label="Primary navigation">
        {bottomTabs.map(([label, segment, icon]) => {
          const href = `${base}/${segment}`;
          const active = pathname === href || pathname.startsWith(`${href}/`);
          return (
            <Link key={segment} href={href} className={active ? "active" : ""}>
              <TabIcon name={icon} />
              <span>{label}</span>
            </Link>
          );
        })}
        <button type="button" onClick={() => setSidebarOpen(true)} aria-label="More navigation">
          <TabIcon name="more" />
          <span>More</span>
        </button>
      </nav>

      {paletteOpen && (
        <div className="commandOverlay" role="presentation" onMouseDown={() => setPaletteOpen(false)}>
          <div className="commandPalette" role="dialog" aria-modal="true" aria-label="Quick navigation" onMouseDown={(event) => event.stopPropagation()}>
            <div className="commandInput"><span>&gt;</span><span>Quick navigation</span><kbd>esc</kbd></div>
            <p className="navLabel">Navigate</p>
            {[...routes, ...workspaceRoutes].map((route) => (
              <button key={route.href} onClick={() => { router.push(route.href); setPaletteOpen(false); }}>
                <span className="navIcon">{route.icon}</span>{route.label}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
