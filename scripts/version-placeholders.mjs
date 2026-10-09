// Version-placeholder resolution for the CI feed generator.
//
// Some vendors build a changelog URL out of the version the feed publishes
// (VS Code: `release-notes/v{major}_{minor}.md`; WeChat: `updates?version={version}`;
// Wine: `wine-mirror/wine/wine-{upstream}/ANNOUNCE.md`). This module owns that
// substitution so the mapping rules are unit-tested instead of living inline in
// `update-feed.mjs`.
//
// All of it happens in CI; the desktop app only ever reads the signed feed.

/**
 * Upstream version of a Debian package version: the part a *vendor* would use in
 * a tag or file name.
 *
 * - the epoch (`1:5.1.9`) is dropped,
 * - everything from the first `~` on is dropped (WineHQ's vendor suffix:
 *   `11.19~resolute-1` → `11.19`),
 * - a trailing Debian revision is dropped (`11.19-1` → `11.19`),
 * - for `~`-suffixed (WineHQ-style) versions, trailing zero components are also
 *   dropped, never below `major.minor`: WineHQ pads its *stable* package
 *   versions to four components (`11.0.0.0~resolute-1` for the `wine-11.0` tag,
 *   `9.0.1.0~resolute-1` for `wine-9.0.1`). Versions without that vendor suffix
 *   are left untouched, so an ordinary `1.135.0` stays `1.135.0`.
 *
 * @param {unknown} version
 * @returns {string}
 */
export function debianUpstreamVersion(version) {
  if (version == null) return "";
  let text = String(version).trim().replace(/^\d+:/, "");
  const vendorSuffix = text.includes("~");
  text = text.split("~")[0];
  if (!vendorSuffix || !/^\d/.test(text)) return text.replace(/-\d+$/, "");
  const parts = text.split(".");
  while (parts.length > 2 && parts[parts.length - 1] === "0") parts.pop();
  return parts.join(".").replace(/-\d+$/, "");
}

/**
 * Resolve placeholders in a URL template from the resolved version.
 *
 * Placeholders (all percent-encoded):
 *   `{version}`  the full version as published (`11.19~resolute-1`)
 *   `{major}`    `1` for `1.135.0-1787669172`
 *   `{minor}`    `135` for `1.135.0-1787669172`
 *   `{patch}`    `0` for `1.135.0-1787669172`
 *   `{upstream}` the Debian upstream version, trailing zeros trimmed
 *                (`11.19~resolute-1` → `11.19`, `11.0.0.0~resolute-1` → `11.0`)
 *
 * @param {unknown} template
 * @param {unknown} version
 * @returns {unknown}
 */
export function resolveVersionPlaceholders(template, version) {
  if (typeof template !== "string") return template;
  const parts = String(version).split(".");
  const major = parts[0] ?? "";
  const minor = parts[1] ?? "";
  const patch = parts[2] ?? "";
  return template
    .replaceAll("{version}", encodeURIComponent(String(version)))
    .replaceAll("{major}", encodeURIComponent(major))
    .replaceAll("{minor}", encodeURIComponent(minor))
    .replaceAll("{patch}", encodeURIComponent(patch))
    .replaceAll("{upstream}", encodeURIComponent(debianUpstreamVersion(version)));
}
