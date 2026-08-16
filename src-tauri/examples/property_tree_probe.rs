use locus_lib::unity_serialized_property::property_tree::{
    read_live_property_tree_with_limits, search_live_property_tree, PropertyTreePath,
    PropertyTreeSearchOptions,
};

#[tokio::main]
async fn main() -> Result<(), String> {
    let project = std::env::args()
        .nth(1)
        .ok_or_else(|| "usage: property_tree_probe <Unity project>".to_string())?;
    let probes = [
        (
            "Assets/Assets/ECSPrototype/Scenes/EloraECSPrototype.unity",
            "showAttackCapsuleDebug",
            "all",
        ),
        (
            "Assets/Assets/ECSPrototype/Scenes/EloraECSPrototype.unity/ECS Prototype/KalanECSPrototype",
            "debug",
            "all",
        ),
        (
            "Assets/Assets/ECSPrototype/Scenes/EloraECSPrototype.unity",
            "EloraEcsPrototypeBridge",
            "type",
        ),
        (
            "Assets/Assets/ECSPrototype/Prefabs/Entity/ECSPrototypeTargetEntity.prefab",
            "radius",
            "field_name,field_value",
        ),
        (
            "Assets/Assets/ECSPrototype/Prefabs/Entity/ECSPrototypeTargetEntity.prefab",
            "health",
            "all",
        ),
        (
            "Assets/Assets/ECSPrototype/Prefabs/Entity/ECSPrototypeTargetEntity.prefab",
            "0.5",
            "all",
        ),
    ];

    for (scope, query, fields) in probes {
        let path = PropertyTreePath::parse(&project, scope)?;
        let result = search_live_property_tree(
            &project,
            &path,
            &PropertyTreeSearchOptions {
                query: query.to_string(),
                match_fields: vec![fields.to_string()],
                limit: 50,
            },
        )
        .await?;
        println!(
            "PROBE scope={scope:?} query={query:?} matches={} truncated={} scanned={}/{}",
            result.matches.len(),
            result.traversal_truncated,
            result.scanned_objects,
            result.scanned_properties,
        );
        for item in &result.matches {
            println!(
                "  {} | {} | {}",
                item.path, item.property_type, item.display_value
            );
            let read_path = PropertyTreePath::parse(&project, &item.path)?;
            let read = read_live_property_tree_with_limits(&project, &read_path, 1, 4).await?;
            println!("    READ {} | {}", read.name, read.display_value);
        }
    }
    Ok(())
}
