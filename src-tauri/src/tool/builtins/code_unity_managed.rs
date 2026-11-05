//! `[SerializeReference]` (managed reference) analysis for
//! `unity_code_usages` and the serialized-type check in `code_diagnostics`.
//!
//! Managed-reference instances are identified only by `{class, ns, asm}` —
//! no script GUID — so this joins three sources:
//! - the `managed_refs` index (what assets serialize, `asset_db`),
//! - C# declarations of every type in a file (`unity_csharp::declared_types`),
//! - Unity's script → assembly assignment (`unity_csharp::assembly`).
//!
//! Resolution of a serialized identity to a declaration takes the file-name
//! fast path first (Unity projects overwhelmingly name files after their
//! type) and falls back to a single declaration / `[MovedFrom]` text scan
//! over every indexed script only when that misses.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::asset_db::types::{guid_to_hex, Guid};
use crate::asset_db::{
    AssetDb, ManagedRefHit, ManagedRefTypeSummary, PrefabOverrideHit, ScriptIndexEntry,
};
use crate::unity_csharp::assembly::{AssemblyResolver, PREDEFINED_ASSEMBLIES};
use crate::unity_csharp::{parse_cs_declared_types, CsFileTypes, CsTypeRef, DeclaredCsType};
use crate::unity_yaml::managed_refs::{
    managed_class_key, managed_class_segments, ManagedClassSegment, ManagedTypeIdentity,
};
use crate::unity_yaml::prefab_overrides::{
    nested_file_id, parse_managed_reference_path, parse_managed_type_value,
};

pub(super) type SharedAssetDb = Arc<Mutex<Option<AssetDb>>>;

/// Base-walk guard: no real hierarchy is this deep, cycles are cut earlier.
const MAX_BASE_WALK: usize = 256;
/// Sample locations listed per unresolved serialized type.
const MAX_SAMPLES_PER_TYPE: usize = 3;

/// One serialized `{class, ns, asm}` identity with usage counts, from
/// `RefIds` registries and prefab-instance type replacements combined.
#[derive(Debug, Clone)]
pub(super) struct IdentityStat {
    pub identity: ManagedTypeIdentity,
    pub instance_count: u64,
    pub asset_count: u64,
}

/// A prefab-instance `managedReferences[rid]` entry that sets an instance
/// type (`value: <asm> <ns>.<class>`).
#[derive(Debug, Clone)]
pub(super) struct OverrideTypeEntry {
    pub hit: PrefabOverrideHit,
    pub rid: i64,
    pub identity: ManagedTypeIdentity,
}

/// Index data the analysis needs, copied out under the AssetDb lock so
/// file reads and C# parsing never hold it.
pub(super) struct ManagedSnapshot {
    scripts: Vec<ScriptIndexEntry>,
    asmdefs: Vec<(Guid, String)>,
    identities: Vec<IdentityStat>,
    /// Every indexed `managedReferences[...]` prefab modification.
    managed_overrides: Vec<PrefabOverrideHit>,
}

impl ManagedSnapshot {
    pub(super) fn load(db: &AssetDb) -> Result<Self, String> {
        let summaries = db.get_managed_ref_type_summaries()?;
        let managed_overrides =
            db.get_prefab_overrides_by_heads(&["managedReferences".to_string()])?;

        let mut stats: Vec<IdentityStat> = summaries
            .iter()
            .map(|summary| IdentityStat {
                identity: summary_identity(summary),
                instance_count: summary.instance_count,
                asset_count: summary.asset_count,
            })
            .collect();
        let mut index: HashMap<ManagedTypeIdentity, usize> = stats
            .iter()
            .enumerate()
            .map(|(i, stat)| (stat.identity.clone(), i))
            .collect();
        let mut override_assets: HashMap<ManagedTypeIdentity, HashSet<Guid>> = HashMap::new();
        for entry in override_type_entries(&managed_overrides) {
            let slot = *index.entry(entry.identity.clone()).or_insert_with(|| {
                stats.push(IdentityStat {
                    identity: entry.identity.clone(),
                    instance_count: 0,
                    asset_count: 0,
                });
                stats.len() - 1
            });
            stats[slot].instance_count += 1;
            if override_assets
                .entry(entry.identity.clone())
                .or_default()
                .insert(entry.hit.src_guid)
            {
                stats[slot].asset_count += 1;
            }
        }

        Ok(Self {
            scripts: db.list_script_index_entries()?,
            asmdefs: db.list_assets_by_ext("asmdef")?,
            identities: stats,
            managed_overrides,
        })
    }

    /// Prefab-instance type replacements (null references excluded).
    pub(super) fn override_type_entries(&self) -> Vec<OverrideTypeEntry> {
        override_type_entries(&self.managed_overrides)
    }

    pub(super) fn identity_stats(&self) -> &[IdentityStat] {
        &self.identities
    }

    /// Every indexed `managedReferences[...]` prefab modification.
    pub(super) fn managed_override_rows(&self) -> &[PrefabOverrideHit] {
        &self.managed_overrides
    }

    /// Canonical index path of a script by GUID (case as stored).
    pub(super) fn script_path(&self, guid: &Guid) -> Option<&str> {
        self.scripts
            .iter()
            .find(|entry| &entry.guid == guid)
            .map(|entry| entry.path.as_str())
    }

    fn script_class_by_guid(&self) -> HashMap<Guid, String> {
        self.scripts
            .iter()
            .map(|entry| {
                let stem = Path::new(&entry.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                (entry.guid, stem)
            })
            .collect()
    }
}

fn override_type_entries(rows: &[PrefabOverrideHit]) -> Vec<OverrideTypeEntry> {
    rows.iter()
        .filter_map(|hit| {
            let (rid, tail) = parse_managed_reference_path(&hit.property_path)?;
            if !tail.is_empty() {
                return None;
            }
            let identity = parse_managed_type_value(&hit.value)?;
            Some(OverrideTypeEntry {
                hit: hit.clone(),
                rid,
                identity,
            })
        })
        .collect()
}

fn summary_identity(summary: &ManagedRefTypeSummary) -> ManagedTypeIdentity {
    ManagedTypeIdentity {
        class_name: summary.class_name.clone(),
        namespace: summary.namespace.clone(),
        assembly: summary.assembly.clone(),
    }
}

fn hit_identity(hit: &ManagedRefHit) -> ManagedTypeIdentity {
    ManagedTypeIdentity {
        class_name: hit.class_name.clone(),
        namespace: hit.namespace.clone(),
        assembly: hit.assembly.clone(),
    }
}

// ─── type catalog ───────────────────────────────────────────────────────────

/// Handle of one declared type: (parsed file, index into its types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct TypeKey {
    file: usize,
    index: usize,
}

struct ParsedScript {
    path: String,
    assembly: Option<String>,
    types: CsFileTypes,
}

/// Declared names and `[MovedFrom]` hosts across every indexed script,
/// built at most once per catalog.
struct DeclarationScan {
    by_name: HashMap<String, Vec<String>>,
    moved_from_paths: Vec<String>,
    /// `(path, source text)` of every indexed script, kept for token search.
    texts: Vec<(String, String)>,
}

