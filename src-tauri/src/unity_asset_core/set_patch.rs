//! Minimal typed writes. Setting a field to a JSON value rewrites only what
//! differs from the existing serialized node: unchanged scalars, comments and
//! elements keep their bytes, identity-keyed elements are matched by identity,
//! and replaced scalars keep their lexical style where that is unambiguous.
//! A shape the patch cannot express re-renders the smallest enclosing entry,
//! element or flow value, which is the previous whole-value behavior.

use serde_json::Value;

use super::identity::{
    identity_scalar, is_packed_marker, json_pptr, json_sequence_keys, matches_value, quoted,
    replacement_string, value_matches,
};
use super::merge::{json_yaml, json_yaml_unity, relative_order_changed, sequence_insertion, sequence_keys, starts_line, yaml_key};
use super::validation::{path_child, pptr};
use super::{Asset, CoreError, Entry, Item, Node, NodeKind, ScalarStyle, Span};

/// The serialized context that owns a node and can be rewritten as a whole.
#[derive(Clone, Copy)]
pub(super) enum Holder<'a> {
    /// Complete block-mapping entry lines.
    Entry(&'a Entry),
    /// Complete block-sequence element lines.
    Item(&'a Item),
    /// A value inside a flow collection; only its own span is replaced.
    Inline,
}

pub(super) struct Patcher<'a> {
    asset: &'a Asset,
    /// Replaced spans and bytes; `true` marks an inline fill of an empty
    /// scalar, which stays on its line instead of starting a new one.
    pub(super) patches: Vec<(Span, Vec<u8>, bool)>,
}

impl<'a> Patcher<'a> {
    pub(super) fn new(asset: &'a Asset) -> Self {
        Self { asset, patches: Vec::new() }
    }

