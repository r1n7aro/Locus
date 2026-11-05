//! Setting labels and descriptions for agent-facing output, read from the UI
//! language catalog (`settings.item.<key>.label` / `.desc`) so there is a
//! single description per setting.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN_CATALOG: &str = include_str!("../../../src/language/en.json");
#[cfg(test)]
const ZH_CATALOG: &str = include_str!("../../../src/language/zh.json");
const PREFIX: &str = "settings.item.";

fn setting_entries(catalog: &str) -> HashMap<String, String> {
    serde_json::from_str::<HashMap<String, String>>(catalog)
        .unwrap_or_default()
        .into_iter()
        .filter(|(key, _)| key.starts_with(PREFIX))
        .collect()
}

fn english() -> &'static HashMap<String, String> {
    static ENTRIES: OnceLock<HashMap<String, String>> = OnceLock::new();
    ENTRIES.get_or_init(|| setting_entries(EN_CATALOG))
}

pub fn label(key: &str) -> Option<&'static str> {
    english()
        .get(&format!("{PREFIX}{key}.label"))
        .map(String::as_str)
}

pub fn description(key: &str) -> Option<&'static str> {
    english()
        .get(&format!("{PREFIX}{key}.desc"))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::definitions::DEFINITIONS;

    #[test]
    fn every_setting_has_a_label_and_description_in_both_languages() {
        for (language, catalog) in [("en", EN_CATALOG), ("zh", ZH_CATALOG)] {
            let entries = setting_entries(catalog);
            for def in DEFINITIONS {
                for suffix in ["label", "desc"] {
                    let key = format!("{PREFIX}{}.{suffix}", def.key);
                    assert!(
                        entries.get(&key).is_some_and(|value| !value.trim().is_empty()),
                        "src/language/{language}.json is missing {key}"
                    );
                }
            }
        }
    }
}
