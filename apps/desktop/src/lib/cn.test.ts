import { describe, expect, it } from "vitest";

import { cn } from "./cn";

describe("cn", () => {
  it("joins truthy class fragments", () => {
    expect(cn("a", "b")).toBe("a b");
  });

  it("drops falsy fragments", () => {
    expect(cn("a", false && "b", undefined, null, "c")).toBe("a c");
  });

  it("flattens arrays and conditional objects like clsx", () => {
    expect(cn(["a", { b: true, c: false }])).toBe("a b");
  });
});