    fn newline(&self) -> &'static str {
        self.asset.newline
    }

    /// `identity`: the node is a numeric reference identity (`fileID`/`rid`),
    /// where strings are never equal to numbers. `quote`: write strings
    /// double-quoted, as for a directly addressed scalar (the typed asset
    /// contract); scalars changed inside a container keep their lexical class.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set(
        &mut self,
        node: &Node,
        holder: Holder<'_>,
        value: &Value,
        path: &str,
        flow: bool,
        identity: bool,
        quote: bool,
    ) -> Result<(), CoreError> {
        if matches_value(self.asset, node, value, identity) {
            return Ok(());
        }
        match (&node.kind, value) {
            (
                NodeKind::Scalar(style),
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null,
            ) if !matches!(style, ScalarStyle::Literal | ScalarStyle::Folded)
                && !self.asset.text(node.span).contains('\n') =>
            {
                let mut text = match value {
                    Value::String(text) if identity || quote => {
                        let raw = self.asset.text(node.span);
                        quoted(text, raw.contains("\\u") || raw.contains("\\U"))
                    }
                    Value::String(text) => replacement_string(self.asset, node, text, flow),
                    other => json_yaml(other)?,
                };
                let inline = node.span.start == node.span.end;
                if inline
                    && matches!(self.asset.bytes[..node.span.start].last(), Some(b':' | b'-'))
                {
                    // An empty scalar sits directly after `key:` or `-`.
                    text.insert(0, ' ');
                }
                self.patches.push((node.span, text.into_bytes(), inline));
                Ok(())
            }
            (NodeKind::Mapping(entries), Value::Object(map))
                if !pptr(node) && !json_pptr(value) && !is_packed_marker(value) =>
            {
                if node.flow {
                    let same_keys = entries.len() == map.len()
                        && entries.iter().all(|entry| map.contains_key(&entry.key));
                    if !same_keys {
                        return self.rerender(node, holder, value);
                    }
                    for entry in entries {
                        let identity = identity_scalar(node, &entry.key);
                        self.set(&entry.value, Holder::Inline, &map[&entry.key], &path_child(path, &entry.key), true, identity, identity)?;
                    }
                    return Ok(());
                }
                let removed: Vec<&Entry> =
                    entries.iter().filter(|entry| !map.contains_key(&entry.key)).collect();
                if map.is_empty()
                    || removed.iter().any(|entry| !starts_line(&self.asset.bytes, entry.span.start))
                {
                    return self.rerender(node, holder, value);
                }
                for entry in entries {
                    if let Some(child) = map.get(&entry.key) {
                        let identity = identity_scalar(node, &entry.key);
                        self.set(&entry.value, Holder::Entry(entry), child, &path_child(path, &entry.key), false, identity, identity)?;
                    }
                }
                for entry in removed {
                    self.patches.push((entry.span, Vec::new(), false));
                }
                let mut added = String::new();
                for (key, child) in map {
                    if entries.iter().any(|entry| &entry.key == key) {
                        continue;
                    }
                    added.push_str(&format!(
                        "{}{}: {}{}",
                        " ".repeat(node.indent),
                        yaml_key(key),
                        json_yaml_unity(child, true)?,
                        self.newline()
                    ));
                }
                if !added.is_empty() {
                    self.patches
                        .push((Span::new(node.span.end, node.span.end), added.into_bytes(), false));
                }
                Ok(())
            }
            (NodeKind::Sequence(items), Value::Array(values)) => {
                if node.flow {
                    if items.len() != values.len() {
                        return self.rerender(node, holder, value);
                    }
                    for (index, (item, child)) in items.iter().zip(values).enumerate() {
                        self.set(&item.value, Holder::Inline, child, &path_child(path, &index.to_string()), true, false, false)?;
                    }
                    return Ok(());
                }
                // A compact nested sequence's first element shares the
                // enclosing `- ` line, so element splices cannot address it.
                if values.is_empty()
                    || items.iter().any(|item| !starts_line(&self.asset.bytes, item.span.start))
                {
                    return self.rerender(node, holder, value);
                }
                match (sequence_keys(self.asset, node, path), json_sequence_keys(path, values)) {
                    (Some(node_keys), Some(value_keys)) => {
                        self.keyed(node, items, &node_keys, values, &value_keys, holder, value, path)
                    }
                    _ => self.positional(node, items, values, path),
                }
            }
            _ => self.rerender(node, holder, value),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn keyed(
        &mut self,
        node: &Node,
        items: &[Item],
        node_keys: &[String],
        values: &[Value],
        value_keys: &[String],
        holder: Holder<'_>,
        value: &Value,
        path: &str,
    ) -> Result<(), CoreError> {
        if relative_order_changed(node_keys, value_keys) {
            return self.rerender(node, holder, value);
        }
        let node_index: std::collections::HashMap<String, usize> =
            node_keys.iter().enumerate().map(|(i, k)| (k.clone(), i)).collect();
        let value_index: std::collections::HashMap<&str, usize> =
            value_keys.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
        for (item, key) in items.iter().zip(node_keys) {
            match value_index.get(key.as_str()) {
                Some(&index) => {
                    self.set(&item.value, Holder::Item(item), &values[index], &path_child(path, key), false, false, false)?
                }
                None => self.patches.push((item.span, Vec::new(), false)),
            }
        }
        for (position, child) in values.iter().enumerate() {
            if node_index.contains_key(&value_keys[position]) {
                continue;
            }
            let (at, _) = sequence_insertion(value_keys, position, &node_index, items, node);
            let bytes = self.render_item(child, node.indent, items.first().map(|i| &i.value))?;
            self.patches.push((Span::new(at, at), bytes.into_bytes(), false));
        }
        Ok(())
    }

    fn positional(
        &mut self,
        node: &Node,
        items: &[Item],
        values: &[Value],
        path: &str,
    ) -> Result<(), CoreError> {
        let mut prefix = 0;
        while prefix < items.len()
            && prefix < values.len()
            && value_matches(self.asset, &items[prefix].value, &values[prefix])
        {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < items.len() - prefix
            && suffix < values.len() - prefix
            && value_matches(
                self.asset,
                &items[items.len() - 1 - suffix].value,
                &values[values.len() - 1 - suffix],
            )
        {
            suffix += 1;
        }
        let old = &items[prefix..items.len() - suffix];
        let new = &values[prefix..values.len() - suffix];
        if old.len() == new.len() {
            for (offset, (item, child)) in old.iter().zip(new).enumerate() {
                self.set(&item.value, Holder::Item(item), child, &path_child(path, &(prefix + offset).to_string()), false, false, false)?;
            }
            return Ok(());
        }
        for item in old {
            self.patches.push((item.span, Vec::new(), false));
        }
        if !new.is_empty() {
            let at = old
                .first()
                .map(|item| item.span.start)
                .or_else(|| prefix.checked_sub(1).map(|i| items[i].span.end))
                .unwrap_or(items[0].span.start);
            let template = items.first().map(|i| &i.value);
            let mut bytes = String::new();
            for child in new {
                bytes.push_str(&self.render_item(child, node.indent, template)?);
            }
            self.patches.push((Span::new(at, at), bytes.into_bytes(), false));
        }
        Ok(())
    }

    fn rerender(&mut self, node: &Node, holder: Holder<'_>, value: &Value) -> Result<(), CoreError> {
        let leading = |position: usize| {
            self.asset.bytes[position..]
                .iter()
                .take_while(|b| **b == b' ')
                .count()
        };
        match holder {
            Holder::Entry(entry) => {
                let text = format!(
                    "{}{}: {}{}",
                    " ".repeat(leading(entry.span.start)),
                    yaml_key(&entry.key),
                    json_yaml_unity(value, true)?,
                    self.newline()
                );
                self.patches.push((entry.span, text.into_bytes(), false));
            }
            Holder::Item(item) => {
                let template = Some(&item.value);
                let start = item.span.start;
                let text = if starts_line(&self.asset.bytes, start) {
                    self.render_item(value, leading(start), template)?
                } else {
                    // The first element of a compact nested sequence keeps the
                    // enclosing `- ` already written before it on the line.
                    let line_start = self.asset.bytes[..start]
                        .iter()
                        .rposition(|b| *b == b'\n')
                        .map_or(0, |i| i + 1);
                    let column = start - line_start;
                    let mut text = self.render_item(value, column, template)?;
                    text.replace_range(..column, "");
                    text
                };
                self.patches.push((item.span, text.into_bytes(), false));
            }
            Holder::Inline => {
                self.patches
                    .push((node.span, json_yaml_unity(value, false)?.into_bytes(), false));
            }
        }
        Ok(())
    }

    /// One block-sequence element. Mappings become Unity-style block elements
    /// whose key order follows an existing sibling; nested values stay flow.
    pub(super) fn render_item(
        &self,
        value: &Value,
        indent: usize,
        template: Option<&Node>,
    ) -> Result<String, CoreError> {
        let pad = " ".repeat(indent);
        let newline = self.newline();
        if let (Value::Array(values), Some(template)) = (value, template) {
            if !values.is_empty() && !template.flow && template.items().is_some() {
                // Unity's compact nested form: the outer marker takes the
                // first inner element's indentation (`- - a` / `  - b`).
                let inner = template.items().and_then(|items| items.first()).map(|item| &item.value);
                let mut out = String::new();
                for child in values {
                    out.push_str(&self.render_item(child, indent + 2, inner)?);
                }
                out.replace_range(..indent + 2, &format!("{pad}- "));
                return Ok(out);
            }
        }
        let Value::Object(map) = value else {
            return Ok(format!("{pad}- {}{newline}", json_yaml_unity(value, true)?));
        };
        if map.is_empty()
            || json_pptr(value)
            || is_packed_marker(value)
            || template.is_some_and(|template| template.flow)
        {
            // Elements keep their siblings' flow style, e.g. `- {x: 0, y: 0}`.
            return Ok(format!("{pad}- {}{newline}", json_yaml_unity(value, true)?));
        }
        let template_entries = template.and_then(Node::entries).unwrap_or(&[]);
        let mut keys: Vec<&String> = template_entries
            .iter()
            .filter_map(|entry| map.get_key_value(&entry.key).map(|(key, _)| key))
            .collect();
        for key in map.keys() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let mut out = String::new();
        for (index, key) in keys.into_iter().enumerate() {
            let child = &map[key];
            let sibling = template_entries
                .iter()
                .find(|entry| &entry.key == key)
                .map(|entry| &entry.value)
                .filter(|node| node.is_scalar());
            let rendered = match (child, sibling) {
                (Value::String(text), Some(sibling)) => {
                    replacement_string(self.asset, sibling, text, false)
                }
                _ => json_yaml_unity(child, true)?,
            };
            let lead = if index == 0 { format!("{pad}- ") } else { format!("{pad}  ") };
            out.push_str(&format!("{lead}{}: {rendered}{newline}", yaml_key(key)));
        }
        Ok(out)
    }
}
