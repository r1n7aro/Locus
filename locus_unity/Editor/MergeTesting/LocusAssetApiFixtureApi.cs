using System;
using System.Collections.Generic;
using UnityEngine;
using UnityEditor;
using UnityEditor.SceneManagement;
using Locus.AssetTesting;

namespace Locus
{
    public static class LocusAssetApiFixtureApi
    {
        [Serializable] private class Pair { public string yaml; public string yaml_live; public string live; public string kind; }
        [Serializable] private class Report { public string folder; public List<Pair> pairs; public string shared; }
        public static string Create(string folder, int count)
        {
            if (!folder.StartsWith("Assets/LocusAssetApiTests/run-", StringComparison.Ordinal) || AssetDatabase.IsValidFolder(folder))
                throw new Exception("Fixture requires a new owned run folder");
            if (!AssetDatabase.IsValidFolder("Assets/LocusAssetApiTests")) AssetDatabase.CreateFolder("Assets", "LocusAssetApiTests");
            AssetDatabase.CreateFolder("Assets/LocusAssetApiTests", folder.Substring("Assets/LocusAssetApiTests/".Length));
            var shared = ScriptableObject.CreateInstance<LocusAssetApiFixture>();
            AssetDatabase.CreateAsset(shared, folder + "/Shared.asset");
            EditorUtility.SetDirty(shared); AssetDatabase.SaveAssetIfDirty(shared);
            var pairs = new List<Pair>();
            for (int i = 0; i < count; i++)
            {
                string yaml = folder + "/Yaml-" + i + ".asset", live = folder + "/Live-" + i + ".asset";
                var asset = ScriptableObject.CreateInstance<LocusAssetApiFixture>();
                var node = new FixtureNode(); node.next = node;
                asset.root = node; asset.alias = node; asset.reference = shared;
                AssetDatabase.CreateAsset(asset, yaml);
                UnityEngine.Serialization.ManagedReferenceUtility.SetManagedReferenceIdForObject(asset, node, 9007199254740993L);
                EditorUtility.SetDirty(asset); AssetDatabase.SaveAssetIfDirty(asset);
                if (!AssetDatabase.CopyAsset(yaml, live)) throw new Exception("Fixture copy failed");
                string online=folder+"/YamlOnline-"+i+".asset";
                if (!AssetDatabase.CopyAsset(yaml,online)) throw new Exception("Online fixture copy failed");
                pairs.Add(new Pair { yaml = yaml, yaml_live=online, live = live, kind = "asset" });
            }
            var go = new GameObject("Asset API fixture"); go.AddComponent<LocusAssetApiComponent>();
            string yp = folder + "/Yaml.prefab", lp = folder + "/Live.prefab";
            PrefabUtility.SaveAsPrefabAsset(go, yp); UnityEngine.Object.DestroyImmediate(go);
            if (!AssetDatabase.CopyAsset(yp, lp)) throw new Exception("Prefab fixture copy failed");
            string op=folder+"/YamlOnline.prefab"; AssetDatabase.CopyAsset(yp,op);
            pairs.Add(new Pair { yaml = yp, yaml_live=op, live = lp, kind = "prefab" });
            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Additive);
            go = new GameObject("Scene fixture"); go.AddComponent<LocusAssetApiComponent>();
            UnityEngine.SceneManagement.SceneManager.MoveGameObjectToScene(go, scene);
            string ys = folder + "/Yaml.unity", ls = folder + "/Live.unity";
            EditorSceneManager.SaveScene(scene, ys); EditorSceneManager.CloseScene(scene, true);
            if (!AssetDatabase.CopyAsset(ys, ls)) throw new Exception("Scene fixture copy failed");
            string os=folder+"/YamlOnline.unity"; AssetDatabase.CopyAsset(ys,os);
            pairs.Add(new Pair { yaml = ys, yaml_live=os, live = ls, kind = "scene" });
            var shader=Shader.Find("Universal Render Pipeline/Lit") ?? Shader.Find("Standard");
            if(shader==null) throw new Exception("Fixture material requires an installed shader");
            var material=new Material(shader);
            string ym=folder+"/Yaml.mat",lm=folder+"/Live.mat",om=folder+"/YamlOnline.mat";
            AssetDatabase.CreateAsset(material,ym);EditorUtility.SetDirty(material);AssetDatabase.SaveAssetIfDirty(material);
            if(!AssetDatabase.CopyAsset(ym,lm)||!AssetDatabase.CopyAsset(ym,om))throw new Exception("Material copy failed");
            pairs.Add(new Pair {yaml=ym,yaml_live=om,live=lm,kind="material"});
            var numeric=LocusAssetApiPrimitiveArraysFixtureApi.CreatePair(folder);
            pairs.Add(new Pair {yaml=numeric.yaml,yaml_live=numeric.yaml_live,live=numeric.live,kind=numeric.kind});
            return JsonUtility.ToJson(new Report { folder=folder, pairs=pairs, shared=folder+"/Shared.asset" });
        }
    }
}
