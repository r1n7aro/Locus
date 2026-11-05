import type { IconNode } from "lucide";

export type ProjectCapabilityId =
  | "assetDatabase"
  | "editorConnection"
  | "codeAnalysis"
  | "hotReload"
  | "blenderConnection";

export interface ProjectServicePresentation {
  serviceId: string;
  icon: IconNode;
  capabilities: readonly ProjectCapabilityId[];
}

/** Unity brand mark from Simple Icons, normalized to the shared 24px icon grid. */
export const UNITY_PROJECT_ICON: IconNode = [["path", {
  d: "m12.9288 4.2939 3.7997 2.1929c.1366.077.1415.2905 0 .3675l-4.515 2.6076a.4192.4192 0 0 1-.4246 0L7.274 6.8543c-.139-.0745-.1415-.293 0-.3675l3.7972-2.193V0L1.3758 5.5977V16.793l3.7177-2.1456v-4.3858c-.0025-.1565.1813-.2682.318-.1838l4.5148 2.6076a.4252.4252 0 0 1 .2136.3676v5.2127c.0025.1565-.1813.2682-.3179.1838l-3.7996-2.1929-3.7178 2.1457L12 24l9.6954-5.5977-3.7178-2.1457-3.7996 2.1929c-.1341.082-.3229-.0248-.3179-.1838V13.053c0-.1565.087-.2956.2136-.3676l4.5149-2.6076c.134-.082.3228.0224.3179.1838v4.3858l3.7177 2.1456V5.5977L12.9288 0Z",
  fill: "currentColor",
  stroke: "none",
}]];

/** Blender brand mark from Simple Icons (16.36.0), on the shared 24px icon grid. */
export const BLENDER_PROJECT_ICON: IconNode = [["path", {
  d: "M12.51 13.214c.046-.8.438-1.506 1.03-2.006a3.424 3.424 0 0 1 2.212-.79c.85 0 1.631.3 2.211.79.592.5.983 1.206 1.028 2.005.045.823-.285 1.586-.865 2.153a3.389 3.389 0 0 1-2.374.938 3.393 3.393 0 0 1-2.376-.938c-.58-.567-.91-1.33-.865-2.152M7.35 14.831c.006.314.106.922.256 1.398a7.372 7.372 0 0 0 1.593 2.757 8.227 8.227 0 0 0 2.787 2.001 8.947 8.947 0 0 0 3.66.76 8.964 8.964 0 0 0 3.657-.772 8.285 8.285 0 0 0 2.785-2.01 7.428 7.428 0 0 0 1.592-2.762 6.964 6.964 0 0 0 .25-3.074 7.123 7.123 0 0 0-1.016-2.779 7.764 7.764 0 0 0-1.852-2.043h.002L13.566 2.55l-.02-.015c-.492-.378-1.319-.376-1.86.002-.547.382-.609 1.015-.123 1.415l-.001.001 3.126 2.543-9.53.01h-.013c-.788.001-1.545.518-1.695 1.172-.154.665.38 1.217 1.2 1.22V8.9l4.83-.01-8.62 6.617-.034.025c-.813.622-1.075 1.658-.563 2.313.52.667 1.625.668 2.447.004L7.414 14s-.069.52-.063.831zm12.09 1.741c-.97.988-2.326 1.548-3.795 1.55-1.47.004-2.827-.552-3.797-1.538a4.51 4.51 0 0 1-1.036-1.622 4.282 4.282 0 0 1 .282-3.519 4.702 4.702 0 0 1 1.153-1.371c.942-.768 2.141-1.183 3.396-1.185 1.256-.002 2.455.41 3.398 1.175.48.391.87.854 1.152 1.367a4.28 4.28 0 0 1 .522 1.706 4.236 4.236 0 0 1-.239 1.811 4.54 4.54 0 0 1-1.035 1.626",
  fill: "currentColor",
  stroke: "none",
}]];

/**
 * Project-specific UI lives behind detected workspace services. New engine
 * integrations (for example Blender or Unreal) register their mark and the
 * capabilities they actually implement here.
 */
export const PROJECT_SERVICE_PRESENTATIONS: readonly ProjectServicePresentation[] = [
  {
    serviceId: "unity",
    icon: UNITY_PROJECT_ICON,
    capabilities: ["assetDatabase", "editorConnection", "codeAnalysis", "hotReload"],
  },
  {
    serviceId: "blender",
    icon: BLENDER_PROJECT_ICON,
    capabilities: ["blenderConnection"],
  },
];

const PROJECT_SERVICE_PRESENTATION_BY_ID = new Map(
  PROJECT_SERVICE_PRESENTATIONS.map((presentation) => [presentation.serviceId, presentation]),
);

function normalizedServiceIds(serviceIds: readonly string[]): Set<string> {
  return new Set(serviceIds.map((serviceId) => serviceId.trim().toLowerCase()).filter(Boolean));
}

export function projectServicePresentation(
  serviceId: string,
): ProjectServicePresentation | undefined {
  return PROJECT_SERVICE_PRESENTATION_BY_ID.get(serviceId.trim().toLowerCase());
}

export function projectIconForServices(serviceIds: readonly string[]): IconNode | null {
  const enabled = normalizedServiceIds(serviceIds);
  return PROJECT_SERVICE_PRESENTATIONS.find(({ serviceId }) => enabled.has(serviceId))?.icon ?? null;
}

export function projectHasService(
  serviceIds: readonly string[],
  serviceId: string,
): boolean {
  return normalizedServiceIds(serviceIds).has(serviceId.trim().toLowerCase());
}

export function projectHasCapability(
  serviceIds: readonly string[],
  capability: ProjectCapabilityId,
): boolean {
  const enabled = normalizedServiceIds(serviceIds);
  return PROJECT_SERVICE_PRESENTATIONS.some(({ serviceId, capabilities }) => (
    enabled.has(serviceId) && capabilities.includes(capability)
  ));
}