/// Unresolvable base classes that make a type a `UnityEngine.Object`
/// (serialized by reference, attached by script GUID — never a plain value).
const UNITY_OBJECT_BASES: &[&str] = &[
    "Object",
    "Component",
    "Behaviour",
    "MonoBehaviour",
    "ScriptableObject",
    "StateMachineBehaviour",
    "NetworkBehaviour",
    "UIBehaviour",
    "Graphic",
    "MaskableGraphic",
    "Selectable",
    "PlayableAsset",
    "TrackAsset",
    "Editor",
    "EditorWindow",
    "ScriptableSingleton",
];

/// How a serialized `{class, ns, asm}` identity maps onto code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Resolution {
    /// Same nested name, namespace and assembly.
    Exact(TypeKey),
    /// Old identity recorded by `[MovedFrom]` on the declaration.
    Legacy(TypeKey),
    /// Name and namespace match but the type now compiles into another
    /// assembly and no `[MovedFrom]` covers the old one: Unity reports the
    /// instance as a missing type.
    AssemblyMismatch(TypeKey),
    Missing,
}

pub(super) struct TypeCatalog<'a> {
    root: PathBuf,
    snapshot: &'a ManagedSnapshot,
    by_class: HashMap<String, Vec<usize>>,
    files: Vec<ParsedScript>,
    file_by_path: HashMap<String, Option<usize>>,
    assemblies: AssemblyResolver<'static>,
    scan: Option<DeclarationScan>,
}

