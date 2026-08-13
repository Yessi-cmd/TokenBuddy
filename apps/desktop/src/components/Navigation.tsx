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

export function PageFrame({ children }: { children: ReactNode }) {
  return (
    <main className="app-shell">
      <AppNavigation />
      {children}
    </main>
  );
}
