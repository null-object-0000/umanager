// The official updater's manifest is ordered newest first. A GitHub release
// alone does not prove that `uv self update` can resolve the version yet.
export function resolveUpdaterManifest(text, platforms) {
  for (const line of text.split(/\r?\n/).filter((line) => line.trim())) {
    const entry = JSON.parse(line);
    if (!/^\d+\.\d+\.\d+$/.test(entry.version ?? "")) continue;
    if (!platforms.every((platform) => entry.artifacts?.some((item) => item.platform === platform))) continue;
    if (!Number.isFinite(Date.parse(entry.date))) throw new Error("更新索引发布时间无效");
    return entry;
  }
  throw new Error("更新索引未包含支持平台的正式版本");
}

export function releaseCommit(payload) {
  if (!/^[a-f0-9]{40}$/.test(payload?.sha ?? "")) throw new Error("发布提交 SHA 无效");
  return payload.sha;
}
