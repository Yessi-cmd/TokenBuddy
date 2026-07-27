import type { ReactNode } from "react";

import { navigate } from "../lib/navigation";

export function AppNavigation() {
  return (
    <nav className="route-nav" aria-label="主要导航">
      <RouteLink to="/dashboard">总览</RouteLink>
      <RouteLink to="/sessions">会话</RouteLink>
      <RouteLink to="/providers">Providers</RouteLink>
      <RouteLink to="/quotas">额度</RouteLink>
      <RouteLink to="/sources">数据源</RouteLink>
      <RouteLink to="/settings">设置</RouteLink>
    </nav>
  );
}

export function RouteLink({ to, children }: { to: string; children: string }) {
  return (
    <a
      href={to}
      onClick={(event) => {
        if (event.button !== 0 || event.metaKey || event.ctrlKey) return;
        event.preventDefault();
        navigate(to);
      }}
    >
      {children}
    </a>
  );
}

export function PageFrame({
  eyebrow,
  title,
  subtitle,
  children,
}: {
  eyebrow: string;
  title: string;
  subtitle: string;
  children: ReactNode;
}) {
  return (
    <main className="app-shell">
      <header className="page-header">
        <div>
          <p className="eyebrow">{eyebrow}</p>
          <h1>{title}</h1>
          <p className="subtitle">{subtitle}</p>
        </div>
        <AppNavigation />
      </header>
      {children}
    </main>
  );
}
