// Tencent Docs desktop changelog parser.
//
// Tencent Docs publishes its Linux desktop changelog as a public "smartdoc"
// (e.g. https://docs.qq.com/aio/p/scqtn9829617css). The page data is served by
// a POST JSON API: `records.block` is a flat map keyed by block id, and the
// page block's `children` array gives the document order. Each version is a
// heading like "【Linux】版本号3.11.7" (the newest version lives in the top
// callout as "版本号3.11.8" + "更新时间：…" + "本次更新："), followed by
// "- …" bullet paragraphs.
//
// These pure functions parse the API payload, select the section for a given
// version (falling back to the first/latest section, matching the best-effort
// rule of the other changelog helpers) and render it as Markdown bullets.

const VERSION_HEADING_RE = /版本号\s*([\d.]+)/;
const UPDATE_TIME_RE = /^更新时间[:：]\s*(.+)$/;

// Walk the page's block tree in document order, grouping bullet paragraphs
// under the nearest version heading and capturing each section's "更新时间"
// line. Returns [{ version, updateTime, bullets }] in document order.
export function parseSmartdocVersionSections(payload, pageId) {
  const blocks = payload?.result?.curPageData?.records?.block;
  if (!blocks || typeof blocks !== "object") return [];
  const page = blocks[pageId];
  const children = Array.isArray(page?.value?.children) ? page.value.children : [];
  if (children.length === 0) return [];

  const textOf = (value) => (Array.isArray(value?.props?.title)
    ? value.props.title.map((segment) => (Array.isArray(segment) ? String(segment[0]) : "")).join("")
    : "");

  const sections = [];
  let current = null;
  const walk = (ids) => {
    for (const id of ids) {
      const block = blocks[id];
      if (!block?.value) continue;
      const value = block.value;
      const text = textOf(value);
      const headingMatch = /^header[234]$/.test(value.type ?? "") ? text.match(VERSION_HEADING_RE) : null;
      if (headingMatch) {
        current = { version: headingMatch[1], updateTime: null, bullets: [] };
        sections.push(current);
      } else if (current && value.type === "text") {
        const updateMatch = text.match(UPDATE_TIME_RE);
        if (updateMatch) {
          current.updateTime = updateMatch[1].trim();
        } else {
          const line = text.trim();
          if (line !== "" && line !== "-" && line !== "本次更新：") {
            current.bullets.push(line);
          }
        }
      }
      if (Array.isArray(value.children)) walk(value.children);
    }
  };
  walk(children);
  return sections;
}

// Select the section whose version equals `version`, else the first (latest).
export function selectSmartdocSection(sections, version) {
  if (!Array.isArray(sections) || sections.length === 0) return null;
  return sections.find((section) => section.version === version) ?? sections[0];
}

// Render a section as "更新时间：…" + Markdown bullets.
export function smartdocSectionToMarkdown(section) {
  if (!section || !Array.isArray(section.bullets) || section.bullets.length === 0) return "";
  const dateLine = section.updateTime ? `更新时间：${section.updateTime}\n\n` : "";
  return `${dateLine}${section.bullets.join("\n")}`;
}

// Parse "2026年8月5日" into integer unix seconds (UTC), or null.
export function smartdocUpdateTimeToUnixSeconds(text) {
  if (!text) return null;
  const match = String(text).match(/(\d{4})年(\d{1,2})月(\d{1,2})日/);
  if (!match) return null;
  return Math.floor(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])) / 1000);
}