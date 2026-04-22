use std::collections::HashMap;
use std::sync::OnceLock;

use crate::asset_db::types::PropertyOverride;
use crate::diff::types::InspectorComponentInference;

use super::unity_builtin::unity_class_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComponentInferenceResult {
    pub(crate) component_type: String,
    pub(crate) inferred_class_id: Option<i32>,
    pub(crate) reason_code: String,
    pub(crate) evidence: Vec<String>,
}

impl ComponentInferenceResult {
    pub(crate) fn to_inspector_inference(&self) -> InspectorComponentInference {
        InspectorComponentInference {
            reason_code: self.reason_code.clone(),
            evidence: self.evidence.clone(),
            inferred_class_id: self.inferred_class_id,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BuiltinComponentSignature {
    class_id: i32,
    name: &'static str,
    family: Option<&'static str>,
    strong_fields: &'static [&'static str],
    weak_fields: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct IndexedCandidate {
    class_id: i32,
    name: &'static str,
    family: Option<&'static str>,
    is_family: bool,
    strength: FieldStrength,
}

#[derive(Debug, Clone, Copy)]
enum FieldStrength {
    Strong,
    Weak,
}

#[derive(Debug, Clone)]
struct CandidateScore {
    class_id: i32,
    name: &'static str,
    family: Option<&'static str>,
    is_family: bool,
    strong_hits: usize,
    weak_hits: usize,
    score: usize,
    evidence: Vec<String>,
}

const EVIDENCE_LIMIT: usize = 5;

const IGNORED_FIELDS: &[&str] = &[
    "serializedVersion",
    "m_ObjectHideFlags",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "m_GameObject",
    "m_Enabled",
    "m_EditorHideFlags",
    "m_Script",
];

const BUILTIN_COMPONENT_SIGNATURES: &[BuiltinComponentSignature] = &[
    sig(
        1,
        "GameObject",
        Some("GameObject"),
        &[
            "m_Name",
            "m_TagString",
            "m_Layer",
            "m_IsActive",
            "m_StaticEditorFlags",
        ],
        &[],
    ),
    sig(
        4,
        "Transform",
        Some("Transform"),
        &[
            "m_LocalPosition",
            "m_LocalRotation",
            "m_LocalScale",
            "m_LocalEulerAnglesHint",
            "m_ConstrainProportionsScale",
        ],
        &["m_Father", "m_RootOrder"],
    ),
    sig(
        224,
        "RectTransform",
        Some("Transform"),
        &[
            "m_AnchorMin",
            "m_AnchorMax",
            "m_AnchoredPosition",
            "m_SizeDelta",
            "m_Pivot",
        ],
        &[],
    ),
    sig(
        25,
        "Renderer",
        Some("Renderer"),
        &[
            "m_Materials",
            "m_CastShadows",
            "m_ReceiveShadows",
            "m_StaticShadowCaster",
            "m_LightProbeUsage",
            "m_ReflectionProbeUsage",
            "m_LightmapIndex",
            "m_LightmapTilingOffset",
        ],
        &["m_SortingLayerID", "m_SortingLayer", "m_SortingOrder"],
    ),
    sig(
        23,
        "MeshRenderer",
        Some("Renderer"),
        &[
            "m_ScaleInLightmap",
            "m_ReceiveGI",
            "m_PreserveUVs",
            "m_ImportantGI",
            "m_StitchLightmapSeams",
        ],
        &[],
    ),
    sig(
        137,
        "SkinnedMeshRenderer",
        Some("Renderer"),
        &[
            "m_Quality",
            "m_UpdateWhenOffscreen",
            "m_SkinnedMotionVectors",
            "m_RootBone",
            "m_Bones",
            "m_BlendShapeWeights",
        ],
        &["m_Mesh"],
    ),
    sig(
        212,
        "SpriteRenderer",
        Some("Renderer"),
        &[
            "m_Sprite",
            "m_FlipX",
            "m_FlipY",
            "m_DrawMode",
            "m_SpriteSortPoint",
        ],
        &["m_Color", "m_Size"],
    ),
    sig(
        120,
        "LineRenderer",
        Some("Renderer"),
        &[
            "m_Parameters",
            "m_Positions",
            "m_UseWorldSpace",
            "m_Loop",
            "m_Alignment",
            "m_TextureMode",
        ],
        &[],
    ),
    sig(
        96,
        "TrailRenderer",
        Some("Renderer"),
        &[
            "m_Time",
            "m_MinVertexDistance",
            "m_Autodestruct",
            "m_Emitting",
            "m_GenerateLightingData",
        ],
        &["m_Parameters"],
    ),
    sig(
        199,
        "ParticleSystemRenderer",
        Some("Renderer"),
        &[
            "m_RenderMode",
            "m_Mesh",
            "m_MinParticleSize",
            "m_MaxParticleSize",
            "m_CameraVelocityScale",
            "m_VelocityScale",
            "m_LengthScale",
        ],
        &[],
    ),
    sig(
        227,
        "BillboardRenderer",
        Some("Renderer"),
        &["m_Billboard"],
        &[],
    ),
    sig(
        483693784,
        "TilemapRenderer",
        Some("Renderer"),
        &[
            "m_Mode",
            "m_DetectChunkCullingBounds",
            "m_ChunkSize",
            "m_ChunkCullingBounds",
        ],
        &[],
    ),
    sig(
        1971053207,
        "SpriteShapeRenderer",
        Some("Renderer"),
        &["m_SpriteShape", "m_SpriteShapeParameters"],
        &[],
    ),
    sig(
        73398921,
        "VFXRenderer",
        Some("Renderer"),
        &["m_VisualEffect", "m_VFXRendererSettings"],
        &[],
    ),
    sig(33, "MeshFilter", Some("MeshFilter"), &[], &["m_Mesh"]),
    sig(
        20,
        "Camera",
        Some("Camera"),
        &[
            "m_ClearFlags",
            "m_BackGroundColor",
            "m_projectionMatrixMode",
            "m_FOVAxisMode",
            "m_TargetTexture",
            "m_TargetDisplay",
            "m_Orthographic",
            "m_OrthographicSize",
            "orthographic",
            "field of view",
            "near clip plane",
            "far clip plane",
        ],
        &["m_Depth"],
    ),
    sig(
        108,
        "Light",
        Some("Light"),
        &[
            "m_Type",
            "m_Intensity",
            "m_Range",
            "m_SpotAngle",
            "m_Shadows",
            "m_BakingOutput",
            "m_Cookie",
        ],
        &["m_Color"],
    ),
    sig(45, "Skybox", Some("Skybox"), &["m_CustomSkybox"], &[]),
    sig(
        119,
        "Projector",
        Some("Projector"),
        &[
            "m_NearClipPlane",
            "m_FarClipPlane",
            "m_FieldOfView",
            "m_AspectRatio",
            "m_Orthographic",
            "m_OrthographicSize",
            "m_Material",
        ],
        &[],
    ),
    sig(
        81,
        "AudioListener",
        Some("AudioListener"),
        &["m_AudioListener"],
        &[],
    ),
    sig(
        82,
        "AudioSource",
        Some("AudioSource"),
        &[
            "m_audioClip",
            "OutputAudioMixerGroup",
            "m_PlayOnAwake",
            "m_Volume",
            "m_Pitch",
            "Loop",
            "rolloffMode",
            "MinDistance",
            "MaxDistance",
            "panLevelCustomCurve",
        ],
        &[],
    ),
    sig(
        181,
        "AudioFilter",
        Some("AudioFilter"),
        &[],
        &["m_DryLevel", "m_WetMix", "m_CutoffFrequency"],
    ),
    sig(
        164,
        "AudioReverbFilter",
        Some("AudioFilter"),
        &["m_ReverbPreset", "m_Room", "m_DecayTime", "m_Reverb"],
        &[],
    ),
    sig(
        165,
        "AudioHighPassFilter",
        Some("AudioFilter"),
        &["m_HighpassResonanceQ", "highpassResonanceQ"],
        &["cutoffFrequency"],
    ),
    sig(
        166,
        "AudioChorusFilter",
        Some("AudioFilter"),
        &[
            "m_DryMix",
            "m_WetMix1",
            "m_WetMix2",
            "m_WetMix3",
            "m_Delay",
            "m_Rate",
            "m_Depth",
        ],
        &[],
    ),
    sig(
        167,
        "AudioReverbZone",
        Some("AudioReverbZone"),
        &["m_MinDistance", "m_MaxDistance", "m_ReverbPreset", "m_Room"],
        &[],
    ),
    sig(
        168,
        "AudioEchoFilter",
        Some("AudioFilter"),
        &["m_Delay", "m_DecayRatio", "m_WetMix", "m_DryMix"],
        &[],
    ),
    sig(
        169,
        "AudioLowPassFilter",
        Some("AudioFilter"),
        &["m_LowpassResonanceQ", "lowpassResonanceQ"],
        &["cutoffFrequency"],
    ),
    sig(
        170,
        "AudioDistortionFilter",
        Some("AudioFilter"),
        &["m_DistortionLevel"],
        &[],
    ),
    sig(
        54,
        "Rigidbody",
        Some("Rigidbody"),
        &[
            "m_UseGravity",
            "m_IsKinematic",
            "m_DetectCollisions",
            "m_CollisionDetection",
            "m_Constraints",
        ],
        &["m_Mass", "m_Drag", "m_AngularDrag", "m_Interpolate"],
    ),
    sig(
        50,
        "Rigidbody2D",
        Some("Rigidbody2D"),
        &[
            "m_GravityScale",
            "m_SleepingMode",
            "m_Simulated",
            "m_UseAutoMass",
        ],
        &["m_Mass", "m_Drag", "m_AngularDrag", "m_Interpolate"],
    ),
    sig(
        171741748,
        "ArticulationBody",
        Some("ArticulationBody"),
        &[
            "m_Immovable",
            "m_UseGravity",
            "m_AnchorPosition",
            "m_AnchorRotation",
            "m_JointType",
            "m_LinearDamping",
            "m_AngularDamping",
        ],
        &[],
    ),
    sig(
        56,
        "Collider",
        Some("Collider"),
        &[],
        &[
            "m_IsTrigger",
            "m_ProvidesContacts",
            "m_IncludeLayers",
            "m_ExcludeLayers",
            "m_LayerOverridePriority",
        ],
    ),
    sig(
        53,
        "Collider2D",
        Some("Collider2D"),
        &[],
        &[
            "m_UsedByComposite",
            "m_UsedByEffector",
            "m_Density",
            "m_Offset",
            "m_CallbackLayers",
            "m_ContactCaptureLayers",
        ],
    ),
    sig(
        65,
        "BoxCollider",
        Some("Collider"),
        &["m_Size"],
        &["m_Center"],
    ),
    sig(
        135,
        "SphereCollider",
        Some("Collider"),
        &["m_Radius"],
        &["m_Center"],
    ),
    sig(
        136,
        "CapsuleCollider",
        Some("Collider"),
        &["m_Radius", "m_Height", "m_Direction"],
        &["m_Center"],
    ),
    sig(
        64,
        "MeshCollider",
        Some("Collider"),
        &["m_Convex", "m_CookingOptions"],
        &["m_Mesh"],
    ),
    sig(
        143,
        "CharacterController",
        Some("Collider"),
        &[
            "m_SlopeLimit",
            "m_StepOffset",
            "m_SkinWidth",
            "m_MinMoveDistance",
            "m_Radius",
            "m_Height",
        ],
        &["m_Center"],
    ),
    sig(
        154,
        "TerrainCollider",
        Some("Collider"),
        &["m_TerrainData", "m_EnableTreeColliders"],
        &[],
    ),
    sig(
        146,
        "WheelCollider",
        Some("Collider"),
        &[
            "m_SuspensionDistance",
            "m_ForceAppPointDistance",
            "m_SuspensionSpring",
            "m_ForwardFriction",
            "m_SidewaysFriction",
        ],
        &["m_Center", "m_Radius", "m_Mass"],
    ),
    sig(
        58,
        "CircleCollider2D",
        Some("Collider2D"),
        &["m_Radius"],
        &["m_Offset"],
    ),
    sig(
        61,
        "BoxCollider2D",
        Some("Collider2D"),
        &["m_Size", "m_EdgeRadius", "m_AutoTiling"],
        &["m_Offset"],
    ),
    sig(
        60,
        "PolygonCollider2D",
        Some("Collider2D"),
        &["m_Points", "m_PathCount"],
        &[],
    ),
    sig(
        68,
        "EdgeCollider2D",
        Some("Collider2D"),
        &["m_Points", "m_EdgeRadius"],
        &[],
    ),
    sig(
        70,
        "CapsuleCollider2D",
        Some("Collider2D"),
        &["m_Size", "m_Direction", "m_CapsuleDirection"],
        &["m_Offset"],
    ),
    sig(
        66,
        "CompositeCollider2D",
        Some("Collider2D"),
        &[
            "m_GeometryType",
            "m_GenerationType",
            "m_VertexDistance",
            "m_OffsetDistance",
        ],
        &[],
    ),
    sig(
        19719996,
        "TilemapCollider2D",
        Some("Collider2D"),
        &["m_MaximumTileChangeCount", "m_ExtrusionFactor"],
        &[],
    ),
    sig(
        893571522,
        "CustomCollider2D",
        Some("Collider2D"),
        &["m_CustomShapes"],
        &[],
    ),
    sig(
        57,
        "Joint",
        Some("Joint"),
        &[],
        &[
            "m_ConnectedBody",
            "m_Anchor",
            "m_Axis",
            "m_BreakForce",
            "m_BreakTorque",
        ],
    ),
    sig(
        59,
        "HingeJoint",
        Some("Joint"),
        &[
            "m_UseSpring",
            "m_Spring",
            "m_UseMotor",
            "m_Motor",
            "m_UseLimits",
            "m_Limits",
        ],
        &[],
    ),
    sig(
        138,
        "FixedJoint",
        Some("Joint"),
        &["m_EnableCollision", "m_EnablePreprocessing"],
        &[],
    ),
    sig(
        144,
        "CharacterJoint",
        Some("Joint"),
        &[
            "m_SwingAxis",
            "m_LowTwistLimit",
            "m_HighTwistLimit",
            "m_Swing1Limit",
            "m_Swing2Limit",
        ],
        &[],
    ),
    sig(
        145,
        "SpringJoint",
        Some("Joint"),
        &[
            "m_Spring",
            "m_Damper",
            "m_MinDistance",
            "m_MaxDistance",
            "m_Tolerance",
        ],
        &[],
    ),
    sig(
        153,
        "ConfigurableJoint",
        Some("Joint"),
        &[
            "m_XMotion",
            "m_YMotion",
            "m_ZMotion",
            "m_AngularXMotion",
            "m_AngularYMotion",
            "m_AngularZMotion",
            "m_TargetPosition",
            "m_TargetVelocity",
            "m_XDrive",
            "m_SlerpDrive",
        ],
        &[],
    ),
    sig(
        230,
        "Joint2D",
        Some("Joint2D"),
        &[],
        &[
            "m_ConnectedRigidBody",
            "m_AutoConfigureConnectedAnchor",
            "m_Anchor",
            "m_ConnectedAnchor",
            "m_BreakForce",
            "m_BreakTorque",
        ],
    ),
    sig(
        231,
        "SpringJoint2D",
        Some("Joint2D"),
        &["m_DampingRatio", "m_Frequency", "m_Distance"],
        &[],
    ),
    sig(
        232,
        "DistanceJoint2D",
        Some("Joint2D"),
        &["m_MaxDistanceOnly", "m_AutoConfigureDistance", "m_Distance"],
        &[],
    ),
    sig(
        233,
        "HingeJoint2D",
        Some("Joint2D"),
        &["m_UseMotor", "m_Motor", "m_UseLimits", "m_AngleLimits"],
        &[],
    ),
    sig(
        234,
        "SliderJoint2D",
        Some("Joint2D"),
        &[
            "m_Angle",
            "m_UseMotor",
            "m_Motor",
            "m_UseLimits",
            "m_TranslationLimits",
        ],
        &[],
    ),
    sig(
        235,
        "WheelJoint2D",
        Some("Joint2D"),
        &["m_Suspension", "m_UseMotor", "m_Motor"],
        &[],
    ),
    sig(
        254,
        "RelativeJoint2D",
        Some("Joint2D"),
        &["m_MaxForce", "m_MaxTorque", "m_CorrectionScale"],
        &[],
    ),
    sig(
        255,
        "FixedJoint2D",
        Some("Joint2D"),
        &["m_DampingRatio", "m_Frequency"],
        &[],
    ),
    sig(
        256,
        "FrictionJoint2D",
        Some("Joint2D"),
        &["m_MaxForce", "m_MaxTorque"],
        &[],
    ),
    sig(
        257,
        "TargetJoint2D",
        Some("Joint2D"),
        &["m_Target", "m_MaxForce", "m_DampingRatio", "m_Frequency"],
        &[],
    ),
    sig(
        75,
        "ConstantForce",
        Some("ConstantForce"),
        &["m_Force", "m_RelativeForce", "m_Torque", "m_RelativeTorque"],
        &[],
    ),
    sig(
        247,
        "ConstantForce2D",
        Some("ConstantForce2D"),
        &["m_Force", "m_RelativeForce", "m_Torque"],
        &[],
    ),
    sig(
        248,
        "Effector2D",
        Some("Effector2D"),
        &[],
        &["m_UseColliderMask", "m_ColliderMask"],
    ),
    sig(
        249,
        "AreaEffector2D",
        Some("Effector2D"),
        &[
            "m_ForceAngle",
            "m_ForceMagnitude",
            "m_ForceVariation",
            "m_Drag",
            "m_AngularDrag",
        ],
        &[],
    ),
    sig(
        250,
        "PointEffector2D",
        Some("Effector2D"),
        &["m_ForceMagnitude", "m_ForceVariation", "m_DistanceScale"],
        &[],
    ),
    sig(
        251,
        "PlatformEffector2D",
        Some("Effector2D"),
        &[
            "m_RotationalOffset",
            "m_UseOneWay",
            "m_SurfaceArc",
            "m_UseSideFriction",
            "m_SideArc",
        ],
        &[],
    ),
    sig(
        252,
        "SurfaceEffector2D",
        Some("Effector2D"),
        &[
            "m_Speed",
            "m_SpeedVariation",
            "m_ForceScale",
            "m_UseContactForce",
        ],
        &[],
    ),
    sig(
        253,
        "BuoyancyEffector2D",
        Some("Effector2D"),
        &[
            "m_SurfaceLevel",
            "m_Density",
            "m_LinearDrag",
            "m_AngularDrag",
            "m_FlowAngle",
            "m_FlowMagnitude",
        ],
        &[],
    ),
    sig(
        95,
        "Animator",
        Some("Animator"),
        &[
            "m_Controller",
            "m_Avatar",
            "m_ApplyRootMotion",
            "m_UpdateMode",
            "m_HasTransformHierarchy",
            "m_CullingMode",
        ],
        &[],
    ),
    sig(
        111,
        "Animation",
        Some("Animation"),
        &[
            "m_Animation",
            "m_Animations",
            "m_WrapMode",
            "m_PlayAutomatically",
            "m_AnimatePhysics",
        ],
        &[],
    ),
    sig(
        320,
        "PlayableDirector",
        Some("PlayableDirector"),
        &[
            "m_PlayableAsset",
            "m_InitialState",
            "m_WrapMode",
            "m_DirectorUpdateMode",
            "m_InitialTime",
        ],
        &[],
    ),
    sig(
        198,
        "ParticleSystem",
        Some("ParticleSystem"),
        &[
            "InitialModule",
            "ShapeModule",
            "EmissionModule",
            "VelocityModule",
            "ColorModule",
            "SizeModule",
            "RotationModule",
            "NoiseModule",
            "SubModule",
            "LightsModule",
            "TrailModule",
        ],
        &[],
    ),
    sig(
        330,
        "ParticleSystemForceField",
        Some("ParticleSystemForceField"),
        &[
            "m_Shape",
            "m_StartRange",
            "m_EndRange",
            "m_DirectionX",
            "m_DirectionY",
            "m_DirectionZ",
            "m_Gravity",
            "m_RotationSpeed",
        ],
        &[],
    ),
    sig(
        102,
        "TextMesh",
        Some("TextMesh"),
        &[
            "m_Text",
            "m_OffsetZ",
            "m_CharacterSize",
            "m_LineSpacing",
            "m_Anchor",
            "m_Alignment",
            "m_TabSize",
            "m_FontSize",
            "m_FontStyle",
            "m_RichText",
        ],
        &[],
    ),
    sig(
        222,
        "CanvasRenderer",
        Some("CanvasRenderer"),
        &["m_CullTransparentMesh"],
        &[],
    ),
    sig(
        223,
        "Canvas",
        Some("Canvas"),
        &[
            "m_RenderMode",
            "m_Camera",
            "m_PixelPerfect",
            "m_PlaneDistance",
            "m_SortingOrder",
            "m_TargetDisplay",
        ],
        &[],
    ),
    sig(
        225,
        "CanvasGroup",
        Some("CanvasGroup"),
        &[
            "m_Alpha",
            "m_Interactable",
            "m_BlocksRaycasts",
            "m_IgnoreParentGroups",
        ],
        &[],
    ),
    sig(
        156049354,
        "Grid",
        Some("Grid"),
        &["m_CellSize", "m_CellGap", "m_CellLayout", "m_CellSwizzle"],
        &[],
    ),
    sig(
        1742807556,
        "GridLayout",
        Some("GridLayout"),
        &["m_CellSize", "m_CellGap", "m_CellLayout", "m_CellSwizzle"],
        &[],
    ),
    sig(
        1839735485,
        "Tilemap",
        Some("Tilemap"),
        &[
            "m_AnimationFrameRate",
            "m_Color",
            "m_TileAnchor",
            "m_Tiles",
            "m_TileArray",
        ],
        &[],
    ),
    sig(
        205,
        "LODGroup",
        Some("LODGroup"),
        &[
            "m_LocalReferencePoint",
            "m_Size",
            "m_FadeMode",
            "m_LODs",
            "m_AnimateCrossFading",
        ],
        &[],
    ),
    sig(
        210,
        "SortingGroup",
        Some("SortingGroup"),
        &["m_SortingLayerID", "m_SortingOrder", "m_SortAtRoot"],
        &[],
    ),
    sig(
        215,
        "ReflectionProbe",
        Some("ReflectionProbe"),
        &[
            "m_Type",
            "m_Mode",
            "m_RefreshMode",
            "m_TimeSlicingMode",
            "m_Resolution",
            "m_BoxSize",
            "m_BoxOffset",
            "m_CullingMask",
        ],
        &[],
    ),
    sig(
        220,
        "LightProbeGroup",
        Some("LightProbeGroup"),
        &["m_SourcePositions"],
        &[],
    ),
    sig(
        259,
        "LightProbeProxyVolume",
        Some("LightProbeProxyVolume"),
        &[
            "m_BoundingBoxMode",
            "m_ResolutionMode",
            "m_ProbePositionMode",
            "m_GridResolutionX",
            "m_GridResolutionY",
            "m_GridResolutionZ",
        ],
        &[],
    ),
    sig(
        41,
        "OcclusionPortal",
        Some("OcclusionPortal"),
        &["m_Open"],
        &[],
    ),
    sig(
        192,
        "OcclusionArea",
        Some("OcclusionArea"),
        &["m_Size", "m_Center"],
        &[],
    ),
    sig(
        182,
        "WindZone",
        Some("WindZone"),
        &[
            "m_Mode",
            "m_Radius",
            "m_WindMain",
            "m_WindTurbulence",
            "m_WindPulseMagnitude",
            "m_WindPulseFrequency",
        ],
        &[],
    ),
    sig(
        183,
        "Cloth",
        Some("Cloth"),
        &[
            "m_StretchingStiffness",
            "m_BendingStiffness",
            "m_UseTethers",
            "m_UseGravity",
            "m_Damping",
            "m_ExternalAcceleration",
            "m_Coefficients",
        ],
        &[],
    ),
    sig(
        218,
        "Terrain",
        Some("Terrain"),
        &[
            "m_TerrainData",
            "m_TreeDistance",
            "m_DetailObjectDistance",
            "m_HeightmapPixelError",
            "m_BasemapsDistance",
            "m_DrawInstanced",
        ],
        &[],
    ),
    sig(
        195,
        "NavMeshAgent",
        Some("NavMeshAgent"),
        &[
            "m_AgentTypeID",
            "m_Radius",
            "m_Speed",
            "m_Acceleration",
            "m_AngularSpeed",
            "m_StoppingDistance",
            "m_AutoTraverseOffMeshLink",
        ],
        &[],
    ),
    sig(
        208,
        "NavMeshObstacle",
        Some("NavMeshObstacle"),
        &[
            "m_Shape",
            "m_Extents",
            "m_MoveThreshold",
            "m_TimeToStationary",
            "m_Carve",
        ],
        &[],
    ),
    sig(
        191,
        "OffMeshLink",
        Some("OffMeshLink"),
        &[
            "m_Start",
            "m_End",
            "m_CostOverride",
            "m_BiDirectional",
            "m_Activated",
            "m_Area",
        ],
        &[],
    ),
    sig(
        328,
        "VideoPlayer",
        Some("VideoPlayer"),
        &[
            "m_Source",
            "m_VideoClip",
            "m_Url",
            "m_PlayOnAwake",
            "m_WaitForFirstFrame",
            "m_Looping",
            "m_PlaybackSpeed",
            "m_TargetCamera",
            "m_TargetTexture",
            "m_TargetRenderer",
            "m_TargetMaterialProperty",
        ],
        &[],
    ),
    sig(
        331,
        "SpriteMask",
        Some("SpriteMask"),
        &[
            "m_AlphaCutoff",
            "m_RangeStart",
            "m_RangeEnd",
            "m_SortingLayer",
        ],
        &["m_Sprite"],
    ),
    sig(
        2083052967,
        "VisualEffect",
        Some("VisualEffect"),
        &[
            "m_Asset",
            "m_StartSeed",
            "m_ResetSeedOnPlay",
            "m_InitialEventName",
        ],
        &[],
    ),
    sig(
        895512359,
        "AimConstraint",
        Some("Constraint"),
        &[
            "m_RotationAtRest",
            "m_RotationOffset",
            "m_AimVector",
            "m_UpVector",
            "m_WorldUpVector",
            "m_WorldUpObject",
            "m_Sources",
        ],
        &[],
    ),
    sig(
        1183024399,
        "LookAtConstraint",
        Some("Constraint"),
        &["m_Roll", "m_UseUpObject", "m_WorldUpObject", "m_Sources"],
        &[],
    ),
    sig(
        1773428102,
        "ParentConstraint",
        Some("Constraint"),
        &[
            "m_TranslationAtRest",
            "m_RotationAtRest",
            "m_TranslationOffsets",
            "m_RotationOffsets",
            "m_AffectTranslationX",
            "m_AffectRotationX",
            "m_Sources",
        ],
        &[],
    ),
    sig(
        1818360608,
        "PositionConstraint",
        Some("Constraint"),
        &[
            "m_TranslationAtRest",
            "m_TranslationOffset",
            "m_AffectTranslationX",
            "m_AffectTranslationY",
            "m_AffectTranslationZ",
            "m_Sources",
        ],
        &[],
    ),
    sig(
        1818360609,
        "RotationConstraint",
        Some("Constraint"),
        &[
            "m_RotationAtRest",
            "m_RotationOffset",
            "m_AffectRotationX",
            "m_AffectRotationY",
            "m_AffectRotationZ",
            "m_Sources",
        ],
        &[],
    ),
    sig(
        1818360610,
        "ScaleConstraint",
        Some("Constraint"),
        &[
            "m_ScaleAtRest",
            "m_ScaleOffset",
            "m_AffectScalingX",
            "m_AffectScalingY",
            "m_AffectScalingZ",
            "m_Sources",
        ],
        &[],
    ),
];

const fn sig(
    class_id: i32,
    name: &'static str,
    family: Option<&'static str>,
    strong_fields: &'static [&'static str],
    weak_fields: &'static [&'static str],
) -> BuiltinComponentSignature {
    BuiltinComponentSignature {
        class_id,
        name,
        family,
        strong_fields,
        weak_fields,
    }
}

pub(crate) fn infer_component_from_override_groups(
    groups: &[&[&PropertyOverride]],
) -> Option<ComponentInferenceResult> {
    infer_component_from_property_paths(
        groups
            .iter()
            .flat_map(|group| group.iter().map(|ovr| ovr.property_path.as_str())),
    )
}

pub(crate) fn infer_component_from_property_paths<'a, I>(
    paths: I,
) -> Option<ComponentInferenceResult>
where
    I: IntoIterator<Item = &'a str>,
{
    let index = field_index();
    let mut scores: HashMap<&'static str, CandidateScore> = HashMap::new();

    for path in paths {
        let field = normalized_root_field(path);
        if field.is_empty() || IGNORED_FIELDS.contains(&field) {
            continue;
        }
        let Some(candidates) = index.get(field) else {
            continue;
        };
        for candidate in candidates {
            let score = scores
                .entry(candidate.name)
                .or_insert_with(|| CandidateScore {
                    class_id: candidate.class_id,
                    name: candidate.name,
                    family: candidate.family,
                    is_family: candidate.is_family,
                    strong_hits: 0,
                    weak_hits: 0,
                    score: 0,
                    evidence: Vec::new(),
                });
            match candidate.strength {
                FieldStrength::Strong => {
                    score.strong_hits += 1;
                    score.score += if candidate.is_family { 3 } else { 4 };
                }
                FieldStrength::Weak => {
                    score.weak_hits += 1;
                    score.score += 1;
                }
            }
            push_evidence(&mut score.evidence, path);
        }
    }

    let mut ranked = scores.values().collect::<Vec<_>>();
    ranked.sort_by_key(|score| std::cmp::Reverse(score.score));
    let best = ranked.first().copied()?;
    if best.score == 0 || (best.strong_hits == 0 && best.weak_hits < 2) {
        return None;
    }
    if ranked.iter().skip(1).any(|score| score.score == best.score) {
        return None;
    }
    if ranked.iter().skip(1).any(|score| {
        score.strong_hits > 0 && !same_component_family(best, score) && best.score < score.score + 4
    }) {
        return None;
    }

    Some(ComponentInferenceResult {
        component_type: best.name.to_string(),
        inferred_class_id: Some(best.class_id),
        reason_code: if best.is_family {
            "propertyPathBuiltinFamily".to_string()
        } else {
            "propertyPathUniqueBuiltinComponent".to_string()
        },
        evidence: best.evidence.clone(),
    })
}

