// Pure helpers for the multi-source feed generation split.
//
// The CI feed generator scrapes vendors in parallel per source group (tencent,
// common, …). Each group job produces a final, signed source feed
// (`feed.<group>.json`, schema v2 with that group's applications + its own
// signed catalog); the merge step aggregates every source feed into the
// central `feed.json` (canonical app order, full catalog, fallback to the
// previous central feed for anything a source could not provide). Everything
// here is pure (no network, no fs) so the grouping and merge rules can be
// unit-tested independently. See DESIGN-multi-source.md §7.

import { entryOrPrevious } from "./feed-fallback.mjs";
import { mergeVersionUpdatedAt } from "./version-time.mjs";

/**
 * Resolve the source group an application belongs to.
 * Precedence: the app's own `sourceGroup` (explicit, recommended) → the
 * script-side built-in mapping (vendors.json apps, e.g. wechat/wemeet) →
 * the default group "common".
 *
 * @param {object} app            application object (built-in or feed-sources)
 * @param {object} builtinGroups  { [sourceId]: string[] } for built-in apps
 * @param {string} [defaultGroup] default group id, "common"
 * @returns {string} source group id
 */
export function sourceGroupOf(app, builtinGroups = {}, defaultGroup = "common") {
  if (app && typeof app.sourceGroup === "string" && app.sourceGroup) {
    return app.sourceGroup;
  }
  if (app?.applicationId && builtinGroups) {
    for (const [groupId, ids] of Object.entries(builtinGroups)) {
      if (ids.includes(app.applicationId)) return groupId;
    }
  }
  return defaultGroup;
}

/**
 * Validate every source-group assignment against the known registry.
 *
 * @param {object[]} apps        all applications (built-in + extra), in order
 * @param {object} builtinGroups { [sourceId]: string[] } built-in mapping
 * @param {string[]} knownGroups registered group ids
 * @returns {string[]} problem descriptions (empty when everything is valid)
 */
export function validateSourceGroups(apps, builtinGroups, knownGroups) {
  const known = new Set(knownGroups);
  const problems = [];
  for (const app of apps) {
    const group = sourceGroupOf(app, builtinGroups);
    if (!known.has(group)) {
      problems.push(`应用 ${app.applicationId}: 未注册的 sourceGroup「${group}」`);
    }
  }
  return problems;
}

/**
 * Parse a signed catalog blob (a JSON array of application records) into a
 * map keyed by applicationId. Returns null when the blob is invalid.
 *
 * @param {string|null|undefined} catalogJson feed.catalogJson
 * @returns {Object<string, object>|null}
 */
export function parseCatalogApplications(catalogJson) {
  if (!catalogJson) return null;
  try {
    const list = JSON.parse(catalogJson);
    if (!Array.isArray(list)) return null;
    const map = {};
    for (const record of list) {
      if (record && typeof record === "object" && record.applicationId) {
        map[record.applicationId] = record;
      }
    }
    return map;
  } catch {
    return null;
  }
}

/**
 * Finalize an entry's version-update time against the previous state and
 * strip the transient `_versionTimeCandidate`. Never throws.
 */
export function applyVersionTime(entry, previousEntry, nowUnixSeconds) {
  if (!entry) return;
  const candidate = entry._versionTimeCandidate || null;
  const merged = mergeVersionUpdatedAt(
    previousEntry ?? null,
    entry.version,
    candidate,
    nowUnixSeconds,
  );
  entry.versionUpdatedAtUnixSeconds = merged?.time ?? null;
  entry.versionUpdatedAtSource = merged?.source ?? null;
  delete entry._versionTimeCandidate;
}

/**
 * Aggregate every source feed into the central feed state.
 *
 * Source feeds are already final (each group job did its own previous-feed
 * fallback + version-time merge), so this step only unions them in canonical
 * app order and fills gaps from the previous central feed when a source group
 * produced no feed at all (its generate job failed). Catalog records are
 * taken from each source feed's own signed `catalogJson`, falling back to the
 * previous central catalog for apps of a failed group.
 *
 * @param {object} options
 * @param {object[]} options.allApps        all applications, in canonical order
 * @param {object[]} options.extraApps      feed-sources applications only, in order
 * @param {Object<string, object>} options.sourceFeeds discovered source feeds keyed by group id
 * @param {object|null} options.previousFeed parsed previous central feed.json, or null
 * @param {object} [options.builtinGroups]  { [sourceId]: string[] } built-in mapping
 * @returns {{ applications: object, catalogApps: object, reused: string[],
 *             notes: string[], missingGroups: string[] }}
 */
export function mergeSourceFeeds({
  allApps,
  extraApps,
  sourceFeeds,
  previousFeed,
  builtinGroups = {},
}) {
  const previous = previousFeed ?? null;
  const reused = [];
  const notes = [];
  const missingGroups = [];
  const knownGroups = new Set([
    ...Object.keys(builtinGroups),
    ...Object.keys(sourceFeeds),
  ]);
  for (const group of knownGroups) {
    if (!sourceFeeds[group]) missingGroups.push(group);
  }

  const applications = {};
  for (const app of allApps) {
    const group = sourceGroupOf(app, builtinGroups);
    const fresh = sourceFeeds[group]?.applications?.[app.applicationId] ?? null;
    const prev = previous?.applications?.[app.applicationId] ?? null;
    const resolved = entryOrPrevious(fresh, prev);
    if (resolved) {
      applications[app.applicationId] = resolved;
      if (!fresh) reused.push(`应用 ${app.applicationId}`);
    } else if (fresh == null && prev == null) {
      notes.push(`应用 ${app.applicationId} 本次抓取失败且历史上从未入 feed，未收录`);
    }
  }

  // Signed catalog: canonical order = feed-sources.json order. Each app's
  // record comes from the source feed that owns it (icon already injected,
  // CI-only fields already stripped); for a failed group, fall back to the
  // previous central feed's signed catalog record.
  const previousCatalogMap = parseCatalogApplications(previous?.catalogJson);
  const catalogApps = {};
  for (const app of extraApps) {
    const group = sourceGroupOf(app, builtinGroups);
    const sourceCatalog = sourceFeeds[group]
      ? parseCatalogApplications(sourceFeeds[group].catalogJson)
      : null;
    const own = sourceCatalog?.[app.applicationId] ?? null;
    const prevCat = previousCatalogMap?.[app.applicationId] ?? null;
    if (own) {
      catalogApps[app.applicationId] = own;
    } else if (prevCat) {
      catalogApps[app.applicationId] = { ...prevCat };
      reused.push(`目录 ${app.applicationId}`);
    } else {
      notes.push(`目录 ${app.applicationId}: 本次无记录且历史上未入 catalogJson，未收录`);
    }
  }

  return { applications, catalogApps, reused, notes, missingGroups };
}