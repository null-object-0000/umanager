import { describe, expect, it } from "vitest";
import { aptInstallCommand, aptPackageName } from "./dependencyGap";

describe("dependency gap remedy wording", () => {
  it("keeps the plain name of a single-alternative group", () => {
    expect(aptPackageName("wine-devel (= 11.18~resolute-1)")).toBe("wine-devel");
    expect(aptPackageName("libgtk-3-0 (>= 3.24)")).toBe("libgtk-3-0");
    expect(aptPackageName("pulseaudio-utils")).toBe("pulseaudio-utils");
    // `libc6:any` drops the architecture qualifier.
    expect(aptPackageName("libc6:any")).toBe("libc6");
  });

  it("refuses to guess between alternatives", () => {
    expect(aptPackageName("python3 | python3.11")).toBeNull();
    expect(aptInstallCommand(["python3 | python3.11"])).toBe("sudo apt-get install -f");
  });

  it("ignores text it cannot turn into a package name", () => {
    expect(aptPackageName("无法读取安装包依赖信息，安装前请手动检查")).toBeNull();
    expect(aptPackageName("")).toBeNull();
  });

  it("builds a precise install command for the wine version pin", () => {
    expect(aptInstallCommand(["wine-devel (= 11.18~resolute-1)"])).toBe(
      "sudo apt-get install wine-devel"
    );
  });

  it("dedupes names and falls back to install -f when nothing can be named", () => {
    expect(aptInstallCommand(["uidmap", "uidmap", "pass (>= 1)"])).toBe(
      "sudo apt-get install uidmap pass"
    );
    expect(aptInstallCommand([])).toBe("sudo apt-get install -f");
  });
});
