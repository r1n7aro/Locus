<template>
  <div class="sub-window-loading" role="status" aria-live="polite">
    <span class="sub-window-loading__spinner" aria-hidden="true" />
  </div>
</template>

<style scoped>
.sub-window-loading {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.sub-window-loading__spinner {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: 2px solid currentColor;
  border-top-color: transparent;
  opacity: 0.55;
  animation: sub-window-spin 0.8s linear infinite;
}

@keyframes sub-window-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
