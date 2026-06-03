const BODY_SELECTION_LOCK_CLASS = "is-dragging-select-lock";

let activeSelectionLocks = 0;

export function acquireSelectionLock(): () => void {
  const body = typeof document !== "undefined" ? document.body : null;
  let released = false;

  activeSelectionLocks += 1;
  body?.classList.add(BODY_SELECTION_LOCK_CLASS);

  return () => {
    if (released) return;
    released = true;
    activeSelectionLocks = Math.max(0, activeSelectionLocks - 1);
    if (activeSelectionLocks === 0) {
      body?.classList.remove(BODY_SELECTION_LOCK_CLASS);
    }
  };
}

export function getSelectionLockBodyClass() {
  return BODY_SELECTION_LOCK_CLASS;
}
