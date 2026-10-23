/**
 * Regression test for pagedStream's prefetch: a consumer that stops early never
 * awaits the prefetched page, so its rejection must be handled by the stream.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { pagedStream } from "./stream_manager";

describe("pagedStream", () => {
  const unhandledRejection = vi.fn();

  afterEach(() => {
    process.off("unhandledRejection", unhandledRejection);
    unhandledRejection.mockReset();
  });

  it("a consumer that breaks after the first page leaves no unhandled rejection from the prefetched page", async () => {
    process.on("unhandledRejection", unhandledRejection);
    const fetchPage = vi.fn(async (page: number) => {
      if (page === 1) {
        return ["first"];
      }

      throw new Error("page 2 failed");
    });
    const stream = pagedStream(fetchPage);

    const first = await stream.next();
    // What `break` in a `for await` loop does.
    await stream.return(undefined);
    // Node reports unhandled rejections once the microtask queue drains.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(first).toEqual({ value: ["first"], done: false });
    expect(fetchPage.mock.calls).toEqual([[1], [2]]);
    expect(unhandledRejection).not.toHaveBeenCalled();
  });
});
