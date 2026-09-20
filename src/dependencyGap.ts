// 依赖缺口文案：纯展示层。只决定「补装命令怎么写给用户看」，不参与 helper
// 授权、不改变不可变计划，也不判断依赖是否真能满足（那是
// src-tauri/src/dependency_check.rs 与 helper 的职责）。
//
// 为什么要精确到包名：`apt-get install -f` 是一次「修复依赖」的完整事务，当
// 依赖根本不在已配置的源里时，它唯一能做的满足方式是*卸载*刚装上的包。直接
// 点名要装的包更安全，也让用户一眼看出这次会动哪个包。
//
// 依赖组里出现 `|` 候选时退回 `install -f`：替用户挑一个候选是在猜。

/** 从依赖组原文（`wine-devel (= 11.18~resolute-1)`）里取出可交给 apt 的包名。 */
export function aptPackageName(group: string): string | null {
  if (group.includes("|")) return null;
  const match = /^\s*([a-z0-9][a-z0-9+.-]*)/.exec(group);
  return match ? match[1] : null;
}

/** 依赖组里能精确点名的包名，保持出现顺序并去重。 */
export function aptPackageNames(groups: string[]): string[] {
  const names: string[] = [];
  for (const group of groups) {
    const name = aptPackageName(group);
    if (name !== null && !names.includes(name)) names.push(name);
  }
  return names;
}

/**
 * 补装依赖的命令：能解析出包名时精确到包名（`sudo apt-get install wine-devel`），
 * 否则退回 `sudo apt-get install -f`。
 */
export function aptInstallCommand(groups: string[]): string {
  const names = aptPackageNames(groups);
  return names.length > 0 ? `sudo apt-get install ${names.join(" ")}` : "sudo apt-get install -f";
}
