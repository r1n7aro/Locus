using UnityEngine;

namespace Locus.MergeTesting
{
    public sealed class LocusMergeFixtureComponent : MonoBehaviour
    {
        public LocusMergeFixtureAsset asset;
        public GameObject sibling;
        [SerializeReference] public MergeNode behavior;
    }
}
