import type { IconNode } from "lucide";

export type ProjectCapabilityId =
  | "assetDatabase"
  | "editorConnection"
  | "codeAnalysis"
  | "hotReload";

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
