/** One running request per provider; typing replaces work that has not started. */
export function createMentionSearchQueue() {
  let running = false;
  let pending: { run: () => void; cancel: () => void } | undefined;

  function drain() {
    if (running || !pending) return;
    const next = pending;
    pending = undefined;
    next.run();
  }

  return {
    run<T>(task: () => Promise<T>): Promise<T | undefined> {
      pending?.cancel();
      return new Promise((resolve, reject) => {
        pending = {
          cancel: () => resolve(undefined),
          run: () => {
            running = true;
            Promise.resolve().then(task).then(resolve, reject).finally(() => {
              running = false;
              drain();
            });
          },
        };
        drain();
      });
    },
    clear() {
      pending?.cancel();
      pending = undefined;
    },
  };
}
