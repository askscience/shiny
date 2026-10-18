/** A mount's owned resources. Disposal is idempotent, in reverse order, and
 * continues after individual cleanup failures. No timers survive an unmount.
 * @returns {{add: (cleanup: Function) => Function, on: Function,
 * interval: Function, timeout: Function, controller: Function, dispose: Function}}
 */
export function createResources() {
  let disposed = false;
  const cleanups = [];
  const add = (cleanup) => {
    if (disposed) cleanup();
    else cleanups.push(cleanup);
    return cleanup;
  };
  return {
    add,
    on(target, type, handler, options) {
      target?.addEventListener(type, handler, options);
      add(() => target?.removeEventListener(type, handler, options));
      return handler;
    },
    interval(fn, ms) {
      const timer = setInterval(fn, ms);
      add(() => clearInterval(timer));
      return timer;
    },
    timeout(fn, ms) {
      const timer = setTimeout(fn, ms);
      add(() => clearTimeout(timer));
      return timer;
    },
    controller() {
      const controller = new AbortController();
      add(() => controller.abort());
      return controller;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      for (const cleanup of cleanups.splice(0).reverse()) {
        try { cleanup(); }
        catch (error) { console.warn('Resource cleanup failed', error); }
      }
    },
  };
}
