// UManager self-release changelog generator.
//
// GitHub release bodies are the canonical "what changed in this version" record
// for UManager itself: `scripts/update-feed.mjs` reads the latest release's
// `body` into the signed feed's `selfUpdate.releaseNotes`, and the desktop app
// shows those notes in the update drawer. Releases without a changelog therefore
// ship an empty "新内容" section, so **every** release must carry one.
//
// This module derives a changelog from Conventional-commit history between the
// previous version tag and the tag being released. Parsing is extracted into
// pure functions so the grouping rules are unit-tested (see
// release-changelog.test.mjs); the CLI entry point wraps them with `git`.
//
// Usage (in CI, the release tag is passed explicitly so the previous tag is the
// newest *older* tag):
//
//   node scripts/release-changelog.mjs --version 0.8.10
//
// Without `--version` it releases the tip of the current checkout (previous tag
// = newest existing `v*` tag, excluding any tag pointing exactly at HEAD).

const COMMIT_TYPES = [
  {
    prefix: "feat",
    heading: "新功能",
    emoji: "✨",
  },
  {
    prefix: "fix",
    heading: "问题修复",
    emoji: "🐛",
  },
  {
    prefix: "perf",
    heading: "性能优化",
    emoji: "⚡",
  },
  {
    prefix: "refactor",
    heading: "重构",
    emoji: "♻️",
  },
  {
    prefix: "docs",
    heading: "文档",
    emoji: "📝",
  },
  {
    prefix: "chore",
    heading: "维护与构建",
    emoji: "🔧",
  },
  {
    prefix: "build",
    heading: "维护与构建",
    emoji: "🔧",
  },
  {
    prefix: "ci",
    heading: "维护与构建",
    emoji: "🔧",
  },
  {
    prefix: "style",
    heading: "代码风格",
    emoji: "🎨",
  },
];

const FALLBACK_HEADING = "其他变更";

const COMMIT_SUBJECT_PATTERN = new RegExp(
  String.raw`^(fix|feat|perf|refactor|docs|chore|build|ci|style|test|revert)?\s*(?:\(([^)]+)\))?:?\s*(.*)$`,
  "i",
);

/**
 * Parse one `git log --format=%s` line into a { headingKey, scope, subject }.
 * Unknown prefixes (including release commits prefixed `release:`) fall back to
 * FALLBACK_HEADING so they never get dropped.
 *
 * @param {unknown} subject line of a commit
 * @returns {{ heading: string, scope: string | null, subject: string }}
 */
export function parseCommitSubject(raw) {
  const line = typeof raw === "string" ? raw.trim() : "";
  if (!line) {
    return { heading: FALLBACK_HEADING, scope: null, subject: line };
  }
  const match = line.match(COMMIT_SUBJECT_PATTERN);
  if (!match) {
    return { heading: FALLBACK_HEADING, scope: null, subject: line };
  }
  const [, prefix, scope, rest] = match;
  const type = COMMIT_TYPES.find(
    (entry) => entry.prefix === (prefix || "").toLowerCase(),
  );
  const heading = type ? type.heading : FALLBACK_HEADING;
  const subject = (rest || line).trim();
  return {
    heading,
    scope: scope ? scope.trim() : null,
    subject: subject || line,
  };
}

/**
 * Group parsed commit lines into headings and render a Markdown changelog.
 *
 * @param {string} gitLogOutput output of `git log --format=%s <from>..<to>`
 * @param {string} [version] version being released (rendered in the title)
 * @returns {string}
 */
