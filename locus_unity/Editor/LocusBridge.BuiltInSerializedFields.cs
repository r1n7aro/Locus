using UnityEditor;

using System;
using System.Collections.Generic;

namespace Locus
{
    public static partial class LocusBridge
    {
        private sealed class BuiltInSerializedObjectReferenceField
        {
            public string ownerTypeFullName;
            public string propertyPath;
            public string[] referenceTypeFullNames;
        }

        private static readonly BuiltInSerializedObjectReferenceField[] BuiltInSerializedObjectReferenceFields =
        {
            Field("UnityEngine.Transform", "m_Father", "UnityEngine.Transform"),
            Field("UnityEngine.RectTransform", "m_Father", "UnityEngine.Transform"),

            Field("UnityEngine.Renderer", "m_Materials[]", "UnityEngine.Material"),
            Field("UnityEngine.Renderer", "m_LightProbeVolumeOverride", "UnityEngine.GameObject"),
            Field("UnityEngine.Renderer", "m_ProbeAnchor", "UnityEngine.Transform"),
            Field("UnityEngine.Renderer", "m_LightmapParameters", "UnityEngine.LightmapParameters"),
            Field("UnityEngine.MeshRenderer", "m_AdditionalVertexStreams", "UnityEngine.Mesh"),
            Field("UnityEngine.MeshRenderer", "m_EnlightenVertexStream", "UnityEngine.Mesh"),
            Field("UnityEngine.SkinnedMeshRenderer", "m_Mesh", "UnityEngine.Mesh"),
            Field("UnityEngine.SkinnedMeshRenderer", "m_RootBone", "UnityEngine.Transform"),
            Field("UnityEngine.SkinnedMeshRenderer", "m_Bones[]", "UnityEngine.Transform"),
            Field("UnityEngine.MeshFilter", "m_Mesh", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystemRenderer", "m_Mesh", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystemRenderer", "m_Mesh1", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystemRenderer", "m_Mesh2", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystemRenderer", "m_Mesh3", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystemRenderer", "m_Meshes[]", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystemRenderer", "m_TrailMaterial", "UnityEngine.Material"),
            Field("UnityEngine.BillboardRenderer", "m_Billboard", "UnityEngine.BillboardAsset"),
            Field("UnityEngine.SpriteRenderer", "m_Sprite", "UnityEngine.Sprite"),
            Field("UnityEngine.SpriteMask", "m_Sprite", "UnityEngine.Sprite"),
            Field("UnityEngine.LODGroup", "m_LODs[].renderers[]", "UnityEngine.Renderer"),

            Field("UnityEngine.BoxCollider", "m_Material", "UnityEngine.PhysicMaterial", "UnityEngine.PhysicsMaterial"),
            Field("UnityEngine.SphereCollider", "m_Material", "UnityEngine.PhysicMaterial", "UnityEngine.PhysicsMaterial"),
            Field("UnityEngine.CapsuleCollider", "m_Material", "UnityEngine.PhysicMaterial", "UnityEngine.PhysicsMaterial"),
            Field("UnityEngine.Collider", "m_Material", "UnityEngine.PhysicMaterial", "UnityEngine.PhysicsMaterial"),
            Field("UnityEngine.MeshCollider", "m_Mesh", "UnityEngine.Mesh"),
            Field("UnityEngine.TerrainCollider", "m_TerrainData", "UnityEngine.TerrainData"),
            Field("UnityEngine.Joint", "m_ConnectedBody", "UnityEngine.Rigidbody"),
            Field("UnityEngine.Joint", "m_ConnectedArticulationBody", "UnityEngine.ArticulationBody"),
            Field("UnityEngine.Cloth", "m_CapsuleColliders[].first", "UnityEngine.CapsuleCollider"),
            Field("UnityEngine.Cloth", "m_CapsuleColliders[].second", "UnityEngine.CapsuleCollider"),
            Field("UnityEngine.Cloth", "m_SphereColliders[]", "UnityEngine.SphereCollider"),

            Field("UnityEngine.Collider2D", "m_Material", "UnityEngine.PhysicsMaterial2D"),
            Field("UnityEngine.Rigidbody2D", "m_Material", "UnityEngine.PhysicsMaterial2D"),
            Field("UnityEngine.Joint2D", "m_ConnectedRigidBody", "UnityEngine.Rigidbody2D"),

            Field("UnityEngine.Camera", "m_TargetTexture", "UnityEngine.RenderTexture"),
            Field("UnityEngine.Light", "m_Flare", "UnityEngine.Flare"),
            Field("UnityEngine.Light", "m_Cookie", "UnityEngine.Texture"),
            Field("UnityEngine.LensFlare", "m_Flare", "UnityEngine.Flare"),
            Field("UnityEngine.Projector", "m_Material", "UnityEngine.Material"),
            Field("UnityEngine.Skybox", "m_Material", "UnityEngine.Material"),
            Field("UnityEngine.ReflectionProbe", "m_CustomBakedTexture", "UnityEngine.Cubemap"),
            Field("UnityEngine.ReflectionProbe", "m_BakedTexture", "UnityEngine.Cubemap"),
            Field("UnityEngine.CustomRenderTexture", "m_Material", "UnityEngine.Material"),
            Field("UnityEngine.CustomRenderTexture", "m_InitTexture", "UnityEngine.Texture"),
            Field("UnityEngine.CustomRenderTexture", "m_InitMaterial", "UnityEngine.Material"),
            Field("UnityEngine.Terrain", "m_TerrainData", "UnityEngine.TerrainData"),
            Field("UnityEngine.Terrain", "m_MaterialTemplate", "UnityEngine.Material"),
            Field("UnityEngine.Terrain", "m_LightmapParameters", "UnityEngine.LightmapParameters"),

            Field("UnityEngine.AudioSource", "m_audioClip", "UnityEngine.AudioClip"),
            Field("UnityEngine.AudioSource", "m_AudioClip", "UnityEngine.AudioClip"),
            Field("UnityEngine.AudioSource", "m_OutputAudioMixerGroup", "UnityEngine.Audio.AudioMixerGroup"),
            Field("UnityEngine.Video.VideoPlayer", "m_VideoClip", "UnityEngine.Video.VideoClip"),
            Field("UnityEngine.Video.VideoPlayer", "m_TargetTexture", "UnityEngine.RenderTexture"),
            Field("UnityEngine.Video.VideoPlayer", "m_TargetCamera", "UnityEngine.Camera"),
            Field("UnityEngine.Video.VideoPlayer", "m_TargetMaterialRenderer", "UnityEngine.Renderer"),
            Field("UnityEngine.Video.VideoPlayer", "m_TargetAudioSources[]", "UnityEngine.AudioSource"),

            Field("UnityEngine.Animator", "m_Avatar", "UnityEngine.Avatar"),
            Field("UnityEngine.Animator", "m_Controller", "UnityEngine.RuntimeAnimatorController"),
            Field("UnityEngine.Animation", "m_Animation", "UnityEngine.AnimationClip"),
            Field("UnityEngine.Animation", "m_Animations[]", "UnityEngine.AnimationClip"),
            Field("UnityEngine.Playables.PlayableDirector", "m_PlayableAsset", "UnityEngine.Playables.PlayableAsset"),
            Field("UnityEngine.Animations.ParentConstraint", "m_Sources[].sourceTransform", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.PositionConstraint", "m_Sources[].sourceTransform", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.RotationConstraint", "m_Sources[].sourceTransform", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.ScaleConstraint", "m_Sources[].sourceTransform", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.AimConstraint", "m_Sources[].sourceTransform", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.AimConstraint", "m_WorldUpObject", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.LookAtConstraint", "m_Sources[].sourceTransform", "UnityEngine.Transform"),
            Field("UnityEngine.Animations.LookAtConstraint", "m_WorldUpObject", "UnityEngine.Transform"),

            Field("UnityEngine.ParticleSystem", "moveWithCustomTransform", "UnityEngine.Transform"),
            Field("UnityEngine.ParticleSystem", "ShapeModule.m_Mesh", "UnityEngine.Mesh"),
            Field("UnityEngine.ParticleSystem", "ShapeModule.m_MeshRenderer", "UnityEngine.MeshRenderer"),
            Field("UnityEngine.ParticleSystem", "ShapeModule.m_SkinnedMeshRenderer", "UnityEngine.SkinnedMeshRenderer"),
            Field("UnityEngine.ParticleSystem", "ShapeModule.m_Sprite", "UnityEngine.Sprite"),
            Field("UnityEngine.ParticleSystem", "ShapeModule.m_SpriteRenderer", "UnityEngine.SpriteRenderer"),
            Field("UnityEngine.ParticleSystem", "ShapeModule.m_Texture", "UnityEngine.Texture2D", "UnityEngine.Texture"),
            Field("UnityEngine.ParticleSystem", "CollisionModule.m_Planes[]", "UnityEngine.Transform"),
            Field("UnityEngine.ParticleSystem", "TriggerModule.primitives[]", "UnityEngine.Component"),
            Field("UnityEngine.ParticleSystem", "SubModule.subEmitters[].emitter", "UnityEngine.ParticleSystem"),
            Field("UnityEngine.ParticleSystem", "UVModule.sprites[].sprite", "UnityEngine.Sprite"),
            Field("UnityEngine.ParticleSystem", "ExternalForcesModule.influenceList[]", "UnityEngine.ParticleSystemForceField"),
            Field("UnityEngine.ParticleSystem", "LightsModule.light", "UnityEngine.Light"),

            Field("UnityEngine.TextMesh", "m_Font", "UnityEngine.Font"),
            Field("UnityEngine.TextMesh", "m_Material", "UnityEngine.Material"),
            Field("UnityEngine.Canvas", "m_Camera", "UnityEngine.Camera"),
            Field("UnityEngine.CanvasRenderer", "m_Materials[]", "UnityEngine.Material"),
            Field("UnityEngine.CanvasRenderer", "m_Texture", "UnityEngine.Texture"),
            Field("UnityEngine.AI.OffMeshLink", "m_Start", "UnityEngine.Transform"),
            Field("UnityEngine.AI.OffMeshLink", "m_End", "UnityEngine.Transform"),
            Field("UnityEngine.VFX.VisualEffect", "m_Asset", "UnityEngine.VFX.VisualEffectAsset"),

            Field("UnityEngine.UIElements.UIDocument", "m_PanelSettings", "UnityEngine.UIElements.PanelSettings"),
            Field("UnityEngine.UIElements.UIDocument", "m_ParentUI", "UnityEngine.UIElements.UIDocument"),
            Field("UnityEngine.UIElements.UIDocument", "sourceAsset", "UnityEngine.UIElements.VisualTreeAsset"),
            Field("UnityEngine.UIElements.UIDocument", "m_WorldSpaceCollider", "UnityEngine.BoxCollider"),
            Field("UnityEngine.UIElements.PanelInputConfiguration", "m_Settings.m_EventCameras[]", "UnityEngine.Camera"),
            Field("UnityEngine.UIElements.PanelSettings", "themeUss", "UnityEngine.UIElements.ThemeStyleSheet", "UnityEngine.UIElements.StyleSheet"),
            Field("UnityEngine.UIElements.PanelSettings", "m_TargetTexture", "UnityEngine.RenderTexture"),
            Field("UnityEngine.UIElements.PanelSettings", "m_AtlasBlitShader", "UnityEngine.Shader"),
            Field("UnityEngine.UIElements.PanelSettings", "m_RuntimeShader", "UnityEngine.Shader"),
            Field("UnityEngine.UIElements.PanelSettings", "m_RuntimeWorldShader", "UnityEngine.Shader"),
            Field("UnityEngine.UIElements.PanelSettings", "m_SDFShader", "UnityEngine.Shader"),
            Field("UnityEngine.UIElements.PanelSettings", "m_BitmapShader", "UnityEngine.Shader"),
            Field("UnityEngine.UIElements.PanelSettings", "m_SpriteShader", "UnityEngine.Shader"),
            Field("UnityEngine.UIElements.PanelSettings", "m_ICUDataAsset", "UnityEngine.TextAsset"),
            Field("UnityEngine.UIElements.PanelSettings", "textSettings", "UnityEngine.UIElements.PanelTextSettings"),

            Field("UnityEngine.TextCore.Text.TextAsset", "m_Material", "UnityEngine.Material"),
            Field("UnityEngine.TextCore.Text.FontAsset", "m_SourceFontFile_EditorRef", "UnityEngine.Font"),
            Field("UnityEngine.TextCore.Text.FontAsset", "m_SourceFontFile", "UnityEngine.Font"),
            Field("UnityEngine.TextCore.Text.FontAsset", "m_AtlasTextures[]", "UnityEngine.Texture2D"),
            Field("UnityEngine.TextCore.Text.FontAsset", "m_FallbackFontAssetTable[]", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.TextCore.Text.FontAsset", "m_FontWeightTable[].regularTypeface", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.TextCore.Text.FontAsset", "m_FontWeightTable[].italicTypeface", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.TextCore.Text.SpriteAsset", "m_SpriteAtlasTexture", "UnityEngine.Texture"),
            Field("UnityEngine.TextCore.Text.SpriteAsset", "fallbackSpriteAssets[]", "UnityEngine.TextCore.Text.SpriteAsset"),
            Field("UnityEngine.TextCore.Text.TextSettings", "m_DefaultFontAsset", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.TextCore.Text.TextSettings", "m_FallbackFontAssets[]", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.TextCore.Text.TextSettings", "m_DefaultSpriteAsset", "UnityEngine.TextCore.Text.SpriteAsset"),
            Field("UnityEngine.TextCore.Text.TextSettings", "m_FallbackSpriteAssets[]", "UnityEngine.TextCore.Text.SpriteAsset"),
            Field("UnityEngine.UIElements.PanelTextSettings", "m_DefaultFontAsset", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.UIElements.PanelTextSettings", "m_FallbackFontAssets[]", "UnityEngine.TextCore.Text.FontAsset"),
            Field("UnityEngine.UIElements.PanelTextSettings", "m_DefaultSpriteAsset", "UnityEngine.TextCore.Text.SpriteAsset"),
            Field("UnityEngine.UIElements.PanelTextSettings", "m_FallbackSpriteAssets[]", "UnityEngine.TextCore.Text.SpriteAsset"),

            Field("UnityEngine.UI.Graphic", "m_Material", "UnityEngine.Material"),
            Field("UnityEngine.UI.Image", "m_Sprite", "UnityEngine.Sprite"),
            Field("UnityEngine.UI.Image", "m_OverrideSprite", "UnityEngine.Sprite"),
            Field("UnityEngine.UI.RawImage", "m_Texture", "UnityEngine.Texture"),
            Field("UnityEngine.UI.Text", "m_FontData.m_Font", "UnityEngine.Font"),
            Field("UnityEngine.UI.Selectable", "m_TargetGraphic", "UnityEngine.UI.Graphic"),
            Field("UnityEngine.UI.Selectable", "m_Navigation.m_SelectOnUp", "UnityEngine.UI.Selectable"),
            Field("UnityEngine.UI.Selectable", "m_Navigation.m_SelectOnDown", "UnityEngine.UI.Selectable"),
            Field("UnityEngine.UI.Selectable", "m_Navigation.m_SelectOnLeft", "UnityEngine.UI.Selectable"),
            Field("UnityEngine.UI.Selectable", "m_Navigation.m_SelectOnRight", "UnityEngine.UI.Selectable"),
            Field("UnityEngine.UI.ScrollRect", "m_Content", "UnityEngine.RectTransform"),
            Field("UnityEngine.UI.ScrollRect", "m_Viewport", "UnityEngine.RectTransform"),
            Field("UnityEngine.UI.ScrollRect", "m_HorizontalScrollbar", "UnityEngine.UI.Scrollbar"),
            Field("UnityEngine.UI.ScrollRect", "m_VerticalScrollbar", "UnityEngine.UI.Scrollbar"),
            Field("UnityEngine.UI.Scrollbar", "m_HandleRect", "UnityEngine.RectTransform"),
            Field("UnityEngine.UI.Slider", "m_FillRect", "UnityEngine.RectTransform"),
            Field("UnityEngine.UI.Slider", "m_HandleRect", "UnityEngine.RectTransform"),
            Field("UnityEngine.UI.Dropdown", "m_Template", "UnityEngine.RectTransform"),
            Field("UnityEngine.UI.Dropdown", "m_CaptionText", "UnityEngine.UI.Text"),
            Field("UnityEngine.UI.Dropdown", "m_CaptionImage", "UnityEngine.UI.Image"),
            Field("UnityEngine.UI.Dropdown", "m_ItemText", "UnityEngine.UI.Text"),
            Field("UnityEngine.UI.Dropdown", "m_ItemImage", "UnityEngine.UI.Image"),
            Field("UnityEngine.UI.InputField", "m_TextComponent", "UnityEngine.UI.Text"),
            Field("UnityEngine.UI.InputField", "m_Placeholder", "UnityEngine.UI.Graphic"),

            Field("UnityEngine.Tilemaps.Tilemap", "m_Tiles[].m_TileAsset", "UnityEngine.Tilemaps.TileBase"),
            Field("UnityEngine.Tilemaps.Tile", "m_Sprite", "UnityEngine.Sprite"),
            Field("UnityEngine.Tilemaps.Tile", "m_InstancedGameObject", "UnityEngine.GameObject"),
            Field("UnityEngine.TerrainData", "m_TerrainLayers[]", "UnityEngine.TerrainLayer"),
            Field("UnityEngine.TerrainData", "m_TreeDatabase.m_TreePrototypes[].m_Prefab", "UnityEngine.GameObject"),
            Field("UnityEngine.TerrainData", "m_DetailDatabase.m_DetailPrototypes[].m_Prototype", "UnityEngine.GameObject"),
            Field("UnityEngine.TerrainData", "m_DetailDatabase.m_DetailPrototypes[].m_PrototypeTexture", "UnityEngine.Texture2D"),
            Field("UnityEngine.TerrainLayer", "m_DiffuseTexture", "UnityEngine.Texture2D"),
            Field("UnityEngine.TerrainLayer", "m_NormalMapTexture", "UnityEngine.Texture2D"),
            Field("UnityEngine.TerrainLayer", "m_MaskMapTexture", "UnityEngine.Texture2D"),
            Field("UnityEngine.LightmapData", "m_Light", "UnityEngine.Texture2D"),
            Field("UnityEngine.LightmapData", "m_Dir", "UnityEngine.Texture2D"),
            Field("UnityEngine.LightmapData", "m_ShadowMask", "UnityEngine.Texture2D"),
        };

        private static readonly Dictionary<string, Type> BuiltInSerializedFieldTypeCache =
            new Dictionary<string, Type>();

        private static BuiltInSerializedObjectReferenceField Field(
            string ownerTypeFullName,
            string propertyPath,
            params string[] referenceTypeFullNames)
        {
            return new BuiltInSerializedObjectReferenceField
            {
                ownerTypeFullName = ownerTypeFullName,
                propertyPath = propertyPath,
                referenceTypeFullNames = referenceTypeFullNames ?? new string[0]
            };
        }

        private static Type ResolveBuiltInSerializedPropertyFieldType(SerializedProperty prop)
        {
            if (prop == null || prop.serializedObject == null || prop.serializedObject.targetObject == null)
                return null;

            Type ownerType = prop.serializedObject.targetObject.GetType();
            string propertyPath = NormalizeBuiltInSerializedPropertyPath(prop.propertyPath);
            if (ownerType == null || string.IsNullOrEmpty(propertyPath))
                return null;

            for (int i = 0; i < BuiltInSerializedObjectReferenceFields.Length; i++)
            {
                BuiltInSerializedObjectReferenceField field = BuiltInSerializedObjectReferenceFields[i];
                if (!BuiltInSerializedOwnerMatches(ownerType, field.ownerTypeFullName))
                    continue;
                if (!string.Equals(propertyPath, field.propertyPath, StringComparison.Ordinal))
                    continue;

                return ResolveBuiltInSerializedReferenceType(field.referenceTypeFullNames);
            }

            return null;
        }

        private static string NormalizeBuiltInSerializedPropertyPath(string propertyPath)
        {
            string path = propertyPath ?? "";
            while (true)
            {
                int start = path.IndexOf(".Array.data[", StringComparison.Ordinal);
                if (start < 0)
                    return path;

                int end = path.IndexOf(']', start);
                if (end < 0)
                    return path;

                path = path.Substring(0, start) + "[]" + path.Substring(end + 1);
            }
        }

        private static bool BuiltInSerializedOwnerMatches(Type ownerType, string ownerTypeFullName)
        {
            for (Type current = ownerType; current != null; current = current.BaseType)
            {
                if (string.Equals(current.FullName, ownerTypeFullName, StringComparison.Ordinal))
                    return true;
            }
            return false;
        }

        private static Type ResolveBuiltInSerializedReferenceType(string[] referenceTypeFullNames)
        {
            if (referenceTypeFullNames == null)
                return null;

            for (int i = 0; i < referenceTypeFullNames.Length; i++)
            {
                Type type = ResolveLoadedType(referenceTypeFullNames[i]);
                if (type != null)
                    return type;
            }

            return null;
        }

        private static Type ResolveLoadedType(string fullName)
        {
            fullName = (fullName ?? "").Trim();
            if (string.IsNullOrEmpty(fullName))
                return null;

            Type cached;
            if (BuiltInSerializedFieldTypeCache.TryGetValue(fullName, out cached))
                return cached;

            Type resolved = null;
            foreach (System.Reflection.Assembly assembly in AppDomain.CurrentDomain.GetAssemblies())
            {
                try
                {
                    resolved = assembly.GetType(fullName, false);
                    if (resolved != null)
                        break;
                }
                catch
                {
                }
            }

            BuiltInSerializedFieldTypeCache[fullName] = resolved;
            return resolved;
        }
    }
}
