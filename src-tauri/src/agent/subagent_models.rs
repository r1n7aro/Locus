//! Per-agent model allow-lists for delegated subagents.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

/// Models each subagent type may run on, keyed by agent id.
///
/// The first model is the default when the delegating agent does not choose
/// one. A missing entry inherits the parent session's model and offers no
/// choice. Settings written before multi-select stored one model string per
/// agent; those values still deserialize as a single-model list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SubagentModelChoices(HashMap<String, Vec<String>>);

impl SubagentModelChoices {
    /// Allowed models for an agent type; empty when the type inherits.
    pub fn allowed(&self, agent_id: &str) -> &[String] {
        self.0.get(agent_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Replaces an agent's list. Blank and duplicate ids are dropped, and an
    /// empty list removes the entry so the agent inherits again.
    pub fn set<I, S>(&mut self, agent_id: impl Into<String>, models: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let agent_id = agent_id.into();
        let models = normalize(models);
        if models.is_empty() {
            self.0.remove(&agent_id);
        } else {
            self.0.insert(agent_id, models);
        }
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Keeps only models accepted by `keep`; returns whether anything changed.
    pub fn retain_models(&mut self, mut keep: impl FnMut(&str) -> bool) -> bool {
        let mut changed = false;
        self.0.retain(|_, models| {
            let before = models.len();
            models.retain(|model| keep(model));
            changed |= models.len() != before;
            !models.is_empty()
        });
        changed
    }

    /// Replaces model ids for which `rewrite` returns a new id; returns whether
    /// anything changed. Lists stay deduplicated in their original order.
    pub fn rewrite_models(&mut self, mut rewrite: impl FnMut(&str) -> Option<String>) -> bool {
        let mut changed = false;
        for models in self.0.values_mut() {
            let rewritten = models
                .iter()
                .map(|model| match rewrite(model) {
                    Some(next) => {
                        changed = true;
                        next
                    }
                    None => model.clone(),
                })
                .collect::<Vec<_>>();
            *models = normalize(rewritten);
        }
        changed
    }
}

fn normalize<I, S>(models: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized = Vec::<String>::new();
    for model in models {
        let model = model.as_ref().trim();
        if !model.is_empty() && !normalized.iter().any(|existing| existing == model) {
            normalized.push(model.to_string());
        }
    }
    normalized
}

impl<'de> Deserialize<'de> for SubagentModelChoices {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Entry {
            One(String),
            Many(Vec<String>),
        }

        let raw = HashMap::<String, Option<Entry>>::deserialize(deserializer)?;
        let mut choices = Self::default();
        for (agent_id, entry) in raw {
            match entry {
                Some(Entry::One(model)) => choices.set(agent_id, [model]),
                Some(Entry::Many(models)) => choices.set(agent_id, models),
                None => {}
            }
        }
        Ok(choices)
    }
}

#[cfg(test)]
mod tests {
    use super::SubagentModelChoices;
    use serde_json::json;

    #[test]
    fn accepts_legacy_single_model_values_and_lists() {
        let choices: SubagentModelChoices = serde_json::from_value(json!({
            "explorer": "openai/gpt-5.6-luna",
            "researcher": [" openai/gpt-5.6-luna ", "", "openai/gpt-5.6-sol", "openai/gpt-5.6-luna"],
            "doc": [],
            "wiki": "  ",
            "simple": null,
        }))
        .unwrap();

        assert_eq!(choices.allowed("explorer"), ["openai/gpt-5.6-luna"]);
        assert_eq!(
            choices.allowed("researcher"),
            ["openai/gpt-5.6-luna", "openai/gpt-5.6-sol"]
        );
        for inherited in ["doc", "wiki", "simple", "unknown"] {
            assert!(choices.allowed(inherited).is_empty(), "{inherited}");
        }
        assert_eq!(
            serde_json::to_value(&choices).unwrap(),
            json!({
                "explorer": ["openai/gpt-5.6-luna"],
                "researcher": ["openai/gpt-5.6-luna", "openai/gpt-5.6-sol"],
            })
        );
    }

    #[test]
    fn retain_and_rewrite_drop_emptied_entries_and_duplicates() {
        let mut choices = SubagentModelChoices::default();
        choices.set("explorer", ["custom/gone/a", "custom/kept/b"]);
        choices.set("doc", ["custom/gone/c"]);
        assert!(choices.retain_models(|model| !model.starts_with("custom/gone/")));
        assert_eq!(choices.allowed("explorer"), ["custom/kept/b"]);
        assert!(choices.allowed("doc").is_empty());
        assert!(!choices.retain_models(|_| true));

        choices.set("explorer", ["custom/legacy", "custom/legacy/first"]);
        assert!(choices.rewrite_models(|model| {
            (model == "custom/legacy").then(|| "custom/legacy/first".to_string())
        }));
        assert_eq!(choices.allowed("explorer"), ["custom/legacy/first"]);
    }
}
