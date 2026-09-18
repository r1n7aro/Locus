using System;
using System.Collections.Generic;
using UnityEngine;

namespace PropertyReview
{
    [Serializable] public class ReviewNode
    {
        public int amount = 7;
        [HideInInspector] public int hidden = 11;
        [SerializeReference] public ReviewNode next;
    }
    [Serializable] public class ReviewLeaf : ReviewNode { public string label = "leaf"; }
    [Serializable] public class ReviewOther : ReviewNode { public bool enabled = true; }

    public sealed class ReviewData : ScriptableObject
    {
        public long wide = 5000000000L;
        public ulong unsignedWide = ulong.MaxValue;
        public int regularInt = 17;
        public string note = "before";
        public bool toggle = true;
        public double precise = 1.23456789012345;
        public Vector3Int intVector = new Vector3Int(123, 456, 789);
        public Color hdr = new Color(3.123456f, 0.25f, 0.5f, 1f);
        public AnimationCurve curve = AnimationCurve.Linear(0, 1, 1, 3);
        public Gradient gradient = new Gradient();
        public UnityEngine.Object reference;
        public Hash128 hash = new Hash128(1, 2, 3, 4);
        public ExposedReference<GameObject> exposed;
        [HideInInspector] public int hidden = 42;
        public List<int> numbers = new List<int>();
        [SerializeReference] public ReviewNode node;
        [SerializeReference] public ReviewNode alias;
        [SerializeReference] public List<ReviewNode> nodes = new List<ReviewNode>();
    }
}
