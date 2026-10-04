import { useEffect } from "react";

import { useToasts } from "../lib/toast";

export function Toaster() {
  const toasts = useToasts((state) => state.toasts);
  const dismiss = useToasts((state) => state.dismiss);

  // Toasts describe actions taken in this panel; a fresh mount starts clean.
  useEffect(() => () => useToasts.getState().clear(), []);

  return (
    <div className="toaster" role="status" aria-live="polite">
      {toasts.map((item) => (
        <div className={`toast toast-${item.tone}`} key={item.id}>
          <span className="toast-icon" aria-hidden="true" />
          <span className="toast-message">{item.message}</span>
          <button
            className="toast-close"
            type="button"
            aria-label="关闭提示"
            onClick={() => dismiss(item.id)}
          >
            ×
          </button>
          <span className="toast-timer" aria-hidden="true" />
        </div>
      ))}
    </div>
  );
}
