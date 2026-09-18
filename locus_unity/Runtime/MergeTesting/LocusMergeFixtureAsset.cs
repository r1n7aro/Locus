using System;
using System.Collections.Generic;
using UnityEngine;

namespace Locus.MergeTesting
{
    // Runtime-assembly serialized types used by the CLI integration driver. Keep their
    // names stable: the corpus deliberately checks assembly/type identities.
    public sealed class LocusMergeFixtureAsset : ScriptableObject
    {
        public int localValue = 10;
        public int incomingValue = 20;
        public string note = "quoted: value\nsecond line\nUnicode 中文";
        public List<MergeRow> rows = new List<MergeRow>();
        [SerializeReference] public MergeNode root;
        [SerializeReference] public MergeNode alias;
        [SerializeReference] public MergeNode optional;
    }

    [Serializable]
    public struct MergeRow
    {
        public string key;
        public int amount;
        public Vector3 position;
    }

    [Serializable]
    public abstract class MergeNode
    {
        public string label;
        [SerializeReference] public MergeNode next;
    }

    [Serializable]
    public sealed class MergeGroup : MergeNode
    {
        [SerializeReference] public List<MergeNode> children = new List<MergeNode>();
    }

    [Serializable]
    public sealed class MergeAction : MergeNode
    {
        public int health;
        public float speed;
    }

    [Serializable]
    public sealed class MergeWeightedAction : MergeNode
    {
        public int health;
        public float weight;
    }
}
