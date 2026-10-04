import { useLayoutEffect, useRef, type ReactNode } from "react";

import { navigate, usePathname } from "../lib/navigation";

const routes = [
  ["/dashboard", "总览"],
  ["/sessions", "会话"],
  ["/providers", "Providers"],
  ["/quotas", "额度"],
  ["/sources", "数据源"],
  ["/settings", "设置"],
] as const;

function isActive(route: string, pathname: string) {
  if (route === "/dashboard") return pathname === "/" || pathname === route;
  return pathname === route || pathname.startsWith(`${route}/`);
}

// Every route mounts its own frame, so the indicator remembers where it was on
// the previous page and glides from there instead of popping into place.
let lastIndicator: { left: number; width: number } | null = null;

export function AppNavigation() {
  const pathname = usePathname();
  const navRef = useRef<HTMLElement | null>(null);
  const indicatorRef = useRef<HTMLSpanElement | null>(null);

  useLayoutEffect(() => {
    const nav = navRef.current;
    const indicator = indicatorRef.current;
    if (!nav || !indicator) return;

    const place = (animate: boolean) => {
      const active = nav.querySelector<HTMLElement>('a[aria-current="page"]');
      if (!active) {
        indicator.style.opacity = "0";
        return;
      }
      const next = { left: active.offsetLeft, width: active.offsetWidth };
      indicator.style.transition = animate ? "" : "none";
      indicator.style.opacity = "1";
      indicator.style.transform = `translateX(${next.left}px)`;
      indicator.style.width = `${next.width}px`;
      lastIndicator = next;
    };

    if (lastIndicator) {
      indicator.style.transition = "none";
      indicator.style.transform = `translateX(${lastIndicator.left}px)`;
      indicator.style.width = `${lastIndicator.width}px`;
      indicator.style.opacity = "1";
      // Force the start position to commit before sliding to the new one.
      void indicator.offsetWidth;
      place(true);
    } else {
      place(false);
    }

    const onResize = () => place(false);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [pathname]);

  return (
    <nav className="route-nav" aria-label="主要导航" ref={navRef}>
      <span className="brand" aria-hidden="true">
        <BrandMark />
        <span className="brand-name">TokenBuddy</span>
      </span>
      <span className="route-nav-links">
        <span className="route-indicator" ref={indicatorRef} />
        {routes.map(([to, label]) => (
          <RouteLink key={to} to={to} active={isActive(to, pathname)}>
            {label}
          </RouteLink>
        ))}
      </span>
    </nav>
  );
}

export function RouteLink({
  to,
  active,
  children,
}: {
  to: string;
  active?: boolean;
  children: ReactNode;
}) {
  return (
    <a
      href={to}
      aria-current={active ? "page" : undefined}
      onClick={(event) => {
        if (event.button !== 0 || event.metaKey || event.ctrlKey) return;
        event.preventDefault();
        if (window.location.pathname !== to) navigate(to);
      }}
    >
      {children}
    </a>
  );
}

function BrandMark() {
  return (
    <svg className="brand-mark" viewBox="0 0 24 24" width="20" height="20">
      <defs>
        <linearGradient id="brand-gradient" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="var(--accent)" />
          <stop offset="1" stopColor="var(--accent-2)" />
        </linearGradient>
      </defs>
      <circle
        cx="12"
        cy="12"
        r="9.5"
        fill="none"
        stroke="url(#brand-gradient)"
        strokeWidth="1.6"
        strokeDasharray="44 16"
        className="brand-orbit"
      />
      <path
        d="M7.5 14.5 10.5 11l2.5 2.5 3.5-5"
        fill="none"
        stroke="url(#brand-gradient)"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * The shared page chrome: navigation always first in the toolbar, then the
 * page's own actions on the right. `busy` draws an indeterminate progress line
 * under the toolbar for long-running actions.
 */
export function PageFrame({
  children,
  actions,
  label = "页面工具栏",
  busy = false,
}: {
  children: ReactNode;
  actions?: ReactNode;
  label?: string;
  busy?: boolean;
}) {
  return (
    <main className="app-shell">
      <header className="app-topbar" aria-label={label}>
        <AppNavigation />
        {actions ? <div className="topbar-actions">{actions}</div> : null}
        <span
          className="topbar-progress"
          data-active={busy || undefined}
          aria-hidden="true"
        />
      </header>
      {children}
    </main>
  );
}
