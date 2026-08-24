import { describe, expect, it } from "vitest";
import { summarizePackages } from "./model";
import type { ManagedPackage } from "./types";

const base: ManagedPackage = {
  packageName: "code",
  displayName: "Visual Studio Code",
  vendor: "Microsoft",
  installedVersion: "1.0",
  candidateVersion: "1.0",
  architecture: "amd64",
  sourceKind: "officialRepository",
  sourceUrl: "https://packages.microsoft.com/repos/code",
  updateState: "upToDate",
  homepage: null,
};

describe("summarizePackages", () => {
  it("counts official repositories and available updates", () => {
    const packages: ManagedPackage[] = [
      base,
      {
        ...base,
        packageName: "google-chrome-stable",
        updateState: "updateAvailable",
      },
      {
        ...base,
        packageName: "flclash",
        sourceKind: "localPackage",
        updateState: "unknown",
      },
      {
        ...base,
        packageName: "chatgpt",
        candidateVersion: null,
        updateState: "unknown",
      },
    ];

    expect(summarizePackages(packages)).toEqual({
      total: 4,
      updates: 1,
      repositories: 3,
      pendingRepositoryChecks: 1,
    });
  });
});
