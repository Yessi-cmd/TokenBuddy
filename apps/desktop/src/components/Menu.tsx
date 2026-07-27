// The tray popover's building blocks. Kept apart from the panel's components:
// this surface mimics a native menu, not the web dashboard.

import type { ReactNode } from "react";

export function MenuSeparator() {
  return <div className="menu-separator" role="separator" />;
}

export function MenuGroupTitle({ children }: { children: string }) {
  return <p className="menu-group-title">{children}</p>;
}

export function MenuValueRow({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="menu-row menu-row-compact">
      <span className="menu-row-label">{label}</span>
      <span className="menu-value">{value}</span>
    </div>
  );
}

export type MenuGlyphName = "chart" | "session" | "gauge";

export function MenuRow({
  glyph,
  tint,
  label,
  sublabel,
  trailing,
}: {
  glyph: MenuGlyphName;
  tint: "blue" | "mint" | "amber";
  label: string;
  sublabel?: string;
  trailing?: ReactNode;
}) {
  return (
    <div className="menu-row">
      <span className={`menu-glyph menu-glyph-${tint}`} aria-hidden="true">
        <MenuGlyph name={glyph} />
      </span>
      <span className="menu-row-body">
        <span className="menu-row-label">{label}</span>
        {sublabel ? <span className="menu-row-sub">{sublabel}</span> : null}
      </span>
      {trailing ? <span className="menu-row-trailing">{trailing}</span> : null}
    </div>
  );
}

export function MenuGlyph({ name }: { name: MenuGlyphName }) {
  if (name === "chart") {
    return (
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path
          d="M2.5 13.5h11M4.75 11V7.5M8 11V4.5M11.25 11V8.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  if (name === "session") {
    return (
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path
          d="M3 3.5h10a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H7l-3 2.4V10.5H3a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1Z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinejoin="round"
        />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        d="M3 12a5 5 0 1 1 10 0M8 8.5l2.6-2.6"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
