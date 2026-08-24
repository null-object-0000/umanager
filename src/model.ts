import type { ManagedPackage } from "./types";

export function summarizePackages(packages: ManagedPackage[]) {
  return {
    total: packages.length,
    updates: packages.filter((item) => item.updateState === "updateAvailable").length,
    repositories: packages.filter((item) => item.sourceKind === "officialRepository").length,
    pendingRepositoryChecks: packages.filter((item) => item.sourceKind === "officialRepository" && item.updateState === "unknown").length,
  };
}
