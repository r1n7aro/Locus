using System;
using UnityEngine;

namespace Locus.AssetTesting
{
    public enum FixtureMode { First = 0, Second = 3, Third = 7 }
    [Serializable] public struct FixtureRow { public string name; public int amount; public Vector3 position; }
    [Serializable] public class FixtureNode { public int health = 10; public string label = "shared"; [SerializeReference] public FixtureNode next; }
    public sealed class LocusAssetApiFixture : ScriptableObject
    {
        public int amount = 10;
        public bool enabled = true;
        public float speed = 2.5f;
        public string note = "initial 中文";
        public FixtureMode mode = FixtureMode.Second;
        public Vector3 vector = new Vector3(1, 2, 3);
        public Color tint = new Color(1, 0.5f, 0.25f, 1);
        public long large = 9007199254740993L;
        public int[] numbers = { 1, 2, 3 };
        public FixtureRow[] rows = { new FixtureRow { name = "row", amount = 1, position = Vector3.one } };
        public UnityEngine.Object reference;
        [SerializeReference] public FixtureNode root;
        [SerializeReference] public FixtureNode alias;
    }
}
