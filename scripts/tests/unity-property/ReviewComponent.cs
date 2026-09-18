using UnityEngine;

namespace PropertyReview
{
    public sealed class ReviewComponent : MonoBehaviour
    {
        public int amount = 10;
        public string label = "base";
        public long wide = 7;
        [SerializeReference] public ReviewNode behavior = new ReviewLeaf();
        public GameObject sceneReference;
    }
}
