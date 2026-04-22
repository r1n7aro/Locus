#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnityClassInfo {
    pub(crate) class_id: i32,
    pub(crate) name: &'static str,
}

// Unity 2022.3 YAML Class ID Reference:
// https://docs.unity3d.com/2022.3/Documentation/Manual/ClassIDReference.html
pub(crate) const UNITY_2022_3_CLASSES: &[UnityClassInfo] = &[
    UnityClassInfo {
        class_id: 0,
        name: "Object",
    },
    UnityClassInfo {
        class_id: 1,
        name: "GameObject",
    },
    UnityClassInfo {
        class_id: 2,
        name: "Component",
    },
    UnityClassInfo {
        class_id: 3,
        name: "LevelGameManager",
    },
    UnityClassInfo {
        class_id: 4,
        name: "Transform",
    },
    UnityClassInfo {
        class_id: 5,
        name: "TimeManager",
    },
    UnityClassInfo {
        class_id: 6,
        name: "GlobalGameManager",
    },
    UnityClassInfo {
        class_id: 8,
        name: "Behaviour",
    },
    UnityClassInfo {
        class_id: 9,
        name: "GameManager",
    },
    UnityClassInfo {
        class_id: 11,
        name: "AudioManager",
    },
    UnityClassInfo {
        class_id: 13,
        name: "InputManager",
    },
    UnityClassInfo {
        class_id: 18,
        name: "EditorExtension",
    },
    UnityClassInfo {
        class_id: 19,
        name: "Physics2DSettings",
    },
    UnityClassInfo {
        class_id: 20,
        name: "Camera",
    },
    UnityClassInfo {
        class_id: 21,
        name: "Material",
    },
    UnityClassInfo {
        class_id: 23,
        name: "MeshRenderer",
    },
    UnityClassInfo {
        class_id: 25,
        name: "Renderer",
    },
    UnityClassInfo {
        class_id: 27,
        name: "Texture",
    },
    UnityClassInfo {
        class_id: 28,
        name: "Texture2D",
    },
    UnityClassInfo {
        class_id: 29,
        name: "OcclusionCullingSettings",
    },
    UnityClassInfo {
        class_id: 30,
        name: "GraphicsSettings",
    },
    UnityClassInfo {
        class_id: 33,
        name: "MeshFilter",
    },
    UnityClassInfo {
        class_id: 41,
        name: "OcclusionPortal",
    },
    UnityClassInfo {
        class_id: 43,
        name: "Mesh",
    },
    UnityClassInfo {
        class_id: 45,
        name: "Skybox",
    },
    UnityClassInfo {
        class_id: 47,
        name: "QualitySettings",
    },
    UnityClassInfo {
        class_id: 48,
        name: "Shader",
    },
    UnityClassInfo {
        class_id: 49,
        name: "TextAsset",
    },
    UnityClassInfo {
        class_id: 50,
        name: "Rigidbody2D",
    },
    UnityClassInfo {
        class_id: 53,
        name: "Collider2D",
    },
    UnityClassInfo {
        class_id: 54,
        name: "Rigidbody",
    },
    UnityClassInfo {
        class_id: 55,
        name: "PhysicsManager",
    },
    UnityClassInfo {
        class_id: 56,
        name: "Collider",
    },
    UnityClassInfo {
        class_id: 57,
        name: "Joint",
    },
    UnityClassInfo {
        class_id: 58,
        name: "CircleCollider2D",
    },
    UnityClassInfo {
        class_id: 59,
        name: "HingeJoint",
    },
    UnityClassInfo {
        class_id: 60,
        name: "PolygonCollider2D",
    },
    UnityClassInfo {
        class_id: 61,
        name: "BoxCollider2D",
    },
    UnityClassInfo {
        class_id: 62,
        name: "PhysicsMaterial2D",
    },
    UnityClassInfo {
        class_id: 64,
        name: "MeshCollider",
    },
    UnityClassInfo {
        class_id: 65,
        name: "BoxCollider",
    },
    UnityClassInfo {
        class_id: 66,
        name: "CompositeCollider2D",
    },
    UnityClassInfo {
        class_id: 68,
        name: "EdgeCollider2D",
    },
    UnityClassInfo {
        class_id: 70,
        name: "CapsuleCollider2D",
    },
    UnityClassInfo {
        class_id: 72,
        name: "ComputeShader",
    },
    UnityClassInfo {
        class_id: 74,
        name: "AnimationClip",
    },
    UnityClassInfo {
        class_id: 75,
        name: "ConstantForce",
    },
    UnityClassInfo {
        class_id: 78,
        name: "TagManager",
    },
    UnityClassInfo {
        class_id: 81,
        name: "AudioListener",
    },
    UnityClassInfo {
        class_id: 82,
        name: "AudioSource",
    },
    UnityClassInfo {
        class_id: 83,
        name: "AudioClip",
    },
    UnityClassInfo {
        class_id: 84,
        name: "RenderTexture",
    },
    UnityClassInfo {
        class_id: 86,
        name: "CustomRenderTexture",
    },
    UnityClassInfo {
        class_id: 89,
        name: "Cubemap",
    },
    UnityClassInfo {
        class_id: 90,
        name: "Avatar",
    },
    UnityClassInfo {
        class_id: 91,
        name: "AnimatorController",
    },
    UnityClassInfo {
        class_id: 93,
        name: "RuntimeAnimatorController",
    },
    UnityClassInfo {
        class_id: 94,
        name: "ShaderNameRegistry",
    },
    UnityClassInfo {
        class_id: 95,
        name: "Animator",
    },
    UnityClassInfo {
        class_id: 96,
        name: "TrailRenderer",
    },
    UnityClassInfo {
        class_id: 98,
        name: "DelayedCallManager",
    },
    UnityClassInfo {
        class_id: 102,
        name: "TextMesh",
    },
    UnityClassInfo {
        class_id: 104,
        name: "RenderSettings",
    },
    UnityClassInfo {
        class_id: 108,
        name: "Light",
    },
    UnityClassInfo {
        class_id: 109,
        name: "ShaderInclude",
    },
    UnityClassInfo {
        class_id: 110,
        name: "BaseAnimationTrack",
    },
    UnityClassInfo {
        class_id: 111,
        name: "Animation",
    },
    UnityClassInfo {
        class_id: 114,
        name: "MonoBehaviour",
    },
    UnityClassInfo {
        class_id: 115,
        name: "MonoScript",
    },
    UnityClassInfo {
        class_id: 116,
        name: "MonoManager",
    },
    UnityClassInfo {
        class_id: 117,
        name: "Texture3D",
    },
    UnityClassInfo {
        class_id: 118,
        name: "NewAnimationTrack",
    },
    UnityClassInfo {
        class_id: 119,
        name: "Projector",
    },
    UnityClassInfo {
        class_id: 120,
        name: "LineRenderer",
    },
    UnityClassInfo {
        class_id: 121,
        name: "Flare",
    },
    UnityClassInfo {
        class_id: 122,
        name: "Halo",
    },
    UnityClassInfo {
        class_id: 123,
        name: "LensFlare",
    },
    UnityClassInfo {
        class_id: 124,
        name: "FlareLayer",
    },
    UnityClassInfo {
        class_id: 126,
        name: "NavMeshProjectSettings",
    },
    UnityClassInfo {
        class_id: 128,
        name: "Font",
    },
    UnityClassInfo {
        class_id: 129,
        name: "PlayerSettings",
    },
    UnityClassInfo {
        class_id: 130,
        name: "NamedObject",
    },
    UnityClassInfo {
        class_id: 134,
        name: "PhysicMaterial",
    },
    UnityClassInfo {
        class_id: 135,
        name: "SphereCollider",
    },
    UnityClassInfo {
        class_id: 136,
        name: "CapsuleCollider",
    },
    UnityClassInfo {
        class_id: 137,
        name: "SkinnedMeshRenderer",
    },
    UnityClassInfo {
        class_id: 138,
        name: "FixedJoint",
    },
    UnityClassInfo {
        class_id: 141,
        name: "BuildSettings",
    },
    UnityClassInfo {
        class_id: 142,
        name: "AssetBundle",
    },
    UnityClassInfo {
        class_id: 143,
        name: "CharacterController",
    },
    UnityClassInfo {
        class_id: 144,
        name: "CharacterJoint",
    },
    UnityClassInfo {
        class_id: 145,
        name: "SpringJoint",
    },
    UnityClassInfo {
        class_id: 146,
        name: "WheelCollider",
    },
    UnityClassInfo {
        class_id: 147,
        name: "ResourceManager",
    },
    UnityClassInfo {
        class_id: 150,
        name: "PreloadData",
    },
    UnityClassInfo {
        class_id: 152,
        name: "MovieTexture",
    },
    UnityClassInfo {
        class_id: 153,
        name: "ConfigurableJoint",
    },
    UnityClassInfo {
        class_id: 154,
        name: "TerrainCollider",
    },
    UnityClassInfo {
        class_id: 156,
        name: "TerrainData",
    },
    UnityClassInfo {
        class_id: 157,
        name: "LightmapSettings",
    },
    UnityClassInfo {
        class_id: 158,
        name: "WebCamTexture",
    },
    UnityClassInfo {
        class_id: 159,
        name: "EditorSettings",
    },
    UnityClassInfo {
        class_id: 162,
        name: "EditorUserSettings",
    },
    UnityClassInfo {
        class_id: 164,
        name: "AudioReverbFilter",
    },
    UnityClassInfo {
        class_id: 165,
        name: "AudioHighPassFilter",
    },
    UnityClassInfo {
        class_id: 166,
        name: "AudioChorusFilter",
    },
    UnityClassInfo {
        class_id: 167,
        name: "AudioReverbZone",
    },
    UnityClassInfo {
        class_id: 168,
        name: "AudioEchoFilter",
    },
    UnityClassInfo {
        class_id: 169,
        name: "AudioLowPassFilter",
    },
    UnityClassInfo {
        class_id: 170,
        name: "AudioDistortionFilter",
    },
    UnityClassInfo {
        class_id: 171,
        name: "SparseTexture",
    },
    UnityClassInfo {
        class_id: 180,
        name: "AudioBehaviour",
    },
    UnityClassInfo {
        class_id: 181,
        name: "AudioFilter",
    },
    UnityClassInfo {
        class_id: 182,
        name: "WindZone",
    },
    UnityClassInfo {
        class_id: 183,
        name: "Cloth",
    },
    UnityClassInfo {
        class_id: 184,
        name: "SubstanceArchive",
    },
    UnityClassInfo {
        class_id: 185,
        name: "ProceduralMaterial",
    },
    UnityClassInfo {
        class_id: 186,
        name: "ProceduralTexture",
    },
    UnityClassInfo {
        class_id: 187,
        name: "Texture2DArray",
    },
    UnityClassInfo {
        class_id: 188,
        name: "CubemapArray",
    },
    UnityClassInfo {
        class_id: 191,
        name: "OffMeshLink",
    },
    UnityClassInfo {
        class_id: 192,
        name: "OcclusionArea",
    },
    UnityClassInfo {
        class_id: 193,
        name: "Tree",
    },
    UnityClassInfo {
        class_id: 195,
        name: "NavMeshAgent",
    },
    UnityClassInfo {
        class_id: 196,
        name: "NavMeshSettings",
    },
    UnityClassInfo {
        class_id: 198,
        name: "ParticleSystem",
    },
    UnityClassInfo {
        class_id: 199,
        name: "ParticleSystemRenderer",
    },
    UnityClassInfo {
        class_id: 200,
        name: "ShaderVariantCollection",
    },
    UnityClassInfo {
        class_id: 205,
        name: "LODGroup",
    },
    UnityClassInfo {
        class_id: 206,
        name: "BlendTree",
    },
    UnityClassInfo {
        class_id: 207,
        name: "Motion",
    },
    UnityClassInfo {
        class_id: 208,
        name: "NavMeshObstacle",
    },
    UnityClassInfo {
        class_id: 210,
        name: "SortingGroup",
    },
    UnityClassInfo {
        class_id: 212,
        name: "SpriteRenderer",
    },
    UnityClassInfo {
        class_id: 213,
        name: "Sprite",
    },
    UnityClassInfo {
        class_id: 214,
        name: "CachedSpriteAtlas",
    },
    UnityClassInfo {
        class_id: 215,
        name: "ReflectionProbe",
    },
    UnityClassInfo {
        class_id: 218,
        name: "Terrain",
    },
    UnityClassInfo {
        class_id: 220,
        name: "LightProbeGroup",
    },
    UnityClassInfo {
        class_id: 221,
        name: "AnimatorOverrideController",
    },
    UnityClassInfo {
        class_id: 222,
        name: "CanvasRenderer",
    },
    UnityClassInfo {
        class_id: 223,
        name: "Canvas",
    },
    UnityClassInfo {
        class_id: 224,
        name: "RectTransform",
    },
    UnityClassInfo {
        class_id: 225,
        name: "CanvasGroup",
    },
    UnityClassInfo {
        class_id: 226,
        name: "BillboardAsset",
    },
    UnityClassInfo {
        class_id: 227,
        name: "BillboardRenderer",
    },
    UnityClassInfo {
        class_id: 228,
        name: "SpeedTreeWindAsset",
    },
    UnityClassInfo {
        class_id: 229,
        name: "AnchoredJoint2D",
    },
    UnityClassInfo {
        class_id: 230,
        name: "Joint2D",
    },
    UnityClassInfo {
        class_id: 231,
        name: "SpringJoint2D",
    },
    UnityClassInfo {
        class_id: 232,
        name: "DistanceJoint2D",
    },
    UnityClassInfo {
        class_id: 233,
        name: "HingeJoint2D",
    },
    UnityClassInfo {
        class_id: 234,
        name: "SliderJoint2D",
    },
    UnityClassInfo {
        class_id: 235,
        name: "WheelJoint2D",
    },
    UnityClassInfo {
        class_id: 236,
        name: "ClusterInputManager",
    },
    UnityClassInfo {
        class_id: 237,
        name: "BaseVideoTexture",
    },
    UnityClassInfo {
        class_id: 238,
        name: "NavMeshData",
    },
    UnityClassInfo {
        class_id: 240,
        name: "AudioMixer",
    },
    UnityClassInfo {
        class_id: 241,
        name: "AudioMixerController",
    },
    UnityClassInfo {
        class_id: 243,
        name: "AudioMixerGroupController",
    },
    UnityClassInfo {
        class_id: 244,
        name: "AudioMixerEffectController",
    },
    UnityClassInfo {
        class_id: 245,
        name: "AudioMixerSnapshotController",
    },
    UnityClassInfo {
        class_id: 246,
        name: "PhysicsUpdateBehaviour2D",
    },
    UnityClassInfo {
        class_id: 247,
        name: "ConstantForce2D",
    },
    UnityClassInfo {
        class_id: 248,
        name: "Effector2D",
    },
    UnityClassInfo {
        class_id: 249,
        name: "AreaEffector2D",
    },
    UnityClassInfo {
        class_id: 250,
        name: "PointEffector2D",
    },
    UnityClassInfo {
        class_id: 251,
        name: "PlatformEffector2D",
    },
    UnityClassInfo {
        class_id: 252,
        name: "SurfaceEffector2D",
    },
    UnityClassInfo {
        class_id: 253,
        name: "BuoyancyEffector2D",
    },
    UnityClassInfo {
        class_id: 254,
        name: "RelativeJoint2D",
    },
    UnityClassInfo {
        class_id: 255,
        name: "FixedJoint2D",
    },
    UnityClassInfo {
        class_id: 256,
        name: "FrictionJoint2D",
    },
    UnityClassInfo {
        class_id: 257,
        name: "TargetJoint2D",
    },
    UnityClassInfo {
        class_id: 258,
        name: "LightProbes",
    },
    UnityClassInfo {
        class_id: 259,
        name: "LightProbeProxyVolume",
    },
    UnityClassInfo {
        class_id: 271,
        name: "SampleClip",
    },
    UnityClassInfo {
        class_id: 272,
        name: "AudioMixerSnapshot",
    },
    UnityClassInfo {
        class_id: 273,
        name: "AudioMixerGroup",
    },
    UnityClassInfo {
        class_id: 290,
        name: "AssetBundleManifest",
    },
    UnityClassInfo {
        class_id: 300,
        name: "RuntimeInitializeOnLoadManager",
    },
    UnityClassInfo {
        class_id: 310,
        name: "UnityConnectSettings",
    },
    UnityClassInfo {
        class_id: 319,
        name: "AvatarMask",
    },
    UnityClassInfo {
        class_id: 320,
        name: "PlayableDirector",
    },
    UnityClassInfo {
        class_id: 328,
        name: "VideoPlayer",
    },
    UnityClassInfo {
        class_id: 329,
        name: "VideoClip",
    },
    UnityClassInfo {
        class_id: 330,
        name: "ParticleSystemForceField",
    },
    UnityClassInfo {
        class_id: 331,
        name: "SpriteMask",
    },
    UnityClassInfo {
        class_id: 363,
        name: "OcclusionCullingData",
    },
    UnityClassInfo {
        class_id: 1001,
        name: "PrefabInstance",
    },
    UnityClassInfo {
        class_id: 1002,
        name: "EditorExtensionImpl",
    },
    UnityClassInfo {
        class_id: 1003,
        name: "AssetImporter",
    },
    UnityClassInfo {
        class_id: 1005,
        name: "Mesh3DSImporter",
    },
    UnityClassInfo {
        class_id: 1006,
        name: "TextureImporter",
    },
    UnityClassInfo {
        class_id: 1007,
        name: "ShaderImporter",
    },
    UnityClassInfo {
        class_id: 1008,
        name: "ComputeShaderImporter",
    },
    UnityClassInfo {
        class_id: 1020,
        name: "AudioImporter",
    },
    UnityClassInfo {
        class_id: 1026,
        name: "HierarchyState",
    },
    UnityClassInfo {
        class_id: 1028,
        name: "AssetMetaData",
    },
    UnityClassInfo {
        class_id: 1029,
        name: "DefaultAsset",
    },
    UnityClassInfo {
        class_id: 1030,
        name: "DefaultImporter",
    },
    UnityClassInfo {
        class_id: 1031,
        name: "TextScriptImporter",
    },
    UnityClassInfo {
        class_id: 1032,
        name: "SceneAsset",
    },
    UnityClassInfo {
        class_id: 1034,
        name: "NativeFormatImporter",
    },
    UnityClassInfo {
        class_id: 1035,
        name: "MonoImporter",
    },
    UnityClassInfo {
        class_id: 1038,
        name: "LibraryAssetImporter",
    },
    UnityClassInfo {
        class_id: 1040,
        name: "ModelImporter",
    },
    UnityClassInfo {
        class_id: 1041,
        name: "FBXImporter",
    },
    UnityClassInfo {
        class_id: 1042,
        name: "TrueTypeFontImporter",
    },
    UnityClassInfo {
        class_id: 1045,
        name: "EditorBuildSettings",
    },
    UnityClassInfo {
        class_id: 1048,
        name: "InspectorExpandedState",
    },
    UnityClassInfo {
        class_id: 1049,
        name: "AnnotationManager",
    },
    UnityClassInfo {
        class_id: 1050,
        name: "PluginImporter",
    },
    UnityClassInfo {
        class_id: 1051,
        name: "EditorUserBuildSettings",
    },
    UnityClassInfo {
        class_id: 1055,
        name: "IHVImageFormatImporter",
    },
    UnityClassInfo {
        class_id: 1101,
        name: "AnimatorStateTransition",
    },
    UnityClassInfo {
        class_id: 1102,
        name: "AnimatorState",
    },
    UnityClassInfo {
        class_id: 1105,
        name: "HumanTemplate",
    },
    UnityClassInfo {
        class_id: 1107,
        name: "AnimatorStateMachine",
    },
    UnityClassInfo {
        class_id: 1108,
        name: "PreviewAnimationClip",
    },
    UnityClassInfo {
        class_id: 1109,
        name: "AnimatorTransition",
    },
    UnityClassInfo {
        class_id: 1110,
        name: "SpeedTreeImporter",
    },
    UnityClassInfo {
        class_id: 1111,
        name: "AnimatorTransitionBase",
    },
    UnityClassInfo {
        class_id: 1112,
        name: "SubstanceImporter",
    },
    UnityClassInfo {
        class_id: 1113,
        name: "LightmapParameters",
    },
    UnityClassInfo {
        class_id: 1120,
        name: "LightingDataAsset",
    },
    UnityClassInfo {
        class_id: 1124,
        name: "SketchUpImporter",
    },
    UnityClassInfo {
        class_id: 1125,
        name: "BuildReport",
    },
    UnityClassInfo {
        class_id: 1126,
        name: "PackedAssets",
    },
    UnityClassInfo {
        class_id: 1127,
        name: "VideoClipImporter",
    },
    UnityClassInfo {
        class_id: 100000,
        name: "int",
    },
    UnityClassInfo {
        class_id: 100001,
        name: "bool",
    },
    UnityClassInfo {
        class_id: 100002,
        name: "float",
    },
    UnityClassInfo {
        class_id: 100003,
        name: "MonoObject",
    },
    UnityClassInfo {
        class_id: 100004,
        name: "Collision",
    },
    UnityClassInfo {
        class_id: 100005,
        name: "Vector3f",
    },
    UnityClassInfo {
        class_id: 100006,
        name: "RootMotionData",
    },
    UnityClassInfo {
        class_id: 100007,
        name: "Collision2D",
    },
    UnityClassInfo {
        class_id: 100008,
        name: "AudioMixerLiveUpdateFloat",
    },
    UnityClassInfo {
        class_id: 100009,
        name: "AudioMixerLiveUpdateBool",
    },
    UnityClassInfo {
        class_id: 100010,
        name: "Polygon2D",
    },
    UnityClassInfo {
        class_id: 100011,
        name: "void",
    },
    UnityClassInfo {
        class_id: 19719996,
        name: "TilemapCollider2D",
    },
    UnityClassInfo {
        class_id: 41386430,
        name: "ImportLog",
    },
    UnityClassInfo {
        class_id: 73398921,
        name: "VFXRenderer",
    },
    UnityClassInfo {
        class_id: 156049354,
        name: "Grid",
    },
    UnityClassInfo {
        class_id: 156483287,
        name: "ScenesUsingAssets",
    },
    UnityClassInfo {
        class_id: 171741748,
        name: "ArticulationBody",
    },
    UnityClassInfo {
        class_id: 181963792,
        name: "Preset",
    },
    UnityClassInfo {
        class_id: 285090594,
        name: "IConstraint",
    },
    UnityClassInfo {
        class_id: 294290339,
        name: "AssemblyDefinitionReferenceImporter",
    },
    UnityClassInfo {
        class_id: 369655926,
        name: "AssetImportInProgressProxy",
    },
    UnityClassInfo {
        class_id: 382020655,
        name: "PluginBuildInfo",
    },
    UnityClassInfo {
        class_id: 387306366,
        name: "MemorySettings",
    },
    UnityClassInfo {
        class_id: 426301858,
        name: "EditorProjectAccess",
    },
    UnityClassInfo {
        class_id: 468431735,
        name: "PrefabImporter",
    },
    UnityClassInfo {
        class_id: 483693784,
        name: "TilemapRenderer",
    },
    UnityClassInfo {
        class_id: 612988286,
        name: "SpriteAtlasAsset",
    },
    UnityClassInfo {
        class_id: 638013454,
        name: "SpriteAtlasDatabase",
    },
    UnityClassInfo {
        class_id: 641289076,
        name: "AudioBuildInfo",
    },
    UnityClassInfo {
        class_id: 644342135,
        name: "CachedSpriteAtlasRuntimeData",
    },
    UnityClassInfo {
        class_id: 662584278,
        name: "AssemblyDefinitionReferenceAsset",
    },
    UnityClassInfo {
        class_id: 668709126,
        name: "BuiltAssetBundleInfoSet",
    },
    UnityClassInfo {
        class_id: 687078895,
        name: "SpriteAtlas",
    },
    UnityClassInfo {
        class_id: 747330370,
        name: "RayTracingShaderImporter",
    },
    UnityClassInfo {
        class_id: 815301076,
        name: "PreviewImporter",
    },
    UnityClassInfo {
        class_id: 825902497,
        name: "RayTracingShader",
    },
    UnityClassInfo {
        class_id: 850595691,
        name: "LightingSettings",
    },
    UnityClassInfo {
        class_id: 877146078,
        name: "PlatformModuleSetup",
    },
    UnityClassInfo {
        class_id: 890905787,
        name: "VersionControlSettings",
    },
    UnityClassInfo {
        class_id: 893571522,
        name: "CustomCollider2D",
    },
    UnityClassInfo {
        class_id: 895512359,
        name: "AimConstraint",
    },
    UnityClassInfo {
        class_id: 937362698,
        name: "VFXManager",
    },
    UnityClassInfo {
        class_id: 947337230,
        name: "RoslynAnalyzerConfigAsset",
    },
    UnityClassInfo {
        class_id: 954905827,
        name: "RuleSetFileAsset",
    },
    UnityClassInfo {
        class_id: 994735392,
        name: "VisualEffectSubgraph",
    },
    UnityClassInfo {
        class_id: 994735403,
        name: "VisualEffectSubgraphOperator",
    },
    UnityClassInfo {
        class_id: 994735404,
        name: "VisualEffectSubgraphBlock",
    },
    UnityClassInfo {
        class_id: 1001480554,
        name: "Prefab",
    },
    UnityClassInfo {
        class_id: 1027052791,
        name: "LocalizationImporter",
    },
    UnityClassInfo {
        class_id: 1114811875,
        name: "ReferencesArtifactGenerator",
    },
    UnityClassInfo {
        class_id: 1152215463,
        name: "AssemblyDefinitionAsset",
    },
    UnityClassInfo {
        class_id: 1154873562,
        name: "SceneVisibilityState",
    },
    UnityClassInfo {
        class_id: 1183024399,
        name: "LookAtConstraint",
    },
    UnityClassInfo {
        class_id: 1210832254,
        name: "SpriteAtlasImporter",
    },
    UnityClassInfo {
        class_id: 1223240404,
        name: "MultiArtifactTestImporter",
    },
    UnityClassInfo {
        class_id: 1268269756,
        name: "GameObjectRecorder",
    },
    UnityClassInfo {
        class_id: 1325145578,
        name: "LightingDataAssetParent",
    },
    UnityClassInfo {
        class_id: 1386491679,
        name: "PresetManager",
    },
    UnityClassInfo {
        class_id: 1403656975,
        name: "StreamingManager",
    },
    UnityClassInfo {
        class_id: 1480428607,
        name: "LowerResBlitTexture",
    },
    UnityClassInfo {
        class_id: 1521398425,
        name: "VideoBuildInfo",
    },
    UnityClassInfo {
        class_id: 1541671625,
        name: "C4DImporter",
    },
    UnityClassInfo {
        class_id: 1542919678,
        name: "StreamingController",
    },
    UnityClassInfo {
        class_id: 1557264870,
        name: "ShaderContainer",
    },
    UnityClassInfo {
        class_id: 1597193336,
        name: "RoslynAdditionalFileAsset",
    },
    UnityClassInfo {
        class_id: 1642787288,
        name: "RoslynAdditionalFileImporter",
    },
    UnityClassInfo {
        class_id: 1660057539,
        name: "SceneRoots",
    },
    UnityClassInfo {
        class_id: 1731078267,
        name: "BrokenPrefabAsset",
    },
    UnityClassInfo {
        class_id: 1736697216,
        name: "AndroidAssetPackImporter",
    },
    UnityClassInfo {
        class_id: 1742807556,
        name: "GridLayout",
    },
    UnityClassInfo {
        class_id: 1766753193,
        name: "AssemblyDefinitionImporter",
    },
    UnityClassInfo {
        class_id: 1773428102,
        name: "ParentConstraint",
    },
    UnityClassInfo {
        class_id: 1777034230,
        name: "RuleSetFileImporter",
    },
    UnityClassInfo {
        class_id: 1818360608,
        name: "PositionConstraint",
    },
    UnityClassInfo {
        class_id: 1818360609,
        name: "RotationConstraint",
    },
    UnityClassInfo {
        class_id: 1818360610,
        name: "ScaleConstraint",
    },
    UnityClassInfo {
        class_id: 1839735485,
        name: "Tilemap",
    },
    UnityClassInfo {
        class_id: 1896753125,
        name: "PackageManifest",
    },
    UnityClassInfo {
        class_id: 1896753126,
        name: "PackageManifestImporter",
    },
    UnityClassInfo {
        class_id: 1903396204,
        name: "RoslynAnalyzerConfigImporter",
    },
    UnityClassInfo {
        class_id: 1953259897,
        name: "TerrainLayer",
    },
    UnityClassInfo {
        class_id: 1971053207,
        name: "SpriteShapeRenderer",
    },
    UnityClassInfo {
        class_id: 2058629509,
        name: "VisualEffectAsset",
    },
    UnityClassInfo {
        class_id: 2058629510,
        name: "VisualEffectImporter",
    },
    UnityClassInfo {
        class_id: 2058629511,
        name: "VisualEffectResource",
    },
    UnityClassInfo {
        class_id: 2059678085,
        name: "VisualEffectObject",
    },
    UnityClassInfo {
        class_id: 2083052967,
        name: "VisualEffect",
    },
    UnityClassInfo {
        class_id: 2083778819,
        name: "LocalizationAsset",
    },
    UnityClassInfo {
        class_id: 2089858483,
        name: "ScriptedImporter",
    },
    UnityClassInfo {
        class_id: 2103361453,
        name: "ShaderIncludeImporter",
    },
];

pub(crate) fn unity_class_name(class_id: i32) -> &'static str {
    UNITY_2022_3_CLASSES
        .iter()
        .find(|entry| entry.class_id == class_id)
        .map(|entry| entry.name)
        .unwrap_or("Object")
}

pub(crate) fn is_model_importer_legacy_component_class_id(class_id: i32) -> bool {
    matches!(class_id, 1 | 4 | 23 | 33 | 64 | 65 | 95 | 137)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_class_name_covers_2022_3_common_and_high_ids() {
        assert_eq!(unity_class_name(4), "Transform");
        assert_eq!(unity_class_name(82), "AudioSource");
        assert_eq!(unity_class_name(328), "VideoPlayer");
        assert_eq!(unity_class_name(73398921), "VFXRenderer");
        assert_eq!(unity_class_name(2083052967), "VisualEffect");
        assert_eq!(unity_class_name(2103361453), "ShaderIncludeImporter");
    }
}
