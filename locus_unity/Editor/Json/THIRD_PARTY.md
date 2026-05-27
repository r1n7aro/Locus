# JSON Third-Party Components

`locus_unity/Editor/Json` contains `Locus.Json.dll`, a merged .NET assembly used only inside the Unity Editor. The original Newtonsoft.Json DLL is kept in `third_party/newtonsoft-json-13.0.3/assemblies` as a bundle input, and is not distributed from `locus_unity`.

Run `bun run unity:bundle-json` after changing any original DLL input.

## File Mapping

| Component | Covered Files | Local Version | Upstream Package | Upstream Project | License | Original Texts |
| --- | --- | --- | --- | --- | --- | --- |
| Newtonsoft.Json | Merged into `Locus.Json.dll` from `third_party/newtonsoft-json-13.0.3/assemblies/Newtonsoft.Json.dll` | Assembly `13.0.0.0`; file `13.0.3.27908`; product `13.0.3+0a2e291c0d9c0c7675d445703e51750363a549ef` | `Newtonsoft.Json` `13.0.3` | `JamesNK/Newtonsoft.Json` | `MIT` | `licenses/newtonsoft-json-13.0.3/LICENSE.md` |

## Version Verification

- This document follows the assembly version and file version of the DLL actually shipped in this directory.
- `Locus.Json.dll` is generated with ILRepack `2.0.44` from the original input above, with assembly identity `Locus.Json, Version=13.0.3.0`.
- The Newtonsoft.Json input is internalized and renamed in the merged assembly. The public API exposed by this bundle is `Locus.Json.LocusJson`.

## Redistribution Requirements

1. When redistributing `locus_unity/Editor/Json`, keep `Locus.Json.dll`, this file, and the entire `licenses/` directory.
2. Provide the applicable copyright notice and the MIT license text for Newtonsoft.Json.
3. When the DLL is updated, update this table, rebuild `Locus.Json.dll`, and update the version-matched original texts at the same time.
4. Keep the original DLL input under `third_party/newtonsoft-json-13.0.3/assemblies`; do not add the original `Newtonsoft.Json.dll` back to `locus_unity`.