pub(crate) fn inference_for_known_class_id(
    class_id: i32,
    reason_code: &str,
    evidence: Vec<String>,
) -> ComponentInferenceResult {
    ComponentInferenceResult {
        component_type: unity_class_name(class_id).to_string(),
        inferred_class_id: Some(class_id),
        reason_code: reason_code.to_string(),
        evidence,
    }
}

fn field_index() -> &'static HashMap<&'static str, Vec<IndexedCandidate>> {
    static INDEX: OnceLock<HashMap<&'static str, Vec<IndexedCandidate>>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index: HashMap<&'static str, Vec<IndexedCandidate>> = HashMap::new();
        for signature in BUILTIN_COMPONENT_SIGNATURES {
            let is_family = is_family_anchor(signature.name);
            for field in signature.strong_fields {
                index.entry(field).or_default().push(IndexedCandidate {
                    class_id: signature.class_id,
                    name: signature.name,
                    family: signature.family,
                    is_family,
                    strength: FieldStrength::Strong,
                });
            }
            for field in signature.weak_fields {
                index.entry(field).or_default().push(IndexedCandidate {
                    class_id: signature.class_id,
                    name: signature.name,
                    family: signature.family,
                    is_family,
                    strength: FieldStrength::Weak,
                });
            }
        }
        index
    })
}

