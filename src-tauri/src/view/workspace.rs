// Shared workspace support for existing packages. This initializes a library, not a View.
pub(super) fn view_workspace_package_json() -> &'static str {
    r#"{
  "name": "locus-view-workspace",
  "private": true,
  "type": "module"
}
"#
}

pub(super) fn view_workspace_tsconfig_json() -> &'static str {
    r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@locus/project-view": ["src/index.ts"],
      "@locus/project-view/*": ["src/*"],
      "@project-view": ["src/index.ts"],
      "@project-view/*": ["src/*"]
    }
  }
}
"#
}

pub(super) fn view_workspace_index_ts() -> &'static str {
    r#"export * from "./propertyDraw";
"#
}

pub(super) fn view_workspace_property_draw_ts() -> &'static str {
    r#"import {
  publicInspectorPropertyDrawerLibrary,
  registerInspectorPropertyDrawer,
  type InspectorPropertyDrawerRegistration,
} from "@locus/view-runtime";

export const projectPropertyDrawerLibrary = publicInspectorPropertyDrawerLibrary;

export function registerProjectPropertyDrawer(
  registration: InspectorPropertyDrawerRegistration,
) {
  if (
    !registration.type &&
    !registration.valueType &&
    !registration.fieldType &&
    !registration.attribute &&
    !registration.propertyPath &&
    !registration.name &&
    !registration.drawerKind &&
    !registration.match
  ) return () => undefined;
  return projectPropertyDrawerLibrary.register(registration);
}

export { registerInspectorPropertyDrawer };
"#
}

pub(super) fn view_workspace_readme_md() -> &'static str {
    r#"# Locus View Workspace

Project-wide View frontend code for this Unity project.

Import from `@locus/project-view` or `@project-view` inside View packages.
"#
}
