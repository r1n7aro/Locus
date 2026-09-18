using System;
using System.Collections.Generic;
using UnityEngine;

namespace PropertyServiceReview
{
    public sealed class ServiceData : ScriptableObject
    {
        [Serializable] public class Node { public int amount; [SerializeReference] public Node next; }
        [Serializable] public class Item { public bool flag; public float weight; public string label; }
        [Serializable] public class Group { public List<Item> items = new List<Item>(); }
        public int amount = 7;
        public List<Group> groups = new List<Group>();
        [SerializeReference] public Node node;
    }
}
