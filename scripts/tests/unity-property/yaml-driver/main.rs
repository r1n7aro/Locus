//! Exercises the production YAML editor against real Unity-produced fixtures.
#![allow(dead_code, unused_imports)]
#[path = "../../../../src-tauri/src/unity_asset_core/mod.rs"]
mod unity_asset_core;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read};
use unity_asset_core::{AssetOperation, PackedElement, ScalarHint};

fn run() -> Result<Value, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let request: Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    let source = request["source"].as_str().ok_or("source required")?;
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let object = request["object_id"].as_str().ok_or("object_id required")?;
    // These hints are proven by the checked-in ReviewData fixture, not guessed
    // from YAML spelling. Production obtains them from ProjectSchema.
    let fields = BTreeMap::from([
        (
            "/MonoBehaviour/numbers".into(),
            ScalarHint::PackedArray(PackedElement::I32),
        ),
        ("/MonoBehaviour/note".into(), ScalarHint::String),
        ("/MonoBehaviour/toggle".into(), ScalarHint::Boolean),
        ("/MonoBehaviour/precise".into(), ScalarHint::Float),
    ]);
    let hints = BTreeMap::from([(object.into(), fields)]);
    if let Some(command)=request.get("authoring") {
        let mut graph=unity_asset_core::prefab::PrefabGraph::default();
        graph.files.insert(source.into(),unity_asset_core::authoring::AuthoringAsset::new(&bytes,hints.clone())?);
        for (guid,path) in request["prefabFiles"].as_object().into_iter().flatten(){
            let path=path.as_str().ok_or("invalid prefab path")?;
            graph.guids.insert(guid.clone(),path.into());
            graph.files.insert(path.into(),unity_asset_core::authoring::AuthoringAsset::new(&std::fs::read(path).map_err(|e|e.to_string())?,Default::default())?);
        }
        let path=request["propertyPath"].as_str().unwrap_or("");
        match command["action"].as_str().unwrap_or("") {
            "createManaged"=>{graph.files.get_mut(source).unwrap().create_managed(object,path,&command["template"])?;},
            "editObjects"=>{
                let file=graph.files.get_mut(source).unwrap();
                file.edit_objects(serde_json::from_value(command["add"].clone()).map_err(|e|e.to_string())?,&serde_json::from_value::<Vec<String>>(command["remove"].clone()).map_err(|e|e.to_string())?)?;
                for update in command["updates"].as_array().into_iter().flatten(){file.set(update["objectId"].as_str().unwrap(),update["propertyPath"].as_str().unwrap(),update["value"].clone())?;}
            },
            "override"|"revert"|"applyToSource"=>{
                let effective=graph.effective(source)?;let object=effective.get(object).ok_or("effective object missing")?;
                if command["action"]=="applyToSource" {
                    let level=command["level"].as_u64().ok_or("level required")? as usize;
                    if level==0||level>object.layers.len(){return Err("invalid level".into());}
                    let value=unity_asset_core::prefab::value_at(&object.object.data,path)?.clone();
                    if level==object.layers.len(){graph.files.get_mut(&object.source_path).unwrap().set(&object.source_id,path,value)?;}
                    else{graph.override_value(&object.layers[level],path,Some(value))?;}
                    for layer in &object.layers[..level]{graph.override_value(layer,path,None)?;}
                }else{graph.override_value(object.layers.first().ok_or("layer required")?,path,if command["action"]=="revert"{None}else{Some(command["value"].clone())})?;}
            },
            "read"=>{},
            _=>return Err("unknown authoring action".into()),
        }
        let effective=graph.effective(source)?;
        let value=effective.get(object).and_then(|o|unity_asset_core::prefab::value_at(&o.object.data,path).ok()).cloned();
        let candidates=graph.files.iter().filter_map(|(path,file)| match file.render(true){Ok(after) if after!=file.original=>Some(Ok(json!({"source":path,"text":String::from_utf8(after).unwrap()}))),Ok(_)=>None,Err(e)=>Some(Err(e))}).collect::<Result<Vec<_>,String>>()?;
        return Ok(json!({"text":String::from_utf8(graph.files[source].render(true)?).unwrap(),"candidates":candidates,"value":value,"value_json":serde_json::to_string(&value).map_err(|e|e.to_string())?}));
    }
    let operations: Vec<AssetOperation> =
        if let Some(writes) = request.get("properties").and_then(Value::as_array) {
            let model = unity_asset_core::semantic::SemanticAsset::new(
                unity_asset_core::inspect_with_hints(&bytes, &hints).map_err(|e| e.to_string())?,
            )?;
            let logical = writes
                .iter()
                .map(|write| {
                    Ok((
                        object.to_string(),
                        write["propertyPath"]
                            .as_str()
                            .ok_or("propertyPath required")?
                            .to_string(),
                        write["value"].clone(),
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            model.lower_writes(&logical)?
        } else {
            serde_json::from_value(request["operations"].clone()).map_err(|e| e.to_string())?
        };
    let output = unity_asset_core::edit_with_hints(&bytes, &operations, &hints)
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"text": String::from_utf8(output.bytes).map_err(|e| e.to_string())?, "snapshot": output.snapshot}),
    )
}

fn main() {
    match run() {
        Ok(value) => println!("{}", value),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    }
}
