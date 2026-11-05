import { describe, expect, it } from 'vitest';
import { antigravityChangelog, antigravityCliVersion } from './antigravity-changelog.mjs';
import { selectTarRelease } from './tar-source.mjs';
const entry = (product, version, title) => `<article class="rn-row" id="rel-${product}-${version}"><div><time>November 3, 2026</time></div><div><h3 class="rn-headline">${title}</h3><div><p>Summary</p></div><details><summary>Improvements<span class="rn-count">(1)</span><svg>arrow</svg></summary><div><ul><li>A change with <a href="/docs/example">documentation</a></li></ul></div></details></div></article>`;
const html = entry('hub', '2.6.0', 'App only') + entry('cli', '2.6.0', 'CLI only') + entry('ide', '2.6.0', 'IDE only') + entry('ide', '2.5.0', 'Previous IDE');
describe('Antigravity release sources', () => {
  it('selects the exact product and version from a shared, multi-tab page', () => {
    const result = antigravityChangelog(html, 'ide', '2.6.0');
    expect(result.releaseNotes).toContain('IDE only');
    for (const wrong of ['App only', 'CLI only', 'Previous IDE', '(1)', 'arrow']) expect(result.releaseNotes).not.toContain(wrong);
    expect(result.releaseNotes).toContain('[documentation](https://antigravity.google/docs/example)');
    expect(result.updatedAtUnixSeconds).toBe(Date.UTC(2026,10,3)/1000);
    expect(result.releaseNotesUrl).toBe('https://antigravity.google/docs/changelog?tab=ide#rel-ide-2.6.0');
  });
  it('keeps the product link without substituting another release when notes lag', () => {
    expect(antigravityChangelog(html, 'cli', '1.5.1')).toEqual({releaseNotes:'',releaseNotesUrl:'https://antigravity.google/docs/changelog?tab=cli'});
    expect(() => antigravityChangelog(html,'unknown','2.6.0')).toThrow();
  });
  it('resolves the official CLI manifest and rejects malformed releases', () => {
    const manifest={version:'1.5.1',url:'https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.1-123/linux-x64/cli_linux_x64.tar.gz',sha512:'a'.repeat(128)};
    expect(antigravityCliVersion(manifest)).toBe('1.5.1');
    for (const bad of [{...manifest,version:'latest'}, {...manifest,url:'https://storage.googleapis.com.evil.test/cli'}, {...manifest,sha512:'bad'}]) expect(()=>antigravityCliVersion(bad)).toThrow();
  });
  it('resolves IDE versions independently of the changing CDN shard path', () => {
    const url='https://edgedl.me.gvt1.com/edgedl/release2/new-shard/antigravity/stable/2.6.0-123/linux-x64/Antigravity%20IDE.tar.gz';
    expect(selectTarRelease(`<a href="${url}">IDE</a>`,{downloadUrlPrefix:'https://edgedl.me.gvt1.com/edgedl/release2/',downloadUrlSuffix:'/linux-x64/Antigravity%20IDE.tar.gz',downloadVersionPattern:'/antigravity/stable/(\\d+\\.\\d+\\.\\d+)-\\d+/'})).toEqual({downloadUrl:url,version:'2.6.0'});
  });
});
