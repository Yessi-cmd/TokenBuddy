// Path-based routing without a router library.
//
// The desktop shell opens windows at a path (`/quick`, `/dashboard`), so the
// panel only needs to read `location.pathname` and re-render when it changes.

import { useEffect, useState } from "react";

export function usePathname(): string {
  const [pathname, setPathname] = useState(() => window.location.pathname);
  useEffect(() => {
    const handlePopState = () => setPathname(window.location.pathname);
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);
  return pathname;
}

export function navigate(path: string) {
  window.history.pushState({}, "", path);
  window.dispatchEvent(new PopStateEvent("popstate"));
}
