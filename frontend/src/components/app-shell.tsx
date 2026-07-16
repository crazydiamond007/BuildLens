"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";

import type { Membership } from "@/lib/types";

type Props = {
  membership: Membership;
  memberships: Membership[];
  user: { name: string | null; email: string; avatarUrl: string | null };
  logoutUrl: string;
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

export function AppShell({ membership, memberships, user, logoutUrl, children }: Props) {
  const pathname = usePathname();
  const router = useRouter();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const base = `/organizations/${membership.id}`;
  const routes = useMemo(
    () => navItems.map(([label, segment, icon]) => ({ label, icon, href: `${base}/${segment}` })),
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
          {routes.map((route) => {
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
          })}
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
            <a className="iconButton" href={logoutUrl}>Sign out</a>
          </div>
        </header>
        <main className="mainContent">{children}</main>
      </div>

      {paletteOpen && (
        <div className="commandOverlay" role="presentation" onMouseDown={() => setPaletteOpen(false)}>
          <div className="commandPalette" role="dialog" aria-modal="true" aria-label="Quick navigation" onMouseDown={(event) => event.stopPropagation()}>
            <div className="commandInput"><span>&gt;</span><span>Quick navigation</span><kbd>esc</kbd></div>
            <p className="navLabel">Navigate</p>
            {routes.map((route) => (
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
