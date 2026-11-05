import { inject, provide, readonly, ref, type InjectionKey, type Ref } from "vue";

/**
 * Hosts that keep a surface mounted while it is not shown (inactive workbench
 * tabs, laid out with `display: none`) provide this, so content inside can skip
 * live reads and layout measurement until the surface is shown again. Content
 * without such a host always counts as shown.
 */
const SURFACE_HIDDEN: InjectionKey<Readonly<Ref<boolean>>> = Symbol("surface-hidden");
const ALWAYS_SHOWN: Readonly<Ref<boolean>> = readonly(ref(false));

export function provideSurfaceHidden(hidden: Readonly<Ref<boolean>>): void {
  provide(SURFACE_HIDDEN, hidden);
}

export function useSurfaceHidden(): Readonly<Ref<boolean>> {
  return inject(SURFACE_HIDDEN, ALWAYS_SHOWN);
}
