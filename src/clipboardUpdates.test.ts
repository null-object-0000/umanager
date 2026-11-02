import { describe, expect, it, vi } from "vitest";
import { clipboardUpdates } from "./clipboardUpdates";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("clipboard updates", () => {
  it("does no reads while hidden and reloads after becoming visible", async () => {
    let visible = false;
    const load = vi.fn(async () => [1]);
    const changed = vi.fn();
    const updates = clipboardUpdates(load, () => visible, changed);
    await updates.refresh();
    expect(load).not.toHaveBeenCalled();
    visible = true;
    await updates.refresh();
    expect(changed).toHaveBeenCalledWith([1]);
  });

  it("coalesces a burst into one follow-up read without concurrent requests", async () => {
    const first = deferred<number>();
    const load = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(2);
    const changed = vi.fn();
    const updates = clipboardUpdates(load, () => true, changed);
    const pending = updates.refresh();
    await Promise.all([updates.refresh(), updates.refresh(), updates.refresh()]);
    expect(load).toHaveBeenCalledTimes(1);
    first.resolve(1);
    await pending;
    expect(load).toHaveBeenCalledTimes(2);
    expect(changed).toHaveBeenLastCalledWith(2);
  });

  it("does not publish in-flight results or start new reads after disposal", async () => {
    const first = deferred<number>();
    const load = vi.fn(() => first.promise);
    const changed = vi.fn();
    const updates = clipboardUpdates(load, () => true, changed);
    const pending = updates.refresh();
    void updates.refresh();
    updates.dispose();
    first.resolve(1);
    await pending;
    await updates.refresh();
    expect(load).toHaveBeenCalledTimes(1);
    expect(changed).not.toHaveBeenCalled();
  });

  it("drops results if hidden during a request and recovers when shown", async () => {
    let visible = true;
    const first = deferred<number>();
    const load = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(2);
    const changed = vi.fn();
    const updates = clipboardUpdates(load, () => visible, changed);
    const pending = updates.refresh();
    visible = false;
    first.resolve(1);
    await pending;
    expect(changed).not.toHaveBeenCalled();
    visible = true;
    await updates.refresh();
    expect(changed).toHaveBeenCalledWith(2);
  });

  it("recovers after a failed read", async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error("unavailable")).mockResolvedValue(2);
    const changed = vi.fn();
    const updates = clipboardUpdates(load, () => true, changed);
    await updates.refresh();
    await updates.refresh();
    expect(changed).toHaveBeenCalledWith(2);
  });
});
