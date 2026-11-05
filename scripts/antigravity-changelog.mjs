// CI-only parsing of Google's shared App / CLI / IDE changelog. The tab query
// does not filter the HTML: every product is present in the same document.
import { htmlToMarkdown } from './changelog-atom.mjs';

export function antigravityChangelog(html, product, version) {
  if (!['hub', 'cli', 'ide'].includes(product)) throw new Error('未知的 Antigravity 产品');
  const articles = [...html.matchAll(/<article\b[^>]*\bid="rel-(hub|cli|ide)-([0-9.]+)"[^>]*>([\s\S]*?)<\/article>/g)]
    .filter((match) => match[1] === product);
  const entry = version == null ? articles[0] : articles.find((match) => match[2] === version);
  const releaseNotesUrl = `https://antigravity.google/docs/changelog?tab=${product}`;
  if (!entry) return { releaseNotes: '', releaseNotesUrl };
  const block = entry[3];
  const start = block.search(/<h3\b[^>]*class="[^"]*rn-headline/);
  if (start < 0) throw new Error('Antigravity 日志缺少正文');
  const notes = block.slice(start)
    .replace(/<(script|style|svg|button)\b[^>]*>[\s\S]*?<\/\1>/gi, '')
    .replace(/<span\b[^>]*class="[^"]*rn-count[^" ]*[^"]*"[^>]*>[\s\S]*?<\/span>/g, '')
    .replace(/<summary\b[^>]*>([\s\S]*?)<\/summary>/g, '<h3>$1</h3>')
    .replace(/<\/?div\b[^>]*>/g, '')
    .replace(/<a\b[^>]*href="([^"]+)"[^>]*>/g, (_, href) => {
      try { const url = new URL(href.replaceAll('&amp;', '&'), 'https://antigravity.google'); return url.protocol === 'https:' ? `<a href="${url.href}">` : '<a>'; }
      catch { return '<a>'; }
    });
  const date = /<time\b[^>]*>([^<]+)<\/time>/.exec(block)?.[1];
  const milliseconds = date ? Date.parse(date.trim() + ' UTC') : NaN;
  return {
    version: entry[2], releaseNotes: htmlToMarkdown(notes),
    releaseNotesUrl: `${releaseNotesUrl}#rel-${product}-${entry[2]}`,
    updatedAtUnixSeconds: Number.isFinite(milliseconds) ? Math.floor(milliseconds / 1000) : null,
  };
}

export function antigravityCliVersion(manifest) {
  if (!/^\d+\.\d+\.\d+$/.test(manifest?.version ?? '')
      || !/^https:\/\/storage\.googleapis\.com\/antigravity-public\/antigravity-cli\//.test(manifest?.url ?? '')
      || !/^[a-f0-9]{128}$/i.test(manifest?.sha512 ?? '')) throw new Error('Antigravity CLI 官方版本索引无效');
  return manifest.version;
}