fn is_family_anchor(name: &str) -> bool {
    // These signatures represent "generic builtin buckets" where the
    // property paths are intentionally broad and may only narrow to the
    // family, not a specific subtype. `Transform` is deliberately excluded:
    // prefab overrides like `m_LocalPosition` should surface as a concrete
    // Transform panel, not a family-level diagnostic.
    matches!(
        name,
        "AudioFilter" | "Collider" | "Collider2D" | "Effector2D" | "Joint" | "Joint2D" | "Renderer"
    )
}

fn same_component_family(left: &CandidateScore, right: &CandidateScore) -> bool {
    if left.name == right.name {
        return true;
    }
    match (left.family, right.family) {
        (Some(left_family), Some(right_family)) if left_family == right_family => true,
        (Some(left_family), _) if left_family == right.name => true,
        (_, Some(right_family)) if right_family == left.name => true,
        _ => false,
    }
}

fn normalized_root_field(path: &str) -> &str {
    let trimmed = path.trim();
    let before_dot = trimmed.split('.').next().unwrap_or(trimmed);
    before_dot.split('[').next().unwrap_or(before_dot).trim()
}

fn push_evidence(evidence: &mut Vec<String>, path: &str) {
    if evidence.len() >= EVIDENCE_LIMIT {
        return;
    }
    if evidence.iter().any(|existing| existing == path) {
        return;
    }
    evidence.push(path.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_representative_builtin_components() {
        assert_eq!(
            infer_component_from_property_paths(["m_LocalPosition.x"])
                .unwrap()
                .component_type,
            "Transform"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_Materials.Array.data[0]"])
                .unwrap()
                .component_type,
            "Renderer"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_ClearFlags"])
                .unwrap()
                .component_type,
            "Camera"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_audioClip", "m_Volume"])
                .unwrap()
                .component_type,
            "AudioSource"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_IsKinematic"])
                .unwrap()
                .component_type,
            "Rigidbody"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_Controller"])
                .unwrap()
                .component_type,
            "Animator"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_RenderMode", "m_PixelPerfect"])
                .unwrap()
                .component_type,
            "Canvas"
        );
        assert_eq!(
            infer_component_from_property_paths(["m_Source", "m_VideoClip"])
                .unwrap()
                .component_type,
            "VideoPlayer"
        );
    }

    #[test]
    fn ignores_generic_fields_and_conflicts() {
        assert!(infer_component_from_property_paths(["m_Enabled"]).is_none());
        assert!(infer_component_from_property_paths(["m_ObjectHideFlags"]).is_none());
        assert!(infer_component_from_property_paths([
            "m_LocalPosition.x",
            "m_Materials.Array.data[0]"
        ])
        .is_none());
    }

    #[test]
    fn resolves_specific_component_when_specific_evidence_breaks_a_family_tie() {
        let inferred =
            infer_component_from_property_paths(["m_Materials.Array.data[0]", "m_Sprite"])
                .expect("sprite field should beat renderer-family evidence");
        assert_eq!(inferred.component_type, "SpriteRenderer");
        assert_eq!(inferred.inferred_class_id, Some(212));
    }

    #[test]
    fn caps_evidence_and_deduplicates_paths() {
        let inferred = infer_component_from_property_paths([
            "m_audioClip",
            "m_audioClip",
            "m_Volume",
            "m_Pitch",
            "Loop",
            "MinDistance",
            "MaxDistance",
        ])
        .expect("audio source");
        assert_eq!(inferred.evidence.len(), EVIDENCE_LIMIT);
        assert_eq!(
            inferred
                .evidence
                .iter()
                .filter(|path| path.as_str() == "m_audioClip")
                .count(),
            1
        );
    }

    #[test]
    fn field_index_is_singleton() {
        assert!(std::ptr::eq(field_index(), field_index()));
    }
}
