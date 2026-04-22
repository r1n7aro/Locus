import type { InspectorField, InspectorPanel } from "../../types";

export interface RendererSection {
  titleKey: string;
  paths: string[];
}

export interface FieldFilteringConfig {
  hiddenRoots: string[];
}

export interface SemanticGroupingConfig {
  sections: RendererSection[];
}

export interface ComponentRendererConfig {
  filtering: FieldFilteringConfig;
  grouping?: SemanticGroupingConfig;
}

export interface SectionResult {
  titleKey: string;
  fields: InspectorField[];
}

export interface PartitionResult {
  flatFields: InspectorField[];
  sections: SectionResult[];
  otherFields: InspectorField[];
  hiddenCount: number;
}

const ALWAYS_HIDDEN = ["serializedVersion"];
const HIDDEN_DESCENDANT_SEGMENTS = new Set(["serializedVersion"]);

const TRANSFORM_PATHS = [
  "m_LocalPosition",
  "m_LocalRotation",
  "m_LocalScale",
  "m_ConstrainProportionsScale",
];

const RENDERER_SHADOW_PATHS = [
  "m_CastShadows",
  "m_ReceiveShadows",
  "m_StaticShadowCaster",
];

const COLLIDER_FILTERING_PATHS = [
  "m_Material",
  "m_IsTrigger",
  "m_IncludeLayers",
  "m_ExcludeLayers",
  "m_LayerOverridePriority",
  "m_ProvidesContacts",
];

const registry = new Map<string, ComponentRendererConfig>([
  [
    "Transform",
    {
      filtering: {
        hiddenRoots: [...ALWAYS_HIDDEN, "m_Father", "m_Children", "m_RootOrder"],
      },
    },
  ],
  [
    "RectTransform",
    {
      filtering: {
        hiddenRoots: [...ALWAYS_HIDDEN, "m_Father", "m_Children", "m_RootOrder"],
      },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.transform",
            paths: TRANSFORM_PATHS,
          },
          {
            titleKey: "diff.optimized.rectLayout",
            paths: [
              "m_AnchorMin",
              "m_AnchorMax",
              "m_AnchoredPosition",
              "m_SizeDelta",
              "m_Pivot",
            ],
          },
        ],
      },
    },
  ],
  [
    "MeshFilter",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.mesh", paths: ["m_Mesh"] },
        ],
      },
    },
  ],
  [
    "MeshRenderer",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.materials", paths: ["m_Materials"] },
          {
            titleKey: "diff.optimized.lightingShadow",
            paths: [
              ...RENDERER_SHADOW_PATHS,
              "m_LightProbeUsage",
              "m_ReflectionProbeUsage",
              "m_LightmapIndex",
              "m_LightmapTilingOffset",
            ],
          },
          {
            titleKey: "diff.optimized.sorting",
            paths: ["m_SortingLayerID", "m_SortingLayer", "m_SortingOrder"],
          },
        ],
      },
    },
  ],
  [
    "SpriteRenderer",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.materials", paths: ["m_Materials"] },
          {
            titleKey: "diff.optimized.lightingShadow",
            paths: RENDERER_SHADOW_PATHS,
          },
          {
            titleKey: "diff.optimized.sprite",
            paths: ["m_Sprite", "m_Color", "m_FlipX", "m_FlipY", "m_DrawMode"],
          },
          {
            titleKey: "diff.optimized.sorting",
            paths: ["m_SortingLayerID", "m_SortingLayer", "m_SortingOrder"],
          },
        ],
      },
    },
  ],
  [
    "LineRenderer",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.materials", paths: ["m_Materials"] },
          {
            titleKey: "diff.optimized.lightingShadow",
            paths: RENDERER_SHADOW_PATHS,
          },
          {
            titleKey: "diff.optimized.widthColor",
            paths: ["m_Parameters"],
          },
          {
            titleKey: "diff.optimized.sorting",
            paths: ["m_SortingLayerID", "m_SortingLayer", "m_SortingOrder"],
          },
        ],
      },
    },
  ],
  [
    "Rigidbody",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.massDrag",
            paths: [
              "m_Mass",
              "m_Drag",
              "m_AngularDrag",
              "m_UseGravity",
              "m_IsKinematic",
            ],
          },
          { titleKey: "diff.optimized.constraints", paths: ["m_Constraints"] },
          {
            titleKey: "diff.optimized.collision",
            paths: ["m_CollisionDetection", "m_Interpolate"],
          },
        ],
      },
    },
  ],
  [
    "BoxCollider",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.shape", paths: ["m_Size", "m_Center"] },
          {
            titleKey: "diff.optimized.filtering",
            paths: COLLIDER_FILTERING_PATHS,
          },
        ],
      },
    },
  ],
  [
    "SphereCollider",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.shape",
            paths: ["m_Radius", "m_Center"],
          },
          {
            titleKey: "diff.optimized.filtering",
            paths: COLLIDER_FILTERING_PATHS,
          },
        ],
      },
    },
  ],
  [
    "CapsuleCollider",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.shape",
            paths: ["m_Radius", "m_Height", "m_Direction", "m_Center"],
          },
          {
            titleKey: "diff.optimized.filtering",
            paths: COLLIDER_FILTERING_PATHS,
          },
        ],
      },
    },
  ],
  [
    "MeshCollider",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.mesh",
            paths: ["m_Mesh", "m_Convex"],
          },
          {
            titleKey: "diff.optimized.filtering",
            paths: COLLIDER_FILTERING_PATHS,
          },
        ],
      },
    },
  ],
  [
    "Camera",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.projection",
            paths: ["orthographic", "field of view", "m_OrthographicSize"],
          },
          {
            titleKey: "diff.optimized.clipping",
            paths: ["near clip plane", "far clip plane"],
          },
          {
            titleKey: "diff.optimized.rendering",
            paths: [
              "m_ClearFlags",
              "m_BackGroundColor",
              "m_TargetTexture",
              "m_Depth",
            ],
          },
        ],
      },
    },
  ],
  [
    "Light",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.typeIntensity",
            paths: ["m_Type", "m_Intensity"],
          },
          {
            titleKey: "diff.optimized.colorRange",
            paths: ["m_Color", "m_Range", "m_SpotAngle"],
          },
          { titleKey: "diff.optimized.shadow", paths: ["m_Shadows"] },
        ],
      },
    },
  ],
  [
    "Animator",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.controller",
            paths: ["m_Controller", "m_Avatar"],
          },
          {
            titleKey: "diff.optimized.playback",
            paths: ["m_Enabled", "m_UpdateMode", "m_ApplyRootMotion"],
          },
          {
            titleKey: "diff.optimized.rootMotion",
            paths: ["m_HasTransformHierarchy"],
          },
        ],
      },
    },
  ],
  [
    "AudioSource",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.clipOutput",
            paths: ["m_audioClip", "OutputAudioMixerGroup"],
          },
          {
            titleKey: "diff.optimized.playback",
            paths: ["m_PlayOnAwake", "Loop", "m_Volume", "m_Pitch"],
          },
          {
            titleKey: "diff.optimized.spatial3d",
            paths: [
              "rolloffMode",
              "MinDistance",
              "MaxDistance",
              "panLevelCustomCurve",
            ],
          },
        ],
      },
    },
  ],
  [
    "Canvas",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.rendering",
            paths: [
              "m_RenderMode",
              "m_Camera",
              "m_SortingLayerID",
              "m_SortingOrder",
              "m_PixelPerfect",
            ],
          },
        ],
      },
    },
  ],
  [
    "CanvasGroup",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          {
            titleKey: "diff.optimized.interaction",
            paths: [
              "m_Alpha",
              "m_Interactable",
              "m_BlocksRaycasts",
              "m_IgnoreParentGroups",
            ],
          },
        ],
      },
    },
  ],
  [
    "ParticleSystem",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.main", paths: ["InitialModule"] },
          { titleKey: "diff.optimized.emission", paths: ["EmissionModule"] },
          { titleKey: "diff.optimized.shapeModule", paths: ["ShapeModule"] },
        ],
      },
    },
  ],
  [
    "TrailRenderer",
    {
      filtering: { hiddenRoots: [...ALWAYS_HIDDEN] },
      grouping: {
        sections: [
          { titleKey: "diff.optimized.material", paths: ["m_Materials"] },
          {
            titleKey: "diff.optimized.lightingShadow",
            paths: RENDERER_SHADOW_PATHS,
          },
          { titleKey: "diff.optimized.widthColor", paths: ["m_Parameters"] },
          {
            titleKey: "diff.optimized.lifetime",
            paths: ["m_Time", "m_MinVertexDistance"],
          },
        ],
      },
    },
  ],
]);

