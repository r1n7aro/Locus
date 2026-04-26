# Roslyn 3.8.0 Bundle Inputs

This directory keeps the original Roslyn and supporting DLLs used to build `locus_unity/Editor/Roslyn/Locus.Roslyn.dll`.

Run this command after changing any input DLL:

```sh
bun run unity:bundle-roslyn
```

The Unity package distributes the merged `Locus.Roslyn.dll`. The original DLLs remain in this repository so the bundle can be rebuilt without retrieving different upstream artifacts.