export function buildChangelog(gitLogOutput, version) {
  const commits = gitLogOutput
    .split("\n")
    .map(parseCommitSubject)
    .filter((entry) => entry.subject.length > 0);
  const sections = new Map();
  for (const { heading, scope, subject } of commits) {
    const scoped = scope ? `\`${scope}\`：${subject}` : subject;
    if (!sections.has(heading)) sections.set(heading, []);
    sections.get(heading).push(`- ${scoped}`);
  }
  const lines = [];
  const title = version ? `## ${version}` : "## 更新内容";
  lines.push(title, "");
  if (sections.size === 0) {
    lines.push("无提交记录。");
  } else {
    for (const [heading, items] of sections) {
      lines.push(`### ${heading}`, "");
      lines.push(...items);
      lines.push("");
    }
  }
  return lines.join("\n").trim();
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let i = 0; i < Math.max(leftParts.length, rightParts.length); i += 1) {
    const diff = (leftParts[i] ?? 0) - (rightParts[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

/**
 * Newest existing version tag *older* than `version` (or the newest tag when no
 * version is given, i.e. releasing the current tip). Version comparison is
 * split-safe (v0.8.10 > v0.8.9).
 *
 * When `excludeTag` is given (e.g. the tag pointing exactly at HEAD — the
 * release being created on a manual re-dispatch) it is filtered out first, so
 * a manual run after the release tag already exists still resolves the
 * previous version instead of the release's own tag.
 *
 * @param {string[]} tags tags (may include a leading `v`)
 * @param {string} [version] version being released
 * @param {string} [excludeTag] tag to ignore (raw form, as found in `tags`)
 * @returns {string | null} previous tag (raw tag as found in `tags`)
 */
export function selectPreviousTag(tags, version, excludeTag = null) {
  const candidates = excludeTag ? tags.filter((tag) => tag !== excludeTag) : tags;
  const normalized = candidates
    .map((tag) => ({ raw: tag, version: tag.replace(/^v/, "") }))
    .filter((entry) => /^\d+(?:\.\d+)*$/.test(entry.version))
    .sort((a, b) => compareVersions(a.version, b.version));
  if (!version) return normalized.at(-1)?.raw ?? null;
  const target = version.replace(/^v/, "");
  const older = normalized.filter((entry) => compareVersions(entry.version, target) < 0);
  return older.at(-1)?.raw ?? null;
}

/**
 * CLI entry point. Resolves the previous tag and prints the changelog to
 * stdout (feeds `releaseBody` in release.yml).
 */
export async function main(argv = process.argv.slice(2), cwd = process.cwd()) {
  const versionIndex = argv.indexOf("--version");
  const version = versionIndex >= 0 && argv[versionIndex + 1] ? argv[versionIndex + 1] : null;

  const { execFileSync } = await import("node:child_process");
  const tags = execFileSync("git", ["tag", "--list", "v*"], { cwd, encoding: "utf8" })
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  // Without an explicit version we release the current tip. If HEAD is exactly
  // the release's own tag (manual re-dispatch after the tag was pushed), that
  // tag must not count as the "previous" one — exclude it so the range still
  // starts at the last real release.
  let headTag = null;
  try {
    headTag = execFileSync("git", ["describe", "--tags", "--exact-match", "HEAD"], { cwd, encoding: "utf8" }).trim();
  } catch {
    /* HEAD is not exactly tagged: no exclusion needed */
  }
  const previousTag = selectPreviousTag(tags, version, headTag);
  if (!previousTag) {
    throw new Error("无法确定上一个版本 tag，无法生成 changelog");
  }
  const range = `${previousTag}..HEAD`;
  const log = execFileSync(
    "git",
    ["log", "--format=%s", range],
    { cwd, encoding: "utf8" },
  );
  return buildChangelog(log, version);
}

// Run only when executed directly (not imported by tests). `pathToFileURL`
// normalizes the comparison so it also works when the checkout path contains
// non-ASCII characters (import.meta.url is percent-encoded).
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";

const IS_CLI = Boolean(
  process.argv[1] &&
    pathToFileURL(resolve(process.argv[1])).href === import.meta.url,
);
if (IS_CLI) {
  main()
    .then((changelog) => process.stdout.write(`${changelog}\n`))
    .catch((error) => {
      process.stderr.write(`${error.message}\n`);
      process.exit(1);
    });
}