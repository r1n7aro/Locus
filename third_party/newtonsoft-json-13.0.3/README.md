# Newtonsoft.Json 13.0.3 Bundle Input

This directory keeps the original `Newtonsoft.Json.dll` used to build
`locus_unity/Editor/Json/Locus.Json.dll`.

The Unity package distributes the merged `Locus.Json.dll`. The original DLL
remains in this repository so the bundle can be rebuilt without retrieving a
different upstream artifact.

Input source:

- NuGet package: `Newtonsoft.Json` `13.0.3`
- DLL path: `lib/netstandard2.0/Newtonsoft.Json.dll`
- Repository commit recorded by the NuGet package:
  `0a2e291c0d9c0c7675d445703e51750363a549ef`
