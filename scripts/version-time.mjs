// UManager feed version-update-time helpers.
//
// The CI feed generator computes, for each app/tool, a best-effort "current
// version published at" timestamp (and its source). These pure functions are
// extracted here so they can be unit-tested independently. See
// DESIGN-version-updated-at.md for the full spec.

/**
 * Parse a value into integer unix seconds.
 * - Number: finite, > 0 -> floor
 * - String: ISO 8601 (Date.parse) or numeric string -> unix seconds
 * - anything else / failure -> null
 */
export function parseUnixSeconds(value) {
  if (value == null) return null;
  if (typeof value === "number") {
    return Number.isFinite(value) && value > 0 ? Math.floor(value) : null;
  }
  if (typeof value === "string") {
    const parsedDate = Date.parse(value);
    if (!Number.isNaN(parsedDate)) return Math.floor(parsedDate / 1000);
    const numeric = Number(value);
    if (Number.isFinite(numeric) && numeric > 0) return Math.floor(numeric);
  }
  return null;
}

/**
 * Parse an HTTP "Last-Modified" header value ("Wed, 21 Oct 2015 07:28:00 GMT")
 * into integer unix seconds, or null on failure.
 */
export function parseLastModified(headerValue) {
  if (!headerValue) return null;
  const parsed = Date.parse(headerValue);
  return Number.isNaN(parsed) ? null : Math.floor(parsed / 1000);
}

/**
 * Merge the previous feed state with this run's candidate to decide the
 * current version-update timestamp + source.
 *
 * @param {{version:string, versionUpdatedAtUnixSeconds?:number|null, versionUpdatedAtSource?:string|null}|null|undefined} previous
 * @param {string} version       current authoritative version (Debian control)
 * @param {{time:number, source:"official"|"serverModified"|"observed"}|null} candidate
 * @param {number} now           current unix seconds (this scrape)
 * @returns {{time:number, source:string}|null}
 */
export function mergeVersionUpdatedAt(previous, version, candidate, now) {
  if (!candidate) {
    if (!previous) return null; // first scrape: no baseline
    if (previous.version !== version) {
      return { time: now, source: "observed" }; // detected an upgrade
    }
    // unchanged; carry the previous value if it was set, else null
    if (previous.versionUpdatedAtUnixSeconds != null && previous.versionUpdatedAtSource) {
      return {
        time: previous.versionUpdatedAtUnixSeconds,
        source: previous.versionUpdatedAtSource,
      };
    }
    return null;
  }
  if (candidate.source === "official") return candidate; // authoritative, always adopt
  // serverModified: adopt on first sight / version change / previous was observed;
  // otherwise keep the previous value to avoid repackaging churn.
  if (!previous || previous.version !== version) return candidate;
  if (!previous.versionUpdatedAtUnixSeconds || previous.versionUpdatedAtSource === "observed") {
    return candidate;
  }
  return {
    time: previous.versionUpdatedAtUnixSeconds,
    source: previous.versionUpdatedAtSource,
  };
}