const defaultBuiltinConfig: ComponentRendererConfig = {
  filtering: {
    hiddenRoots: [...ALWAYS_HIDDEN],
  },
};

export function getRendererConfig(
  panel: InspectorPanel,
): ComponentRendererConfig | null {
  if (panel.componentSource !== "builtin") return null;
  return registry.get(panel.componentType ?? "") ?? defaultBuiltinConfig;
}

export function partitionFields(
  fields: InspectorField[],
  config: ComponentRendererConfig,
): PartitionResult {
  const hiddenSet = new Set(config.filtering.hiddenRoots);
  let hiddenCount = 0;
  const visible: InspectorField[] = [];

  for (const field of fields) {
    const [prunedField, prunedCount] = pruneHiddenField(field, hiddenSet);
    hiddenCount += prunedCount;
    if (prunedField) {
      visible.push(prunedField);
    }
  }

  const grouping = config.grouping;
  if (!grouping || grouping.sections.length === 0) {
    return {
      flatFields: visible,
      sections: [],
      otherFields: [],
      hiddenCount,
    };
  }

  const consumed = new Set<string>();
  const sections: SectionResult[] = [];

  for (const section of grouping.sections) {
    const pathSet = new Set(section.paths);
    const matched: InspectorField[] = [];
    for (const field of visible) {
      if (pathSet.has(field.propertyPath) && !consumed.has(field.propertyPath)) {
        matched.push(field);
        consumed.add(field.propertyPath);
      }
    }
    if (matched.length > 0) {
      sections.push({ titleKey: section.titleKey, fields: matched });
    }
  }

  const otherFields = visible.filter((f) => !consumed.has(f.propertyPath));

  return {
    flatFields: [],
    sections,
    otherFields,
    hiddenCount,
  };
}

function pruneHiddenField(
  field: InspectorField,
  hiddenRoots: Set<string>,
): [InspectorField | null, number] {
  if (shouldHideField(field.propertyPath, hiddenRoots)) {
    return [null, 1];
  }

  if (!field.children?.length) {
    return [field, 0];
  }

  let hiddenCount = 0;
  const children: InspectorField[] = [];
  for (const child of field.children) {
    const [prunedChild, childHiddenCount] = pruneHiddenField(child, hiddenRoots);
    hiddenCount += childHiddenCount;
    if (prunedChild) {
      children.push(prunedChild);
    }
  }

  return [{ ...field, children }, hiddenCount];
}

function shouldHideField(propertyPath: string, hiddenRoots: Set<string>): boolean {
  if (hiddenRoots.has(propertyPath)) {
    return true;
  }
  const segments = propertyPath.split(".");
  const lastSegment = segments[segments.length - 1] ?? propertyPath;
  return HIDDEN_DESCENDANT_SEGMENTS.has(lastSegment);
}
