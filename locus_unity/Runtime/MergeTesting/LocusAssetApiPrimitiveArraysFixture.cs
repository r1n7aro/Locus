using UnityEngine;

namespace Locus.AssetTesting
{
    public enum PrimitiveArrayMode : int { Negative = -2, Zero = 0, Positive = 7 }

    public sealed class LocusAssetApiPrimitiveArraysFixture : ScriptableObject
    {
        public int amount = 101;
        public byte[] bytes = { 1, 2, 250 };
        public sbyte[] signedBytes = { -100, 0, 100 };
        public short[] shorts = { -30000, 0, 30000 };
        public ushort[] unsignedShorts = { 0, 1000, 65000 };
        public int[] integers = { -12, 0, 512 };
        public uint[] unsignedIntegers = { 1U, 2147483648U, 4000000000U };
        public long[] longs = { -9007199254740993L, 1L, 9007199254740993L };
        public ulong[] unsignedLongs = { 1UL, 9007199254740993UL, 18446744073709551615UL };
        public float[] floats = { 0.125f, -2.5f, 7.75f };
        public double[] doubles = { 0.125, -2.5, 7.75 };
        public bool[] booleans = { true, false, true };
        public char[] characters = { 'A', '中', 'z' };
        public PrimitiveArrayMode[] modes = { PrimitiveArrayMode.Negative, PrimitiveArrayMode.Zero, PrimitiveArrayMode.Positive };
    }
}
