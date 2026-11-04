import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useAsyncData } from "@/lib/use-async-data";

describe("useAsyncData", () => {
  it("exposes resolved data and clears loading", async () => {
    const loader = vi.fn(async () => "store");
    const { result } = renderHook(() => useAsyncData(loader, []));

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.data).toBe("store");
    expect(result.current.error).toBeNull();
  });

  it("surfaces loader errors", async () => {
    const loader = vi.fn(async () => {
      throw new Error("Could not reach the store");
    });
    const { result } = renderHook(() => useAsyncData(loader, []));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe("Could not reach the store");
    expect(result.current.data).toBeNull();
  });
});
