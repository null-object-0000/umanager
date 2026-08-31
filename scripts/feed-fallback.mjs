// Pure helper for the "never silently drop" feed policy.
//
// The CI feed generator scrapes every vendor source best-effort. When a source
// fails this run but was present in the previous published feed, we reuse its
// last known-good entry rather than leaving it out of the feed (which would
// make the desktop app report "元数据源中缺少 … 的最新版本信息"). Extracted so
// the decision can be unit-tested independently.

/**
 * @template T
 * @param {T|null|undefined} fresh    the entry scraped this run, if successful
 * @param {T|null|undefined} previous the entry from the previous published feed
 * @returns {T|null} `fresh` when available, else a shallow copy of `previous`,
 *                   else null.
 */
export function entryOrPrevious(fresh, previous) {
  if (fresh) return fresh;
  return previous ? { ...previous } : null;
}
