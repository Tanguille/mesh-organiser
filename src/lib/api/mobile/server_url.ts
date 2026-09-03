/**
 * Normalise a remote mesh-organiser server base URL for the mobile / web client.
 * Behaviour aligns with `src-tauri/src/commands/server_url.rs` (`normalize_remote_server_url`).
 */
const CONTROL_RE = /\p{Cc}/u;

function schemePrefixLen(withoutTrailing: string): number | null {
  if (
    withoutTrailing.length >= 8 &&
    withoutTrailing.slice(0, 8).toLowerCase() === "https://"
  ) {
    return 8;
  }

  if (
    withoutTrailing.length >= 7 &&
    withoutTrailing.slice(0, 7).toLowerCase() === "http://"
  ) {
    return 7;
  }

  return null;
}

/**
 * Trim, strip trailing slashes, and validate `http://` or `https://` with a non-empty host.
 */
export function normalizeServerBaseUrl(url: string): string {
  const trimmed = url.trim();

  if (trimmed.length === 0) {
    throw new Error("URL cannot be empty");
  }

  if (CONTROL_RE.test(trimmed)) {
    throw new Error("URL must not contain control characters");
  }

  const withoutTrailing = trimmed.replace(/\/+$/, "");
  const schemeLen = schemePrefixLen(withoutTrailing);

  if (schemeLen === null) {
    throw new Error("URL must start with http:// or https://");
  }

  const afterScheme = withoutTrailing.slice(schemeLen);

  if ([...afterScheme].some((ch) => /\s/u.test(ch))) {
    throw new Error("URL must not contain whitespace in the host or path");
  }

  if (afterScheme === "" || afterScheme.startsWith("/")) {
    throw new Error("URL must include a host (e.g. https://example.com)");
  }

  return withoutTrailing;
}
