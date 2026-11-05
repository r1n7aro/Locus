//! A large, rarely changing part of a merge job (snapshot, catalog, schemas).
//! Clones share one allocation, so a loaded job costs no deep copy. Reading
//! goes through `Deref`; a mutation copies on write and forgets where the part
//! was stored, so the journal writes it again (see `journal`).
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub struct Shared<T> {
    value: Arc<T>,
    /// The job directory and content hash of the part file holding exactly
    /// this value. Shared between clones until one of them mutates.
    stored: Arc<OnceLock<(PathBuf, String)>>,
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self { value: Arc::new(value), stored: Arc::new(OnceLock::new()) }
    }

    pub(super) fn from_arc(value: Arc<T>, dir: &Path, hash: String) -> Self {
        let stored = OnceLock::new();
        let _ = stored.set((dir.to_path_buf(), hash));
        Self { value, stored: Arc::new(stored) }
    }

    pub(super) fn arc(&self) -> &Arc<T> {
        &self.value
    }

    /// The content hash when this exact value is already stored in `dir`.
    pub(super) fn stored_in(&self, dir: &Path) -> Option<&str> {
        self.stored.get().filter(|(stored, _)| stored == dir).map(|(_, hash)| hash.as_str())
    }

    pub(super) fn mark_stored(&self, dir: &Path, hash: String) {
        let _ = self.stored.set((dir.to_path_buf(), hash));
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self { value: self.value.clone(), stored: self.stored.clone() }
    }
}

impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.stored = Arc::new(OnceLock::new());
        Arc::make_mut(&mut self.value)
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value) || self.value == other.value
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::new)
    }
}

impl<'a, T> IntoIterator for &'a Shared<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&*self.value).into_iter()
    }
}

impl<T: Clone> Shared<T> {
    /// An owned copy of the value, for a caller that mutates it freely.
    pub fn to_inner(&self) -> T {
        (*self.value).clone()
    }
}
