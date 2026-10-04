import { isDesktopRuntime } from "./api";

async function panelVisible(): Promise<boolean> {
  if (document.hidden) return false;
  if (!isDesktopRuntime()) return true;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const window = getCurrentWindow();
  return (await window.isVisible()) && !(await window.isMinimized());
}

/** Completion-based polling: never overlap requests, and do no data queries while hidden. */
export function startPolling(
  task: () => Promise<void>,
  interval: number,
  visible: () => Promise<boolean> = panelVisible,
): () => void {
  let stopped = false;
  let running = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const tick = async () => {
    if (stopped || running) return;
    clearTimeout(timer);
    running = true;
    try {
      if ((await visible()) && !stopped) await task();
    } catch (error) {
      console.error("面板刷新失败", error);
    } finally {
      running = false;
      if (!stopped) timer = setTimeout(() => void tick(), interval);
    }
  };
  const wake = () => void tick();
  document.addEventListener("visibilitychange", wake);
  window.addEventListener("focus", wake);
  void tick();
  return () => {
    stopped = true;
    clearTimeout(timer);
    document.removeEventListener("visibilitychange", wake);
    window.removeEventListener("focus", wake);
  };
}
