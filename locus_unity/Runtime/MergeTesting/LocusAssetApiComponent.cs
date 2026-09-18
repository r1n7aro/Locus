using UnityEngine;
namespace Locus.AssetTesting
{
    public sealed class LocusAssetApiComponent : MonoBehaviour
    {
        public int amount = 10;
        public string note = "component baseline";
        public int[] numbers = { 1, 2, 3 };
        public Vector3 vector = new Vector3(1, 2, 3);
    }
}
