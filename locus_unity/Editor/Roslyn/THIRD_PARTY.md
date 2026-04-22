# Roslyn Third-Party Components

`locus_unity/Editor/Roslyn` contains precompiled .NET assemblies used only inside the Unity Editor. When this directory is redistributed as part of the project or a Unity package, keep this file together with the original texts under `licenses/` to satisfy the currently verified license and notice requirements.

## File Mapping

| Component | Covered Files | Local Version | Upstream Package | Upstream Project | License | Original Texts |
| --- | --- | --- | --- | --- | --- | --- |
| Roslyn Core | `Microsoft.CodeAnalysis.dll`, `Microsoft.CodeAnalysis.resources.dll` | Assembly `3.8.0.0`; file `3.800.20.56202` | `Microsoft.CodeAnalysis.Common` `3.8.0` | `dotnet/roslyn` | `MIT` | `licenses/roslyn-3.8.0/License.txt`, `licenses/roslyn-3.8.0/ThirdPartyNotices.rtf` |
| Roslyn C# | `Microsoft.CodeAnalysis.CSharp.dll`, `Microsoft.CodeAnalysis.CSharp.resources.dll` | Assembly `3.8.0.0`; file `3.800.20.56202` | `Microsoft.CodeAnalysis.CSharp` `3.8.0` | `dotnet/roslyn` | `MIT` | `licenses/roslyn-3.8.0/License.txt`, `licenses/roslyn-3.8.0/ThirdPartyNotices.rtf` |
| Roslyn Scripting | `Microsoft.CodeAnalysis.Scripting.dll`, `Microsoft.CodeAnalysis.Scripting.resources.dll` | Assembly `3.8.0.0`; file `3.800.20.56202` | `Microsoft.CodeAnalysis.Scripting` `3.8.0` | `dotnet/roslyn` | `MIT` | `licenses/roslyn-3.8.0/License.txt`, `licenses/roslyn-3.8.0/ThirdPartyNotices.rtf` |
| Roslyn C# Scripting | `Microsoft.CodeAnalysis.CSharp.Scripting.dll`, `Microsoft.CodeAnalysis.CSharp.Scripting.resources.dll` | Assembly `3.8.0.0`; file `3.800.20.56202` | `Microsoft.CodeAnalysis.CSharp.Scripting` `3.8.0` | `dotnet/roslyn` | `MIT` | `licenses/roslyn-3.8.0/License.txt`, `licenses/roslyn-3.8.0/ThirdPartyNotices.rtf` |
| Immutable Collections | `System.Collections.Immutable.dll` | Assembly `8.0.0.0`; file `8.0.23.53103` | `System.Collections.Immutable` `8.0.0` | `dotnet/runtime` | `MIT` | `licenses/system-collections-immutable-8.0.0/LICENSE.TXT`, `licenses/system-collections-immutable-8.0.0/THIRD-PARTY-NOTICES.TXT` |
| Metadata Reader | `System.Reflection.Metadata.dll` | Assembly `1.4.3.0`; file `4.6.26515.06` | `System.Reflection.Metadata` `1.6.0` | `dotnet/corefx` | `MIT` | `licenses/system-reflection-metadata-1.6.0/LICENSE.TXT`, `licenses/system-reflection-metadata-1.6.0/THIRD-PARTY-NOTICES.TXT` |
| Unsafe Helpers | `System.Runtime.CompilerServices.Unsafe.dll` | Assembly `4.0.4.1`; file `4.6.28619.01` | `System.Runtime.CompilerServices.Unsafe` `4.5.3` | `dotnet/corefx` | `MIT` | `licenses/system-runtime-compilerservices-unsafe-4.5.3/LICENSE.TXT`, `licenses/system-runtime-compilerservices-unsafe-4.5.3/THIRD-PARTY-NOTICES.TXT` |

## Version Verification

- This document follows the assembly versions and file versions of the DLLs actually shipped in this directory. It does not infer the `System.*` component versions from the minimum dependency versions declared by Roslyn `3.8.0`.
- The Roslyn DLL group reports `ProductVersion` `3.8.0-5.20562.2+8de9e4b2beba5b7c0edd6f1e6a4f192a51fdc872`, which matches the official `3.8.0` packages built from the same commit.
- `System.Collections.Immutable.dll` matches the assembly version and file version from the official `System.Collections.Immutable 8.0.0` package.
- `System.Reflection.Metadata.dll` matches the assembly version and file version from the official `System.Reflection.Metadata 1.6.0` package.
- `System.Runtime.CompilerServices.Unsafe.dll` matches the assembly version and file version from the official `System.Runtime.CompilerServices.Unsafe 4.5.3` package.

## Redistribution Requirements

1. When redistributing `locus_unity/Editor/Roslyn`, keep this file and the entire `licenses/` directory.
2. Provide the applicable copyright notices, the MIT license text, and the associated third-party notices for each component.
3. `*.resources.dll` files are localization satellite assemblies of their host assemblies and follow the same license and notice set as the host package.
4. When any DLL is updated, update this table and the version-matched original texts at the same time. Do not mix original texts from different upstream versions.
