// 合并突发通知，最多保留一个读取请求；隐藏时不加载，销毁后不回写 React 状态。
export function clipboardUpdates<T>(
  load: () => Promise<T>,
  visible: () => boolean,
  changed: (value: T) => void,
) {
  let active = true;
  let running = false;
  let dirty = false;

  const refresh = async () => {
    if (!active || !visible()) return;
    dirty = true;
    if (running) return;
    running = true;
    try {
      while (active && dirty && visible()) {
        dirty = false;
        try {
          const value = await load();
          if (active && visible()) changed(value);
        } catch { /* 恢复可见或下一次通知时重试，保留已有内容。 */ }
      }
    } finally {
      running = false;
    }
  };

  return { refresh, dispose: () => { active = false; } };
}
