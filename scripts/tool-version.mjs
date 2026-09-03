// npm dist-tag version resolution for the feed's dev tools.
//
// The feed reports each npm-distributed tool's "latest" version. npm exposes a
// package's channels as dist-tags: `latest` is the conventional stable tag, but
// a vendor may leave it pointing at a release behind the newest one (e.g.
// deepseek-harness's `latest` currently resolves to `0.1.1-rc.2` while the
// newest release lives on the `alpha` tag). A tool can pin the channel it
// tracks with `distTag` in vendors.json; the feed then resolves that tag, and
// the app installs the same tag, so the advertised and installed versions
// always agree.

/**
 * Resolve the version a tool entry should report from an npm registry package
 * document, honoring an optional dist-tag channel.
 *
 * @param {unknown} doc - the npm registry package document.
 * @param {string} [distTag] - npm dist-tag to resolve (`latest` by default).
 * @returns {string} the version the tag points at.
 * @throws when the document, its dist-tags, or the requested tag is missing or
 * invalid — a misconfigured channel must fail the feed for that tool loudly
 * instead of silently reporting a different channel's version.
 */
export function resolveNpmDistTagVersion(doc, distTag) {
  if (!doc || typeof doc !== "object" || Array.isArray(doc)) {
    throw new Error("npm 包信息不是有效对象");
  }
  const tag = typeof distTag === "string" && distTag.trim() ? distTag.trim() : "latest";
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(tag)) {
    throw new Error(`非法 npm dist-tag ${JSON.stringify(tag)}`);
  }
  const tags = doc["dist-tags"];
  if (!tags || typeof tags !== "object" || Array.isArray(tags)) {
    throw new Error("npm 包未返回 dist-tags");
  }
  const resolved = tags[tag];
  if (typeof resolved !== "string" || !resolved) {
    throw new Error(`npm 包的 ${JSON.stringify(tag)} 标签未发布版本`);
  }
  return resolved;
}