using System.Runtime.CompilerServices;

namespace Locus.HotReload
{
    /// <summary>
    /// Instance-field virtualization for hot patches (M4): a field ADDED by
    /// a hot edit cannot live in the original type's layout, so every access
    /// is rewritten to a per-field store keyed by the instance. One static
    /// LocusFieldStore per added field, declared by the FIRST patch that
    /// introduces the field and reused by later patches (FieldStoreRegistry
    /// binds them), which is why this type must come from a single shared
    /// assembly rather than being emitted into each patch.
    ///
    /// `Ref` returns a reference INTO the per-instance box, so reads,
    /// writes, compound assignments and `ref` arguments all work through
    /// one uniform rewrite. Instances the store has never seen yield
    /// default(TValue) — exactly the semantics of "this object predates the
    /// field". Values do not survive a domain reload; the convergence
    /// recompile turns the field real and normal initialization takes over.
    /// </summary>
    public sealed class LocusFieldStore<TValue>
    {
        private sealed class Box
        {
            public TValue Value;
        }

        private readonly ConditionalWeakTable<object, Box> _table =
            new ConditionalWeakTable<object, Box>();

        public ref TValue Ref(object target)
        {
            Box box;
            if (!_table.TryGetValue(target, out box))
            {
                box = _table.GetValue(target, _ => new Box());
            }
            return ref box.Value;
        }
    }
}