impl<'a> TypeCatalog<'a> {
    pub(super) fn new(root: &Path, snapshot: &'a ManagedSnapshot) -> Self {
        let mut by_class: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, entry) in snapshot.scripts.iter().enumerate() {
            if !entry.class_lower.is_empty() {
                by_class
                    .entry(entry.class_lower.clone())
                    .or_default()
                    .push(index);
            }
        }
        let asmdef_by_guid: HashMap<String, String> = snapshot
            .asmdefs
            .iter()
            .map(|(guid, path)| (guid_to_hex(guid), path.clone()))
            .collect();
        Self {
            root: root.to_path_buf(),
            snapshot,
            by_class,
            files: Vec::new(),
            file_by_path: HashMap::new(),
            assemblies: AssemblyResolver::new(root, move |guid| asmdef_by_guid.get(guid).cloned()),
            scan: None,
        }
    }

    /// Assemblies compiled from project sources: every asmdef under
    /// `Assets/` / `Packages/` plus Unity's predefined assemblies. Serialized
    /// types from other assemblies (packages in `Library/PackageCache`, the
    /// engine) cannot be verified from source and are skipped.
    pub(super) fn project_assemblies(&mut self) -> HashSet<String> {
        let mut names: HashSet<String> = PREDEFINED_ASSEMBLIES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let paths: Vec<String> = self
            .snapshot
            .asmdefs
            .iter()
            .map(|(_, p)| p.clone())
            .collect();
        for path in paths {
            if let Some(name) = self.assemblies.asmdef_name(&path) {
                names.insert(name);
            }
        }
        names
    }

    fn load_file(&mut self, rel_path: &str) -> Option<usize> {
        let key = rel_path.replace('\\', "/").to_ascii_lowercase();
        if let Some(found) = self.file_by_path.get(&key) {
            return *found;
        }
        let parsed = std::fs::read(self.root.join(rel_path)).ok().map(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            let types = parse_cs_declared_types(&text);
            let assembly = self.assemblies.assembly_for(rel_path).map(|a| a.name);
            self.files.push(ParsedScript {
                path: rel_path.replace('\\', "/"),
                assembly,
                types,
            });
            self.files.len() - 1
        });
        self.file_by_path.insert(key, parsed);
        parsed
    }

    pub(super) fn types_in_file(&mut self, rel_path: &str) -> Vec<TypeKey> {
        match self.load_file(rel_path) {
            Some(file) => (0..self.files[file].types.types.len())
                .map(|index| TypeKey { file, index })
                .collect(),
            None => Vec::new(),
        }
    }

    pub(super) fn file_assembly(&mut self, rel_path: &str) -> Option<String> {
        let file = self.load_file(rel_path)?;
        self.files[file].assembly.clone()
    }

    pub(super) fn decl(&self, key: TypeKey) -> &DeclaredCsType {
        &self.files[key.file].types.types[key.index]
    }

    /// Indexed scripts whose source contains `token` as a whole identifier.
    pub(super) fn files_mentioning(&mut self, token: &str) -> Vec<String> {
        if token.is_empty() {
            return Vec::new();
        }
        let scan = self.ensure_scan();
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        scan.texts
            .iter()
            .filter(|(_, text)| {
                text.match_indices(token).any(|(index, _)| {
                    let before = text[..index].chars().next_back();
                    let after = text[index + token.len()..].chars().next();
                    !before.map(is_ident).unwrap_or(false) && !after.map(is_ident).unwrap_or(false)
                })
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub(super) fn file_path(&self, key: TypeKey) -> &str {
        &self.files[key.file].path
    }

    /// Resolve a type reference written inside `from` (C# lookup order).
    pub(super) fn resolve_type_ref(&mut self, from: TypeKey, type_ref: &CsTypeRef) -> Vec<TypeKey> {
        self.resolve_base(from, type_ref)
    }

    /// Derives from a `UnityEngine.Object` type (MonoBehaviour,
    /// ScriptableObject, …): serialized by reference, never as a value.
    pub(super) fn is_unity_object(&mut self, key: TypeKey) -> bool {
        let mut stack = vec![key];
        let mut visited: HashSet<TypeKey> = HashSet::new();
        while let Some(current) = stack.pop() {
            if visited.len() >= MAX_BASE_WALK || !visited.insert(current) {
                continue;
            }
            for fragment in self.fragments(current) {
                let bases = self.decl(fragment).bases.clone();
                for base in &bases {
                    let resolved = self.resolve_base(fragment, base);
                    if resolved.is_empty() {
                        if UNITY_OBJECT_BASES.contains(&base.simple_name()) {
                            return true;
                        }
                    } else {
                        stack.extend(resolved);
                    }
                }
            }
        }
        false
    }

    /// Script GUID of a declaration Unity can attach by GUID: a top-level
    /// type named after its `.cs` file.
    pub(super) fn script_guid_of(&self, key: TypeKey) -> Option<Guid> {
        let decl = self.decl(key);
        if decl.nested_path.len() != 1 {
            return None;
        }
        let path = &self.files[key.file].path;
        let stem = Path::new(path).file_stem()?.to_string_lossy();
        if stem != decl.name() {
            return None;
        }
        self.snapshot
            .scripts
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(path))
            .map(|entry| entry.guid)
    }

    pub(super) fn assembly_of(&self, key: TypeKey) -> Option<&str> {
        self.files[key.file].assembly.as_deref()
    }

    pub(super) fn display(&self, key: TypeKey) -> String {
        self.decl(key).display_name()
    }

    fn ensure_scan(&mut self) -> &DeclarationScan {
        if self.scan.is_none() {
            let decl_re =
                regex::Regex::new(r"\b(?:class|struct|interface)\s+([A-Za-z_][A-Za-z0-9_]*)")
                    .expect("valid declaration regex");
            let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
            let mut moved_from_paths = Vec::new();
            let mut texts = Vec::with_capacity(self.snapshot.scripts.len());
            for entry in &self.snapshot.scripts {
                let Ok(bytes) = std::fs::read(self.root.join(&entry.path)) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let mut seen: HashSet<&str> = HashSet::new();
                for caps in decl_re.captures_iter(&text) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    if seen.insert(name) {
                        by_name
                            .entry(name.to_string())
                            .or_default()
                            .push(entry.path.clone());
                    }
                }
                if text.contains("MovedFrom") {
                    moved_from_paths.push(entry.path.clone());
                }
                texts.push((entry.path.clone(), text));
            }
            self.scan = Some(DeclarationScan {
                by_name,
                moved_from_paths,
                texts,
            });
        }
        self.scan.as_ref().expect("scan initialized")
    }

    /// Files that may declare a type whose outermost name is `name`: the
    /// file-stem primary-type index, plus (with `allow_scan`) every file
    /// that textually declares it.
    fn candidate_paths(&mut self, name: &str, allow_scan: bool) -> Vec<String> {
        let mut paths: Vec<String> = self
            .by_class
            .get(&name.to_ascii_lowercase())
            .map(|indices| {
                indices
                    .iter()
                    .map(|i| self.snapshot.scripts[*i].path.clone())
                    .collect()
            })
            .unwrap_or_default();
        if allow_scan {
            if let Some(found) = self.ensure_scan().by_name.get(name) {
                paths.extend(found.iter().cloned());
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }

    fn decls_in(&mut self, paths: &[String]) -> Vec<TypeKey> {
        let mut keys = Vec::new();
        for path in paths {
            keys.extend(self.types_in_file(path));
        }
        keys
    }

    /// Serialized nested names + arity + namespace equal (assembly ignored).
    pub(super) fn names_match(&self, identity: &ManagedTypeIdentity, key: TypeKey) -> bool {
        let decl = self.decl(key);
        decl.namespace == identity.namespace
            && segments_match(&managed_class_segments(&identity.class_name), decl)
    }

    /// `[MovedFrom]` legacy identity of `key` equals `identity`.
    pub(super) fn legacy_match(&self, identity: &ManagedTypeIdentity, key: TypeKey) -> bool {
        let decl = self.decl(key);
        let Some(moved) = decl.moved_from.as_ref() else {
            return false;
        };
        let namespace = moved
            .source_namespace
            .clone()
            .unwrap_or_else(|| decl.namespace.clone());
        let mut path = decl.nested_path.clone();
        if let Some(class_name) = moved.source_class_name.as_deref() {
            if class_name.contains(['/', '+']) {
                path = class_name
                    .split(['/', '+'])
                    .map(|s| s.trim().to_string())
                    .collect();
            } else if let Some(last) = path.last_mut() {
                *last = class_name.trim().to_string();
            }
        }
        let assembly = moved
            .source_assembly
            .as_deref()
            .or_else(|| self.assembly_of(key));
        let segments = managed_class_segments(&identity.class_name);
        identity.namespace == namespace
            && segments.len() == path.len()
            && segments.iter().zip(path.iter()).all(|(s, p)| &s.name == p)
            && assembly.map(|a| a == identity.assembly).unwrap_or(true)
    }

    pub(super) fn resolve_identity(&mut self, identity: &ManagedTypeIdentity) -> Resolution {
        let segments = managed_class_segments(&identity.class_name);
        let Some(outer) = segments.first().map(|s| s.name.clone()) else {
            return Resolution::Missing;
        };
        // A same-name declaration in another assembly is only a mismatch once
        // the full scan has also failed to find the exact type (or a
        // [MovedFrom] alias) elsewhere — e.g. in a multi-class file.
        let mut mismatch = None;
        for allow_scan in [false, true] {
            let paths = self.candidate_paths(&outer, allow_scan);
            let keys = self.decls_in(&paths);
            for &key in &keys {
                if self.names_match(identity, key) {
                    if self.assembly_of(key) == Some(identity.assembly.as_str()) {
                        return Resolution::Exact(key);
                    }
                    if self.legacy_match(identity, key) {
                        return Resolution::Legacy(key);
                    }
                    mismatch.get_or_insert(key);
                } else if self.legacy_match(identity, key) {
                    return Resolution::Legacy(key);
                }
            }
            if allow_scan {
                let moved_paths = self.ensure_scan().moved_from_paths.clone();
                for key in self.decls_in(&moved_paths) {
                    if self.legacy_match(identity, key) {
                        return Resolution::Legacy(key);
                    }
                }
            }
        }
        match mismatch {
            Some(key) => Resolution::AssemblyMismatch(key),
            None => Resolution::Missing,
        }
    }

    /// Every declaration fragment of `key`'s type (partial classes may put
    /// the base list on another fragment, possibly in another file).
    fn fragments(&mut self, key: TypeKey) -> Vec<TypeKey> {
        let decl = self.decl(key).clone();
        let own_path = self.files[key.file].path.clone();
        let mut paths = self.candidate_paths(&decl.nested_path[0], false);
        paths.push(own_path);
        paths.sort();
        paths.dedup();
        self.decls_in(&paths)
            .into_iter()
            .filter(|other| {
                let o = self.decl(*other);
                o.namespace == decl.namespace
                    && o.nested_path == decl.nested_path
                    && o.arity == decl.arity
            })
            .collect()
    }

    /// Resolve a base-list entry written inside `from` following C# lookup
    /// order: nested/enclosing types, enclosing namespaces, the global
    /// namespace, then `using` imports. Qualified names match by suffix.
    fn resolve_base(&mut self, from: TypeKey, base: &CsTypeRef) -> Vec<TypeKey> {
        for allow_scan in [false, true] {
            let found = self.base_candidates(from, base, allow_scan);
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }

    fn base_candidates(
        &mut self,
        from: TypeKey,
        base: &CsTypeRef,
        allow_scan: bool,
    ) -> Vec<TypeKey> {
        let ctx = self.decl(from).clone();
        let ctx_file = from.file;
        let usings = self.files[from.file].types.usings.clone();
        let simple = base.simple_name().to_string();
        let outer_name = base.segments.first().cloned().unwrap_or_default();

        let mut paths = self.candidate_paths(&simple, allow_scan);
        if outer_name != simple {
            paths.extend(self.candidate_paths(&outer_name, allow_scan));
        }
        let mut keys = self.decls_in(&paths);
        keys.extend(
            (0..self.files[ctx_file].types.types.len()).map(|index| TypeKey {
                file: ctx_file,
                index,
            }),
        );
        keys.sort();
        keys.dedup();

        let named: Vec<TypeKey> = keys
            .into_iter()
            .filter(|key| {
                let decl = self.decl(*key);
                decl.name() == simple && decl.arity == base.arity
            })
            .collect();

        if base.segments.len() > 1 {
            return named
                .into_iter()
                .filter(|key| {
                    let decl = self.decl(*key);
                    let mut full: Vec<&str> = if decl.namespace.is_empty() {
                        Vec::new()
                    } else {
                        decl.namespace.split('.').collect()
                    };
                    full.extend(decl.nested_path.iter().map(String::as_str));
                    full.len() >= base.segments.len()
                        && full[full.len() - base.segments.len()..]
                            .iter()
                            .zip(base.segments.iter())
                            .all(|(a, b)| *a == b)
                })
                .collect();
        }

        let rank = |decl: &DeclaredCsType, key: &TypeKey| -> Option<u8> {
            if decl.nested_path.len() > 1 {
                let parent = &decl.nested_path[..decl.nested_path.len() - 1];
                let visible = key.file == ctx_file
                    && decl.namespace == ctx.namespace
                    && ctx.nested_path.starts_with(parent);
                return visible.then_some(0);
            }
            if !decl.namespace.is_empty()
                && (decl.namespace == ctx.namespace
                    || ctx.namespace.starts_with(&format!("{}.", decl.namespace)))
            {
                return Some(1);
            }
            if decl.namespace.is_empty() {
                return Some(2);
            }
            usings.contains(&decl.namespace).then_some(3)
        };
        let ranked: Vec<(u8, TypeKey)> = named
            .into_iter()
            .filter_map(|key| rank(self.decl(key), &key).map(|r| (r, key)))
            .collect();
        let Some(best) = ranked.iter().map(|(r, _)| *r).min() else {
            return Vec::new();
        };
        ranked
            .into_iter()
            .filter(|(r, _)| *r == best)
            .map(|(_, key)| key)
            .collect()
    }

    /// The first of `targets` that `start` derives from or implements
    /// (transitively, through every base-list entry); `None` otherwise.
    pub(super) fn derives_from(
        &mut self,
        start: TypeKey,
        targets: &HashSet<TypeKey>,
    ) -> Option<TypeKey> {
        let target_fragments: HashMap<TypeKey, TypeKey> = targets
            .iter()
            .flat_map(|target| {
                self.fragments(*target)
                    .into_iter()
                    .map(move |fragment| (fragment, *target))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut stack = vec![start];
        let mut visited: HashSet<TypeKey> = HashSet::new();
        while let Some(current) = stack.pop() {
            if visited.len() >= MAX_BASE_WALK || !visited.insert(current) {
                continue;
            }
            for fragment in self.fragments(current) {
                let bases = self.decl(fragment).bases.clone();
                for base in &bases {
                    for resolved in self.resolve_base(fragment, base) {
                        if let Some(target) = target_fragments.get(&resolved) {
                            return Some(*target);
                        }
                        stack.push(resolved);
                    }
                }
            }
        }
        None
    }
}

fn segments_match(segments: &[ManagedClassSegment], decl: &DeclaredCsType) -> bool {
    segments.len() == decl.nested_path.len()
        && segments
            .iter()
            .zip(decl.nested_path.iter())
            .all(|(segment, name)| &segment.name == name)
        && segments.last().map(|s| s.arity).unwrap_or(0) == decl.arity
}

// ─── unity_code_usages ──────────────────────────────────────────────────────

/// How one serialized identity relates to the queried file.
#[derive(Debug, Clone)]
pub(super) struct Relation {
    /// Display name of the type declared in the queried file.
    pub queried: String,
    /// Display name of the concrete declaration when it differs (subtypes).
    pub concrete: Option<String>,
    pub subtype: bool,
    pub legacy: bool,
    /// Current assembly when the serialized `asm` no longer matches it.
    pub assembly_mismatch: Option<String>,
    /// The queried type (or one of its bases in the file) declares the
    /// requested member as a serialized field.
    pub declares_member: bool,
}

pub(super) struct MemberPlan {
    pub member: String,
    /// `member` plus `[FormerlySerializedAs]` names of the declaring field.
    pub names: Vec<String>,
    pub declared_in_file: bool,
}

pub(super) struct UsagePlan {
    pub declared_types: Vec<String>,
    pub file_assembly: Option<String>,
    pub relations: HashMap<ManagedTypeIdentity, Relation>,
    pub member: Option<MemberPlan>,
}

impl UsagePlan {
    pub(super) fn class_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .relations
            .keys()
            .map(|identity| managed_class_key(&identity.class_name))
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }
}

/// Decide which serialized identities belong to the types declared in
/// `script_rel_path`: the types themselves (current, `[MovedFrom]` legacy, or
/// stranded in a previous assembly) and every project subclass / implementer.
pub(super) fn plan_usage_query(
    catalog: &mut TypeCatalog,
    script_rel_path: &str,
    snapshot: &ManagedSnapshot,
    member: Option<&str>,
) -> UsagePlan {
    let file_keys = catalog.types_in_file(script_rel_path);
    let file_assembly = catalog.file_assembly(script_rel_path);
    let declared_types: Vec<String> = file_keys.iter().map(|k| catalog.display(*k)).collect();

    let member_plan = member.map(|member| {
        let mut names = vec![member.to_string()];
        let mut declared_in_file = false;
        for key in &file_keys {
            for field in &catalog.decl(*key).serialized_fields {
                if field.name == member {
                    declared_in_file = true;
                    names.extend(field.former_names.iter().cloned());
                }
            }
        }
        names.sort();
        names.dedup();
        MemberPlan {
            member: member.to_string(),
            names,
            declared_in_file,
        }
    });
    let declares_member = |catalog: &TypeCatalog, key: TypeKey| -> bool {
        match member {
            Some(member) => catalog
                .decl(key)
                .serialized_fields
                .iter()
                .any(|field| field.name == member),
            None => false,
        }
    };

    let mut relations = HashMap::new();
    if file_keys.is_empty() {
        return UsagePlan {
            declared_types,
            file_assembly,
            relations,
            member: member_plan,
        };
    }

    let targets: HashSet<TypeKey> = file_keys.iter().copied().collect();
    let project_assemblies = catalog.project_assemblies();
    // Types in the file that declare the member, plus file types deriving
    // from them, so a member on a base reaches instances of its subclasses.
    let member_owners: HashSet<TypeKey> = file_keys
        .iter()
        .copied()
        .filter(|key| declares_member(&catalog, *key))
        .collect();
    let mut member_types = member_owners.clone();
    if !member_owners.is_empty() {
        for &key in &file_keys {
            if !member_types.contains(&key) && catalog.derives_from(key, &member_owners).is_some() {
                member_types.insert(key);
            }
        }
    }

    for stat in &snapshot.identities {
        let identity = stat.identity.clone();

        // The file's own types: exact, legacy, or stranded by an asm move.
        let mut direct: Option<Relation> = None;
        for &key in &file_keys {
            let names = catalog.names_match(&identity, key);
            let legacy = catalog.legacy_match(&identity, key);
            if !names && !legacy {
                continue;
            }
            let current_asm = catalog.assembly_of(key).map(str::to_string);
            let asm_ok = current_asm.as_deref() == Some(identity.assembly.as_str());
            if names && !asm_ok && !legacy {
                // Same name in another assembly is only "stranded" when no
                // other declaration owns the serialized identity exactly.
                if let Resolution::Exact(other) | Resolution::Legacy(other) =
                    catalog.resolve_identity(&identity)
                {
                    if !targets.contains(&other) {
                        continue;
                    }
                }
            }
            direct = Some(Relation {
                queried: catalog.display(key),
                concrete: None,
                subtype: false,
                legacy: legacy && !(names && asm_ok),
                assembly_mismatch: if names && !asm_ok && !legacy {
                    current_asm
                } else {
                    None
                },
                declares_member: member_types.contains(&key),
            });
            break;
        }
        if let Some(relation) = direct {
            relations.insert(identity, relation);
            continue;
        }

        if !project_assemblies.contains(&identity.assembly) {
            continue;
        }
        let (concrete, legacy, mismatch) = match catalog.resolve_identity(&identity) {
            Resolution::Exact(key) => (key, false, None),
            Resolution::Legacy(key) => (key, true, None),
            Resolution::AssemblyMismatch(key) => {
                let current = catalog.assembly_of(key).map(str::to_string);
                (key, false, current)
            }
            Resolution::Missing => continue,
        };
        if targets.contains(&concrete) {
            continue;
        }
        if let Some(base) = catalog.derives_from(concrete, &targets) {
            let declares =
                !member_types.is_empty() && catalog.derives_from(concrete, &member_types).is_some();
            relations.insert(
                identity,
                Relation {
                    queried: catalog.display(base),
                    concrete: Some(catalog.display(concrete)),
                    subtype: true,
                    legacy,
                    assembly_mismatch: mismatch,
                    declares_member: declares,
                },
            );
        }
    }

    UsagePlan {
        declared_types,
        file_assembly,
        relations,
        member: member_plan,
    }
}

// ─── prefab-instance overrides ──────────────────────────────────────────────

/// Content of one managed reference reached through prefab instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RidType {
    Type(ManagedTypeIdentity),
    /// Unity's null reference (`rid: -2`, or an empty type replacement).
    Null,
}

/// Lookups allowed per resolution (each tries one registry row and one
/// override row); bounds fan-out over prefabs with many instances.
const MAX_RID_RESOLUTION_STEPS: usize = 64;
const MAX_PREFAB_NESTING: usize = 10;
const NULL_RID: i64 = -2;

#[cfg(test)]
/// Type of `rid` on object `file_id` of asset `guid`. The object either holds
/// the registry itself, or comes from a source prefab through one of the
/// asset's prefab instances — then the instance's `managedReferences[rid]`
/// type replacement wins, else the source's value (recursively, through
/// Unity's `(instance ^ sourceFileID) & i64::MAX` nesting rule).
pub(super) fn resolve_rid_type(
    db: &AssetDb,
    guid: &Guid,
    file_id: i64,
    rid: i64,
) -> Result<Option<RidType>, String> {
    let mut budget = MAX_RID_RESOLUTION_STEPS;
    resolve_rid_type_inner(db, guid, file_id, rid, 0, &mut budget)
}

/// Type of `rid` as seen by one override row's prefab instance.
pub(super) fn override_rid_type(
    db: &AssetDb,
    hit: &PrefabOverrideHit,
    rid: i64,
) -> Result<Option<RidType>, String> {
    let mut budget = MAX_RID_RESOLUTION_STEPS;
    instance_rid_type(
        db,
        &hit.src_guid,
        hit.instance_file_id,
        &hit.target_guid,
        hit.target_file_id,
        rid,
        0,
        &mut budget,
    )
}

fn resolve_rid_type_inner(
    db: &AssetDb,
    guid: &Guid,
    file_id: i64,
    rid: i64,
    depth: usize,
    budget: &mut usize,
) -> Result<Option<RidType>, String> {
    if rid == NULL_RID {
        return Ok(Some(RidType::Null));
    }
    if depth > MAX_PREFAB_NESTING || *budget == 0 {
        return Ok(None);
    }
    *budget -= 1;
    if let Some((class_name, namespace, assembly)) =
        db.find_managed_ref_identity(guid, file_id, rid)?
    {
        return Ok(Some(RidType::Type(ManagedTypeIdentity {
            class_name,
            namespace,
            assembly,
        })));
    }
    for (instance, source) in db.get_prefab_instance_sources(guid)? {
        let source_file_id = nested_file_id(instance, file_id);
        if let Some(found) = instance_rid_type(
            db,
            guid,
            instance,
            &source,
            source_file_id,
            rid,
            depth,
            budget,
        )? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn instance_rid_type(
    db: &AssetDb,
    guid: &Guid,
    instance: i64,
    source: &Guid,
    source_file_id: i64,
    rid: i64,
    depth: usize,
    budget: &mut usize,
) -> Result<Option<RidType>, String> {
    if rid == NULL_RID {
        return Ok(Some(RidType::Null));
    }
    let path = format!("managedReferences[{rid}]");
    if let Some(value) =
        db.find_prefab_override_value(guid, instance, source, source_file_id, &path)?
    {
        return Ok(Some(match parse_managed_type_value(&value) {
            Some(identity) => RidType::Type(identity),
            None => RidType::Null,
        }));
    }
    resolve_rid_type_inner(db, source, source_file_id, rid, depth + 1, budget)
}

/// A type replacement belonging to the query, with presentation facts.
#[derive(Debug, Clone)]
pub(super) struct OverrideInstance {
    pub entry: OverrideTypeEntry,
    pub source_path: Option<String>,
    /// Modification that points the owning field at the rid (`main`,
    /// `extra.Array.data[2]`, `managedReferences[R].next`).
    pub via: Option<String>,
}

/// A `managedReferences[R].<member>…` modification on an instance whose
/// effective type belongs to the query.
#[derive(Debug, Clone)]
pub(super) struct OverrideMember {
    pub hit: PrefabOverrideHit,
    pub rid: i64,
    pub identity: ManagedTypeIdentity,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct OverrideFacts {
    pub instances: Vec<OverrideInstance>,
    pub members: Vec<OverrideMember>,
}

/// Resolve the prefab-instance overrides relevant to `plan` (DB lookups only;
/// call under the AssetDb lock, before any file is read).
pub(super) fn collect_override_facts(
    db: &AssetDb,
    plan: &UsagePlan,
    snapshot: &ManagedSnapshot,
) -> Result<OverrideFacts, String> {
    let mut facts = OverrideFacts::default();
    let mut paths: HashMap<Guid, Option<String>> = HashMap::new();
    let mut path_of = |guid: &Guid| -> Result<Option<String>, String> {
        if let Some(found) = paths.get(guid) {
            return Ok(found.clone());
        }
        let found = db.resolve_path_by_guid(guid)?;
        paths.insert(*guid, found.clone());
        Ok(found)
    };

    for entry in snapshot.override_type_entries() {
        if !plan.relations.contains_key(&entry.identity) {
            continue;
        }
        let rid_text = entry.rid.to_string();
        let via = db
            .get_prefab_overrides_for_target(&entry.hit.target_guid, entry.hit.target_file_id)?
            .into_iter()
            .find(|other| {
                other.src_guid == entry.hit.src_guid
                    && other.instance_file_id == entry.hit.instance_file_id
                    && other.value == rid_text
                    && other.property_path != entry.hit.property_path
            })
            .map(|other| other.property_path);
        facts.instances.push(OverrideInstance {
            source_path: path_of(&entry.hit.target_guid)?,
            via,
            entry,
        });
    }

    if let Some(member) = plan.member.as_ref() {
        for hit in &snapshot.managed_overrides {
            let Some((rid, tail)) = parse_managed_reference_path(&hit.property_path) else {
                continue;
            };
            if tail.is_empty() {
                continue;
            }
            let head = crate::unity_yaml::prefab_overrides::property_path_head(tail);
            if !member.names.iter().any(|name| name == head) {
                continue;
            }
            let Some(RidType::Type(identity)) = override_rid_type(db, hit, rid)? else {
                continue;
            };
            let Some(relation) = plan.relations.get(&identity) else {
                continue;
            };
            if member.declared_in_file && !relation.declares_member {
                continue;
            }
            facts.members.push(OverrideMember {
                hit: hit.clone(),
                rid,
                identity,
                source_path: path_of(&hit.target_guid)?,
            });
        }
    }
    Ok(facts)
}

/// Formatted entries for the managed-reference part of `unity_code_usages`.
#[derive(Default)]
pub(super) struct UsageReport {
    /// `(asset path, line, description)` per instance.
    pub instances: Vec<(String, u32, String)>,
    /// `(asset path, line, description)` per serialized member value.
    pub members: Vec<(String, u32, String)>,
    pub asset_count: usize,
    pub type_count: usize,
}

pub(super) fn build_usage_report(
    root: &Path,
    plan: &UsagePlan,
    hits: &[ManagedRefHit],
    overrides: &OverrideFacts,
    snapshot: &ManagedSnapshot,
) -> UsageReport {
    let class_by_guid = snapshot.script_class_by_guid();
    let mut report = UsageReport::default();
    let mut files: HashMap<String, Option<AssetText>> = HashMap::new();
    let mut assets: HashSet<&str> = HashSet::new();
    let mut types: HashSet<ManagedTypeIdentity> = HashSet::new();

    let instance_label = |files: &mut HashMap<String, Option<AssetText>>,
                          path: &str,
                          instance_file_id: i64,
                          source_path: Option<&str>|
     -> String {
        let name = files
            .entry(path.to_string())
            .or_insert_with(|| AssetText::load(&root.join(path)))
            .as_ref()
            .and_then(|asset| asset.instance_name(instance_file_id));
        let source = source_path.unwrap_or("?");
        match name {
            Some(name) => format!("prefab instance '{name}' of {source}"),
            None => format!("prefab instance of {source}"),
        }
    };

    for instance in &overrides.instances {
        let entry = &instance.entry;
        let Some(relation) = plan.relations.get(&entry.identity) else {
            continue;
        };
        assets.insert(entry.hit.path.as_str());
        types.insert(entry.identity.clone());
        let label = instance_label(
            &mut files,
            &entry.hit.path,
            entry.hit.instance_file_id,
            instance.source_path.as_deref(),
        );
        let via = instance
            .via
            .as_deref()
            .map(|via| format!(" via {via}"))
            .unwrap_or_default();
        report.instances.push((
            entry.hit.path.clone(),
            entry.hit.line,
            format!(
                "[SerializeReference override] {} (rid {}) — {label} sets this instance type{via}{}",
                entry.identity.display_name(),
                entry.rid,
                relation_note(relation, &entry.identity)
            ),
        ));
    }

    if let Some(member) = plan.member.as_ref() {
        for found in &overrides.members {
            let label = instance_label(
                &mut files,
                &found.hit.path,
                found.hit.instance_file_id,
                found.source_path.as_deref(),
            );
            let field = found
                .hit
                .property_path
                .split_once("].")
                .map(|(_, tail)| tail)
                .unwrap_or("");
            let head = crate::unity_yaml::prefab_overrides::property_path_head(field);
            let former = if head != member.member {
                format!(" [FormerlySerializedAs \"{head}\"]")
            } else {
                String::new()
            };
            report.members.push((
                found.hit.path.clone(),
                found.hit.line,
                format!(
                    "[SerializeReference override] {}: {} — {} (rid {}), {label}{former}",
                    found.hit.property_path,
                    found.hit.value,
                    found.identity.display_name(),
                    found.rid
                ),
            ));
        }
    }

    for hit in hits {
        let identity = hit_identity(hit);
        let Some(relation) = plan.relations.get(&identity) else {
            continue;
        };
        assets.insert(hit.path.as_str());
        types.insert(identity.clone());
        let asset = files
            .entry(hit.path.clone())
            .or_insert_with(|| AssetText::load(&root.join(&hit.path)));

        let mut desc = format!(
            "[SerializeReference] {} (rid {})",
            identity.display_name(),
            hit.rid
        );
        if let Some(asset) = asset.as_mut() {
            desc.push_str(&asset.host_description(hit.host_file_id, &class_by_guid));
            if let Some(path) = asset.field_path(hit.host_file_id, hit.rid) {
                desc.push_str(&format!(" via {path}"));
            }
        }
        desc.push_str(&relation_note(relation, &identity));
        report.instances.push((hit.path.clone(), hit.line, desc));

        if let (Some(member), Some(asset)) = (plan.member.as_ref(), asset.as_ref()) {
            // Restrict to instances whose type declares / inherits the member
            // when the file declares it; otherwise search every instance.
            if member.declared_in_file && !relation.declares_member {
                continue;
            }
            for (line, key, value) in asset.data_fields(hit.line, &member.names) {
                let former = if key != member.member {
                    format!(" [FormerlySerializedAs \"{key}\"]")
                } else {
                    String::new()
                };
                report.members.push((
                    hit.path.clone(),
                    line,
                    format!(
                        "[SerializeReference field] {key}: {value} — {} (rid {}){former}",
                        identity.display_name(),
                        hit.rid
                    ),
                ));
            }
        }
    }
    report.asset_count = assets.len();
    report.type_count = types.len();
    report
}

fn relation_note(relation: &Relation, identity: &ManagedTypeIdentity) -> String {
    let mut notes = Vec::new();
    if relation.subtype {
        notes.push(format!("subclass/implementer of {}", relation.queried));
    }
    if relation.legacy {
        notes.push(format!(
            "legacy name, resolves via [MovedFrom] on {}",
            relation.concrete.as_deref().unwrap_or(&relation.queried)
        ));
    }
    if let Some(current) = relation.assembly_mismatch.as_deref() {
        notes.push(format!(
            "BROKEN: serialized asm '{}' but {} compiles into '{current}' — Unity reports a missing type unless [MovedFrom(sourceAssembly: \"{}\")] is added",
            identity.assembly,
            relation.concrete.as_deref().unwrap_or(&relation.queried),
            identity.assembly
        ));
    }
    if notes.is_empty() {
        String::new()
    } else {
        format!(" — {}", notes.join("; "))
    }
}

// ─── asset text helpers ─────────────────────────────────────────────────────

/// One YAML asset loaded for presentation: host lookup, rid back-links and
/// registry `data:` member values.
struct AssetText {
    lines: Vec<String>,
    docs: Vec<crate::unity_yaml::YamlDoc>,
    backlinks: HashMap<i64, HashMap<i64, Vec<String>>>,
}

impl AssetText {
    fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let docs = crate::unity_yaml::parse_yaml_docs_str(&text);
        Some(Self {
            lines: text.lines().map(str::to_string).collect(),
            docs,
            backlinks: HashMap::new(),
        })
    }

    /// Instance name of a PrefabInstance document (its `m_Name` modification).
    fn instance_name(&self, instance_file_id: i64) -> Option<String> {
        self.docs
            .iter()
            .find(|d| d.file_id == instance_file_id && d.class_id == 1001)
            .and_then(|d| d.m_name.clone())
            .filter(|name| !name.is_empty())
    }

    fn host_description(&self, host_file_id: i64, class_by_guid: &HashMap<Guid, String>) -> String {
        let Some(host) = self.docs.iter().find(|d| d.file_id == host_file_id) else {
            return String::new();
        };
        let class = host
            .m_script_guid
            .and_then(|guid| class_by_guid.get(&guid).cloned())
            .unwrap_or_else(|| host.type_name.clone());
        let go_name = host.m_game_object_id.and_then(|go| {
            self.docs
                .iter()
                .find(|d| d.file_id == go && d.class_id == 1)
                .and_then(|d| d.m_name.clone())
        });
        match (go_name, host.m_name.as_deref()) {
            (Some(go), _) => format!(" on GameObject '{go}' ({class})"),
            (None, Some(name)) if !name.is_empty() => format!(" in {class} '{name}'"),
            _ => format!(" in {class}"),
        }
    }

    fn field_path(&mut self, host_file_id: i64, rid: i64) -> Option<String> {
        if !self.backlinks.contains_key(&host_file_id) {
            let links = match self.docs.iter().find(|d| d.file_id == host_file_id) {
                Some(doc) => {
                    let end = doc.line_end.min(self.lines.len());
                    let lines: Vec<&str> = self.lines[doc.line_start.min(end)..end]
                        .iter()
                        .map(String::as_str)
                        .collect();
                    rid_backlinks(&lines)
                }
                None => HashMap::new(),
            };
            self.backlinks.insert(host_file_id, links);
        }
        let paths = self.backlinks.get(&host_file_id)?.get(&rid)?;
        let first = paths.first()?.clone();
        Some(if paths.len() > 1 {
            format!("{first} (+{} more)", paths.len() - 1)
        } else {
            first
        })
    }

    fn data_fields(&self, entry_line: u32, names: &[String]) -> Vec<(u32, String, String)> {
        let lines: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        registry_data_fields(&lines, entry_line, names)
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `key: value` split of a (possibly `- `-prefixed) YAML line.
fn split_key_value(trimmed: &str) -> Option<(&str, &str)> {
    let item = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let colon = item.find(':')?;
    let key = item[..colon].trim();
    if key.is_empty() || key.contains(' ') || key.starts_with(['{', '\'', '"']) {
        return None;
    }
    Some((key, item[colon + 1..].trim()))
}

fn flow_rid(value: &str) -> Option<i64> {
    let inner = value.strip_prefix('{')?.strip_suffix('}')?;
    let rest = inner.trim().strip_prefix("rid:")?;
    rest.trim().parse().ok()
}

/// Field paths (`events[1]`, `root`, `managedReferences[7].child`) pointing at
/// each rid inside one host document's lines (header line first). Handles the
/// block (`field:\n  rid: N`), list (`- rid: N`) and flow (`{rid: N}`) forms
/// Unity writes; v1 `id:` links are not attributed.
pub(super) fn rid_backlinks(lines: &[&str]) -> HashMap<i64, Vec<String>> {
    struct Frame {
        indent: usize,
        segment: String,
        element: bool,
        children: usize,
    }
    let render = |stack: &[Frame], tail: Option<&str>| -> String {
        let mut out = String::new();
        let segments = stack
            .iter()
            .map(|f| f.segment.as_str())
            .chain(tail)
            .filter(|s| !s.is_empty());
        for segment in segments {
            if !out.is_empty() && !segment.starts_with('[') {
                out.push('.');
            }
            out.push_str(segment);
        }
        out
    };

    let mut links: HashMap<i64, Vec<String>> = HashMap::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut registry_indent: Option<usize> = None;

    for line in lines.iter().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("---") {
            continue;
        }
        let indent = indent_of(line);
        let dashed = trimmed.starts_with("- ") || trimmed == "-";
        if indent == 0 {
            // `MonoBehaviour:` type header.
            stack.clear();
            continue;
        }

        if let Some(block) = registry_indent {
            if indent <= block {
                registry_indent = None;
                stack.clear();
            } else if indent == block + 2 {
                // Registry entry header (`- rid: N` / v1 `NNNNNNNN:`) or
                // `version:` / `RefIds:`; links inside the entry's data are
                // attributed to `managedReferences[N]`.
                stack.clear();
                let rid = trimmed
                    .strip_prefix("- rid:")
                    .and_then(|v| v.trim().parse::<i64>().ok())
                    .or_else(|| {
                        let key = trimmed.strip_suffix(':')?;
                        key.bytes()
                            .all(|b| b.is_ascii_digit())
                            .then(|| key.parse().ok())?
                    });
                if let Some(rid) = rid {
                    stack.push(Frame {
                        indent,
                        segment: format!("managedReferences[{rid}]"),
                        element: true,
                        children: 0,
                    });
                }
                continue;
            } else if indent == block + 4 && !dashed {
                if trimmed.starts_with("data:") {
                    stack.truncate(1);
                    stack.push(Frame {
                        indent,
                        segment: String::new(),
                        element: false,
                        children: 0,
                    });
                }
                continue;
            }
        } else if indent == 2 && !dashed && trimmed == "references:" {
            registry_indent = Some(indent);
            stack.clear();
            continue;
        }

        if dashed {
            while stack
                .last()
                .map(|f| f.indent > indent || (f.indent == indent && f.element))
                .unwrap_or(false)
            {
                stack.pop();
            }
            let index = match stack.last_mut() {
                Some(parent) if parent.indent == indent && !parent.element => {
                    parent.children += 1;
                    parent.children - 1
                }
                _ => 0,
            };
            stack.push(Frame {
                indent,
                segment: format!("[{index}]"),
                element: true,
                children: 0,
            });
        } else {
            while stack.last().map(|f| f.indent >= indent).unwrap_or(false) {
                stack.pop();
            }
        }

        let item = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(rid) = flow_rid(item) {
            links.entry(rid).or_default().push(render(&stack, None));
            continue;
        }
        let Some((key, value)) = split_key_value(trimmed) else {
            continue;
        };
        let key_indent = if dashed { indent + 2 } else { indent };
        if key == "rid" {
            if let Ok(rid) = value.parse::<i64>() {
                links.entry(rid).or_default().push(render(&stack, None));
            }
            continue;
        }
        if let Some(rid) = flow_rid(value) {
            links
                .entry(rid)
                .or_default()
                .push(render(&stack, Some(key)));
            continue;
        }
        if value.is_empty() {
            stack.push(Frame {
                indent: key_indent,
                segment: key.to_string(),
                element: false,
                children: 0,
            });
        }
    }
    for paths in links.values_mut() {
        paths.retain(|p| !p.is_empty());
    }
    links.retain(|_, paths| !paths.is_empty());
    links
}

/// First-level `data:` fields named in `names` of the registry entry that
/// starts at 1-based `entry_line`: `(line, key, value preview)`.
pub(super) fn registry_data_fields(
    lines: &[&str],
    entry_line: u32,
    names: &[String],
) -> Vec<(u32, String, String)> {
    let mut out = Vec::new();
    let start = entry_line.saturating_sub(1) as usize;
    let Some(entry) = lines.get(start) else {
        return out;
    };
    let entry_indent = indent_of(entry);
    let mut data_indent: Option<usize> = None;
    for (offset, line) in lines.iter().enumerate().skip(start + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = indent_of(line);
        if indent <= entry_indent {
            break;
        }
        if indent == entry_indent + 2 {
            data_indent = trimmed.starts_with("data:").then_some(indent);
            continue;
        }
        let Some(data) = data_indent else {
            continue;
        };
        if indent != data + 2 || trimmed.starts_with('-') {
            continue;
        }
        let Some((key, value)) = split_key_value(trimmed) else {
            continue;
        };
        if names.iter().any(|name| name == key) {
            let mut preview = if value.is_empty() {
                "{…}".to_string()
            } else {
                value.to_string()
            };
            if preview.chars().count() > 60 {
                preview = preview.chars().take(60).collect::<String>() + "…";
            }
            out.push((offset as u32 + 1, key.to_string(), preview));
        }
    }
    out
}

// ─── code_diagnostics: serialized types that no longer resolve ─────────────

#[derive(Debug, Clone)]
pub(super) struct ManagedTypeIssue {
    pub identity: ManagedTypeIdentity,
    pub instance_count: u64,
    pub asset_count: u64,
    /// `Some((declared type, current assembly))` for an assembly move;
    /// `None` when no declaration exists at all.
    pub moved_to: Option<(String, String)>,
    pub samples: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ManagedTypeReport {
    pub checked_types: usize,
    pub checked_instances: u64,
    pub skipped_external_types: usize,
    pub issues: Vec<ManagedTypeIssue>,
}

/// Verify every serialized `[SerializeReference]` type identity from a
/// project-owned assembly still resolves to a declaration. With
/// `scope_file`, only identities in that file's assembly or naming one of its
/// types are checked. `Ok(None)` when the asset index is unavailable.
pub(super) fn check_managed_reference_types(
    asset_db: &SharedAssetDb,
    root: &Path,
    scope_file: Option<&str>,
) -> Result<Option<ManagedTypeReport>, String> {
    let snapshot = {
        let guard = asset_db
            .lock()
            .map_err(|error| format!("Failed to lock AssetDb: {error}"))?;
        let Some(db) = guard.as_ref() else {
            return Ok(None);
        };
        ManagedSnapshot::load(db)?
    };

    let mut catalog = TypeCatalog::new(root, &snapshot);
    let project_assemblies = catalog.project_assemblies();
    let (scope_assembly, scope_keys) = match scope_file {
        Some(path) => {
            let canonical = canonical_script_path(&snapshot, path);
            (
                catalog.file_assembly(&canonical),
                catalog.types_in_file(&canonical),
            )
        }
        None => (None, Vec::new()),
    };

    let mut report = ManagedTypeReport::default();
    for stat in &snapshot.identities {
        let identity = stat.identity.clone();
        if !project_assemblies.contains(&identity.assembly) {
            report.skipped_external_types += 1;
            continue;
        }
        if scope_file.is_some() {
            let in_assembly = scope_assembly.as_deref() == Some(identity.assembly.as_str());
            let names_file_type = scope_keys.iter().any(|key| {
                catalog.names_match(&identity, *key) || catalog.legacy_match(&identity, *key)
            });
            if !in_assembly && !names_file_type {
                continue;
            }
        }
        report.checked_types += 1;
        report.checked_instances += stat.instance_count;
        let moved_to = match catalog.resolve_identity(&identity) {
            Resolution::Exact(_) | Resolution::Legacy(_) => continue,
            Resolution::AssemblyMismatch(key) => Some((
                catalog.display(key),
                catalog.assembly_of(key).unwrap_or("?").to_string(),
            )),
            Resolution::Missing => None,
        };
        report.issues.push(ManagedTypeIssue {
            identity,
            instance_count: stat.instance_count,
            asset_count: stat.asset_count,
            moved_to,
            samples: Vec::new(),
        });
    }

    if !report.issues.is_empty() {
        let keys: Vec<String> = report
            .issues
            .iter()
            .map(|issue| managed_class_key(&issue.identity.class_name))
            .collect();
        let hits = {
            let guard = asset_db
                .lock()
                .map_err(|error| format!("Failed to lock AssetDb: {error}"))?;
            match guard.as_ref() {
                Some(db) => db.get_managed_refs_by_class_keys(&keys)?,
                None => Vec::new(),
            }
        };
        let override_entries = snapshot.override_type_entries();
        for issue in &mut report.issues {
            issue.samples = hits
                .iter()
                .filter(|hit| hit_identity(hit) == issue.identity)
                .map(|hit| (hit.path.clone(), hit.line))
                .chain(
                    override_entries
                        .iter()
                        .filter(|entry| entry.identity == issue.identity)
                        .map(|entry| (entry.hit.path.clone(), entry.hit.line)),
                )
                .take(MAX_SAMPLES_PER_TYPE)
                .collect();
        }
    }
    Ok(Some(report))
}

pub(super) fn format_managed_type_section(report: &ManagedTypeReport, max_issues: usize) -> String {
    if report.checked_types == 0 && report.issues.is_empty() {
        return String::new();
    }
    if report.issues.is_empty() {
        return format!(
            "\n\nSerializeReference types: all {} serialized project type{} resolve ({} instance{}).",
            report.checked_types,
            if report.checked_types == 1 { "" } else { "s" },
            report.checked_instances,
            if report.checked_instances == 1 { "" } else { "s" },
        );
    }
    let shown = report.issues.len().min(max_issues);
    let mut out = format!(
        "\n\nSerializeReference types: {} unresolved of {} checked{} — Unity loads these instances as missing types\n",
        report.issues.len(),
        report.checked_types,
        if shown < report.issues.len() {
            format!(", showing first {shown}")
        } else {
            String::new()
        }
    );
    for issue in report.issues.iter().take(max_issues) {
        let reason = match &issue.moved_to {
            Some((declared, current)) => format!(
                "{declared} now compiles into '{current}', not '{}' — add [MovedFrom(sourceAssembly: \"{}\")] or move it back",
                issue.identity.assembly, issue.identity.assembly
            ),
            None => "no such type in code — restore it or add [MovedFrom] to its replacement".to_string(),
        };
        out.push_str(&format!(
            "  {} [asm {}]: {reason} ({} instance{} in {} asset{})\n",
            issue.identity.display_name(),
            issue.identity.assembly,
            issue.instance_count,
            if issue.instance_count == 1 { "" } else { "s" },
            issue.asset_count,
            if issue.asset_count == 1 { "" } else { "s" },
        ));
        for (path, line) in &issue.samples {
            out.push_str(&format!("    {path}:{line}\n"));
        }
    }
    out
}

/// The index's canonical path for a script given any-cased input.
pub(super) fn canonical_script_path(snapshot: &ManagedSnapshot, rel_path: &str) -> String {
    let normalized = rel_path.replace('\\', "/");
    snapshot
        .scripts
        .iter()
        .find(|entry| entry.path.eq_ignore_ascii_case(&normalized))
        .map(|entry| entry.path.clone())
        .unwrap_or(normalized)
}
