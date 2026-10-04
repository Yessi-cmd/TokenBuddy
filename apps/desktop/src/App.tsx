import { useEffect } from "react";

import { Toaster } from "./components/Toaster";
import { DashboardView } from "./features/dashboard/DashboardView";
import { ProvidersView } from "./features/providers/ProvidersView";
import { QuotasView } from "./features/quotas/QuotasView";
import { QuickSummaryView } from "./features/quick/QuickSummaryView";
import { SessionRouteView } from "./features/sessions/SessionRouteView";
import { SessionsView } from "./features/sessions/SessionsView";
import { SettingsView } from "./features/settings/SettingsView";
import { SourcesView } from "./features/sources/SourcesView";
import { installPointerGlow } from "./lib/motion";
import { usePathname } from "./lib/navigation";

function PanelRoute({ pathname }: { pathname: string }) {
  if (pathname === "/providers") return <ProvidersView />;
  if (pathname === "/quotas") return <QuotasView />;
  if (pathname === "/settings") return <SettingsView />;
  if (pathname === "/sources") return <SourcesView />;
  if (pathname === "/sessions") return <SessionsView />;
  if (pathname.startsWith("/sessions/")) {
    return (
      <SessionRouteView
        sessionId={decodeURIComponent(pathname.slice("/sessions/".length))}
      />
    );
  }
  return <DashboardView />;
}

function PanelApp({ pathname }: { pathname: string }) {
  useEffect(() => installPointerGlow(), []);
  return (
    <>
      <div className="backdrop" aria-hidden="true" />
      <PanelRoute pathname={pathname} />
      <Toaster />
    </>
  );
}

function App() {
  const pathname = usePathname();
  if (pathname === "/quick") return <QuickSummaryView />;
  return <PanelApp pathname={pathname} />;
}

export default App;
