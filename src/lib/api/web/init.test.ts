// @vitest-environment jsdom
import { describe, it, expect, vi } from "vitest";
import { getContainer } from "../dependency_injection";
import { IBlobApi, type Blob } from "../shared/blob_api";
import { initWebApi } from "./init";

describe("initWebApi", () => {
  // The mobile client passes the remote server URL; blob URLs must point there,
  // not at the webview origin.
  it("builds blob download URLs against the given server URL", async () => {
    const fetchImplementation = vi.fn().mockImplementation(
      async () =>
        new Response(
          JSON.stringify({
            id: 2,
            username: "user",
            email: "user@example.com",
            created_at: "2026-01-01T00:00:00Z",
            permissions: [],
            sync_url: "hash",
            sync_token: null,
            last_sync: null,
          }),
          { status: 200 },
        ),
    );

    await initWebApi("https://server.example", fetchImplementation);

    const blob = getContainer().require<IBlobApi>(IBlobApi);
    const url = await blob.getBlobDownloadUrl({ sha256: "abc" } as Blob);

    expect(url).toBe(
      "https://server.example/api/v1/blobs/abc/download?user_hash=hash&user_id=2",
    );
  });
});
