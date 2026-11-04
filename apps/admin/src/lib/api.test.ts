import { ApiError } from "@fastcart/api-client";
import { describe, expect, it, vi } from "vitest";

import { errorMessage, fetchAll } from "@/lib/api";
import { page } from "@/test/fixtures";

describe("fetchAll", () => {
  it("walks every page until the reported total is reached", async () => {
    const fetchPage = vi.fn(async ({ offset }: { offset?: number }) => {
      const data = offset === 0 ? ["a", "b"] : ["c"];
      return page(data, 3);
    });

    const records = await fetchAll(fetchPage, 10);
    expect(records).toEqual(["a", "b", "c"]);
    expect(fetchPage).toHaveBeenNthCalledWith(1, { limit: 10, offset: 0 });
    expect(fetchPage).toHaveBeenNthCalledWith(2, { limit: 8, offset: 2 });
  });

  it("caps the number of records read", async () => {
    const fetchPage = vi.fn(async ({ limit = 0, offset = 0 }) =>
      page(
        Array.from({ length: limit }, (_, index) => `row-${offset + index}`),
        1_000,
      ),
    );

    const records = await fetchAll(fetchPage, 3);
    expect(records).toHaveLength(3);
  });
});

describe("errorMessage", () => {
  it("maps authentication and permission failures to operator copy", () => {
    expect(errorMessage(new ApiError(401, "unauthorized", "expired", null))).toBe(
      "Your session has expired. Sign in again.",
    );
    expect(errorMessage(new ApiError(403, "forbidden", "forbidden", null))).toBe(
      "You do not have permission to do that.",
    );
    expect(errorMessage(new ApiError(429, "rate_limited", "slow down", null))).toBe(
      "Too many attempts. Wait a moment and try again.",
    );
  });

  it("keeps server validation messages", () => {
    expect(errorMessage(new ApiError(422, "validation", "title must not be empty", null))).toBe(
      "title must not be empty",
    );
    expect(errorMessage(new Error("Plain failure"))).toBe("Plain failure");
  });
});
