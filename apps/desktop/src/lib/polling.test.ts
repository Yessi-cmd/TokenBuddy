import { afterEach, expect, it, vi } from "vitest";
import { startPolling } from "./polling";

afterEach(() => vi.useRealTimers());

it("waits for an outstanding request and stops on cleanup", async () => {
  vi.useFakeTimers();
  let resolve!: () => void;
  const task = vi.fn(
    () =>
      new Promise<void>((done) => {
        resolve = done;
      }),
  );
  const stop = startPolling(task, 100, async () => true);
  await vi.advanceTimersByTimeAsync(1000);
  expect(task).toHaveBeenCalledTimes(1);
  resolve();
  await vi.advanceTimersByTimeAsync(100);
  expect(task).toHaveBeenCalledTimes(2);
  stop();
  resolve();
  await vi.advanceTimersByTimeAsync(1000);
  expect(task).toHaveBeenCalledTimes(2);
});

it("skips hidden windows and refreshes after becoming visible", async () => {
  vi.useFakeTimers();
  let visible = false;
  const task = vi.fn(async () => {});
  const stop = startPolling(task, 100, async () => visible);
  await vi.advanceTimersByTimeAsync(500);
  expect(task).not.toHaveBeenCalled();
  visible = true;
  document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(0);
  expect(task).toHaveBeenCalledTimes(1);
  stop();
  document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(500);
  expect(task).toHaveBeenCalledTimes(1);
});
