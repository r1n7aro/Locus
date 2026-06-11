use super::{
    now_millis, ViewCapabilities, ViewManifest, ViewRequirements, ViewScriptManifest,
    ViewTemplateSummary, VIEW_API_VERSION, VIEW_SCHEMA,
};

mod blank;
mod canvas_board;
mod common;
mod field_blocks;
mod inspector_form;
mod link_board;
mod node_graph;
mod serialized_table;

pub(super) fn supported_view_templates() -> Vec<ViewTemplateSummary> {
    vec![
        ViewTemplateSummary {
            id: "blank".to_string(),
            name: "Blank".to_string(),
            description: "Minimal editable View package.".to_string(),
        },
        ViewTemplateSummary {
            id: "inspector-form".to_string(),
            name: "Inspector Form".to_string(),
            description: "Field-oriented form scaffold for Unity data.".to_string(),
        },
        ViewTemplateSummary {
            id: "canvas-board".to_string(),
            name: "Canvas Board".to_string(),
            description: "Freeform canvas scaffold with draggable custom blocks.".to_string(),
        },
        ViewTemplateSummary {
            id: "field-blocks".to_string(),
            name: "Field Blocks".to_string(),
            description: "Freeform canvas scaffold for Unity SerializedProperty field blocks."
                .to_string(),
        },
        ViewTemplateSummary {
            id: "node-graph".to_string(),
            name: "Node Graph".to_string(),
            description: "Canvas-backed node graph scaffold with scripted Unity write-back."
                .to_string(),
        },
        ViewTemplateSummary {
            id: "link-board".to_string(),
            name: "Link Board".to_string(),
            description: "Two-column link mapping scaffold with serialized connections."
                .to_string(),
        },
        ViewTemplateSummary {
            id: "serialized-table".to_string(),
            name: "Serialized Table".to_string(),
            description: "Table scaffold for browsing and editing Unity serialized properties."
                .to_string(),
        },
    ]
}

pub(super) fn is_supported_template(template: &str) -> bool {
    matches!(
        template,
        "blank"
            | "inspector-form"
            | "canvas-board"
            | "field-blocks"
            | "node-graph"
            | "link-board"
            | "serialized-table"
    )
}

fn default_icon_for_template(template: &str) -> &'static str {
    match template {
        "canvas-board" => "PanelsTopLeft",
        "field-blocks" => "FormInput",
        "inspector-form" => "InspectionPanel",
        "node-graph" => "Network",
        "link-board" => "Link2",
        "serialized-table" => "TableProperties",
        _ => "View",
    }
}

pub(super) fn template_manifest(
    id: &str,
    name: &str,
    template: &str,
    icon: Option<&str>,
) -> ViewManifest {
    let scripts = match template {
        "inspector-form" => vec![ViewScriptManifest {
            name: "InspectorViewApi".to_string(),
            path: "unity/ViewApi.cs".to_string(),
            entry_type: "InspectorViewApi".to_string(),
        }],
        "node-graph" => vec![ViewScriptManifest {
            name: "GraphViewApi".to_string(),
            path: "unity/ViewApi.cs".to_string(),
            entry_type: "GraphViewApi".to_string(),
        }],
        "serialized-table" => vec![ViewScriptManifest {
            name: "SerializedTableApi".to_string(),
            path: "unity/ViewApi.cs".to_string(),
            entry_type: "SerializedTableApi".to_string(),
        }],
        _ => Vec::new(),
    };

    ViewManifest {
        schema: VIEW_SCHEMA.to_string(),
        api_version: VIEW_API_VERSION.to_string(),
        id: id.to_string(),
        name: name.to_string(),
        version: "0.1.0".to_string(),
        template: template.to_string(),
        display_path: None,
        icon: Some(
            icon.unwrap_or_else(|| default_icon_for_template(template))
                .to_string(),
        ),
        entry: "src/main.ts".to_string(),
        style: "src/style.css".to_string(),
        scripts,
        capabilities: ViewCapabilities {
            unity: matches!(
                template,
                "inspector-form" | "field-blocks" | "node-graph" | "serialized-table"
            ),
        },
        requirements: Some(ViewRequirements {
            unity_connection: matches!(
                template,
                "inspector-form" | "field-blocks" | "node-graph" | "serialized-table"
            ),
        }),
    }
}

