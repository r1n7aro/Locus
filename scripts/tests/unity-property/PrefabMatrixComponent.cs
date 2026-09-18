using System;
using System.Collections.Generic;
using UnityEngine;

namespace PropertyPrefabMatrix
{
    public sealed class PrefabMatrixComponent : MonoBehaviour
    {
        [Serializable] public class Item { public int amount; public string label; public bool flag; public float weight; public List<int> values = new List<int>(); }
        [Serializable] public class Node { public int amount = 7; [SerializeReference] public Node next; }
        public int amount = 7;
        public List<int> numbers = new List<int> { 10, 20 };
        public List<Item> items = new List<Item> { new Item { amount = 1, label = "first", values = new List<int> { 1, 2 } } };
        public GameObject link;
        public List<Item> empty = new List<Item>();
        [SerializeReference] public Node node = new Node();
    }
}
