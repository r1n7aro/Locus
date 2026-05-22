import {
  Box,
  Braces,
  ChartNoAxesCombined,
  FileCode2,
  FormInput,
  Grid2X2,
  InspectionPanel,
  Kanban,
  Layers,
  Link2,
  Network,
  Package,
  PanelTopOpen,
  PanelsTopLeft,
  Puzzle,
  ScanSearch,
  TableProperties,
  View,
  Workflow,
  type IconNode,
} from "lucide";

export const DEFAULT_LOCUS_VIEW_ICON = "View";

export const LOCUS_VIEW_ICON_LIBRARY = {
  View,
  PanelTopOpen,
  PanelsTopLeft,
  InspectionPanel,
  TableProperties,
  Network,
  Link2,
  Workflow,
  Kanban,
  Grid2X2,
  Layers,
  Package,
  Box,
  Braces,
  FileCode2,
  Puzzle,
  ScanSearch,
  ChartNoAxesCombined,
  FormInput,
} as const satisfies Record<string, IconNode>;

export type LocusViewIconName = keyof typeof LOCUS_VIEW_ICON_LIBRARY;

export const LOCUS_VIEW_ICON_NAMES = Object.keys(
  LOCUS_VIEW_ICON_LIBRARY,
) as LocusViewIconName[];

export function resolveLocusViewIcon(icon?: string | null): IconNode {
  const name = icon?.trim() as LocusViewIconName | undefined;
  if (name && Object.prototype.hasOwnProperty.call(LOCUS_VIEW_ICON_LIBRARY, name)) {
    return LOCUS_VIEW_ICON_LIBRARY[name];
  }
  return LOCUS_VIEW_ICON_LIBRARY[DEFAULT_LOCUS_VIEW_ICON];
}