pub(super) fn template_files(id: &str, name: &str, template: &str) -> Vec<(&'static str, String)> {
    let created_at = now_millis();
    let readme = format!(
        "# {name}\n\nView Package `{id}` generated from `{template}`.\n\nEdit files under this directory and reload the View from Locus.\n"
    );

    let mut files = vec![
        ("README.md", readme),
        ("src/main.ts", common::main_ts()),
    ];

    match template {
        "inspector-form" => {
            files.push(("src/App.vue", inspector_form::app_vue(name)));
            files.push(("src/style.css", inspector_form::style_css()));
            files.push(("unity/ViewApi.cs", inspector_form::view_api_cs()));
        }
        "canvas-board" => {
            files.push(("src/App.vue", canvas_board::app_vue(name)));
            files.push(("src/style.css", canvas_board::style_css()));
        }
        "field-blocks" => {
            files.push(("src/App.vue", field_blocks::app_vue(name)));
            files.push(("src/style.css", field_blocks::style_css()));
        }
        "node-graph" => {
            files.push(("src/App.vue", node_graph::app_vue(name)));
            files.push(("src/style.css", node_graph::style_css()));
            files.push(("unity/ViewApi.cs", node_graph::view_api_cs()));
        }
        "link-board" => {
            files.push(("src/App.vue", link_board::app_vue(name)));
            files.push(("src/style.css", link_board::style_css()));
        }
        "serialized-table" => {
            files.push(("src/App.vue", serialized_table::app_vue(name)));
            files.push(("src/tableConfig.ts", serialized_table::table_config_ts()));
            files.push(("src/style.css", serialized_table::style_css()));
            files.push(("unity/ViewApi.cs", serialized_table::view_api_cs()));
        }
        _ => {
            files.push(("src/App.vue", blank::app_vue(name)));
            files.push(("src/style.css", blank::style_css()));
        }
    }

    files.push((
        ".locus-view",
        format!("createdAt={created_at}\ntemplate={template}\n"),
    ));
    files
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn view_create_tool_json() -> serde_json::Value {
        serde_json::from_str(crate::prompt::tools::VIEW_CREATE)
            .expect("tools/view_create.json must be valid JSON")
    }

    #[test]
    fn template_registry_matches_view_create_tool_schema() {
        let json = view_create_tool_json();
        let enum_values: Vec<String> = json["parameters"]["properties"]["template"]["enum"]
            .as_array()
            .expect("template enum must be an array")
            .iter()
            .map(|value| value.as_str().expect("template enum entry").to_string())
            .collect();
        let registry_ids: Vec<String> = supported_view_templates()
            .iter()
            .map(|template| template.id.clone())
            .collect();

        assert_eq!(
            registry_ids, enum_values,
            "supported_view_templates() and the template enum in tools/view_create.json must list the same templates in the same order",
        );
        for id in &registry_ids {
            assert!(is_supported_template(id), "registry template must be supported: {id}");
        }
        assert!(!is_supported_template("unknown-template"));

        let description = json["parameters"]["properties"]["template"]["description"]
            .as_str()
            .unwrap_or_default();
        for id in &registry_ids {
            assert!(
                description.contains(id),
                "template enum description must mention {id}",
            );
        }
    }

    #[test]
    fn template_default_icons_are_in_tool_icon_enum() {
        let json = view_create_tool_json();
        let icons: Vec<&str> = json["parameters"]["properties"]["icon"]["enum"]
            .as_array()
            .expect("icon enum must be an array")
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        for template in supported_view_templates() {
            let icon = default_icon_for_template(&template.id);
            assert!(
                icons.contains(&icon),
                "default icon for {} must be part of the icon enum: {icon}",
                template.id,
            );
        }
    }

    #[test]
    fn template_files_share_base_layout() {
        for template in supported_view_templates() {
            let files = template_files("test-view", "Test View", &template.id);
            let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
            for required in ["README.md", "src/main.ts", "src/App.vue", "src/style.css", ".locus-view"] {
                assert!(
                    names.contains(&required),
                    "{} must generate {required}",
                    template.id,
                );
            }

            let style = &files
                .iter()
                .find(|(name, _)| *name == "src/style.css")
                .expect("style.css")
                .1;
            assert!(
                style.starts_with(common::base_css()),
                "{} style.css must start with the shared base stylesheet",
                template.id,
            );

            let app = &files
                .iter()
                .find(|(name, _)| *name == "src/App.vue")
                .expect("App.vue")
                .1;
            assert!(
                app.contains(&format!("data-locus-template=\"{}\"", template.id)),
                "{} App.vue must tag its root with data-locus-template",
                template.id,
            );
        }
    }

    #[test]
    fn unity_templates_declare_capabilities_and_scripts() {
        for template in supported_view_templates() {
            let manifest = template_manifest("test-view", "Test View", &template.id, None);
            let needs_unity = matches!(
                template.id.as_str(),
                "inspector-form" | "field-blocks" | "node-graph" | "serialized-table"
            );
            assert_eq!(
                manifest.capabilities.unity, needs_unity,
                "unity capability mismatch for {}",
                template.id,
            );

            let has_script_file = template_files("test-view", "Test View", &template.id)
                .iter()
                .any(|(name, _)| *name == "unity/ViewApi.cs");
            assert_eq!(
                !manifest.scripts.is_empty(),
                has_script_file,
                "{} must generate unity/ViewApi.cs exactly when view.json declares scripts",
                template.id,
            );
            for script in &manifest.scripts {
                assert!(!script.entry_type.trim().is_empty());
                assert_eq!(script.path, "unity/ViewApi.cs");
            }
        }
    }
}
