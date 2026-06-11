// Vitest setup, loaded once per test file (see vitest.config.ts).
// Registers the jest-dom matchers (toBeInTheDocument, …) for future
// component tests; pure-logic tests just ignore them.
import "@testing-library/jest-dom/vitest";

// Node ≥22 ships an experimental `localStorage` global that throws
// SecurityError unless node runs with --localstorage-file; under vitest
// it shadows jsdom's implementation (and `window` IS `globalThis` here,
// so aliasing recurses). Install a plain in-memory Storage so module
// code using bare `localStorage` (e.g. pdf/exportOptions.ts) gets real
// Storage semantics in tests.
class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get length(): number {
    return this.map.size;
  }
  clear(): void {
    this.map.clear();
  }
  getItem(key: string): string | null {
    return this.map.get(key) ?? null;
  }
  key(index: number): string | null {
    return [...this.map.keys()][index] ?? null;
  }
  removeItem(key: string): void {
    this.map.delete(key);
  }
  setItem(key: string, value: string): void {
    this.map.set(key, String(value));
  }
}

Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: new MemoryStorage(),
});
