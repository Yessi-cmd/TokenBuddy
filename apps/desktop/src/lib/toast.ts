// Transient action feedback ("扫描完成…", "已导出…"). These used to overwrite
// the toolbar status pill, which truncated long messages and hid the data-layer
// state behind the result of the last click. Persistent failures still belong
// in a notice; a toast is only for "this action finished".

import { create } from "zustand";

export type ToastTone = "success" | "info" | "warning";

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
}

interface ToastState {
  toasts: Toast[];
  push: (message: string, tone?: ToastTone) => void;
  dismiss: (id: number) => void;
  clear: () => void;
}

const TOAST_LIFETIME_MS = 5200;
let nextId = 1;

export const useToasts = create<ToastState>((set, get) => ({
  toasts: [],
  push: (message, tone = "success") => {
    const id = nextId++;
    // Newest last; keep the stack short so a burst of actions cannot bury the page.
    set({ toasts: [...get().toasts.slice(-3), { id, tone, message }] });
    setTimeout(() => get().dismiss(id), TOAST_LIFETIME_MS);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
  clear: () => set({ toasts: [] }),
}));

export function toast(message: string, tone?: ToastTone) {
  useToasts.getState().push(message, tone);
}
