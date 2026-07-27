import "@testing-library/jest-dom/vitest";

// jsdom does not implement ResizeObserver, which the tray popover uses to keep
// its window as tall as its content. A minimal stub is enough here: layout in
// jsdom never changes on its own, so observing is a no-op and only the initial
// measurement matters.
if (!("ResizeObserver" in globalThis)) {
  class ResizeObserverStub {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  Object.defineProperty(globalThis, "ResizeObserver", {
    value: ResizeObserverStub,
    writable: true,
  });
}
