use std::any::Any;
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};

trait ClonableAny: Any + Debug + Send + Sync {
    fn clone_box(&self) -> Box<dyn ClonableAny>;
    fn as_any_ref(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T> ClonableAny for T
where
    T: Any + Clone + Debug + Send + Sync,
{
    fn clone_box(&self) -> Box<dyn ClonableAny> {
        Box::new(self.clone())
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Clone for Box<dyn ClonableAny> {
    fn clone(&self) -> Self {
        self.as_ref().clone_box()
    }
}

/// A string keyed map that holds values of any type and checks the type on
/// every lookup.
///
/// Nodes reach it as [`RuntimeState::user_state`](crate::RuntimeState::user_state).
/// Values must be `Clone + Debug + Send + Sync` because the store is cloned onto
/// worker threads together with the rest of the runtime state.
#[derive(Debug, Clone, Default)]
pub struct GenericStore {
    entries: HashMap<String, Box<dyn ClonableAny>>,
}

/// Why a [`GenericStore`] lookup failed.
#[derive(Debug)]
pub enum GetError {
    /// No value is stored under the key.
    NotFound,
    /// A value is stored under the key, but it has a different type than the
    /// one requested.
    InvalidType,
}

impl Display for GetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for GetError {}

impl GenericStore {
    /// Stores `value` under `key`, replacing any previous value regardless of
    /// its type.
    pub fn insert<T>(&mut self, key: impl Into<String>, value: T)
    where
        T: Any + Clone + Debug + Send + Sync,
    {
        self.entries.insert(key.into(), Box::new(value));
    }

    /// Borrows the value stored under `key` as a `T`.
    ///
    /// # Errors
    ///
    /// [`GetError::NotFound`] when the key is absent, and
    /// [`GetError::InvalidType`] when the stored value is not a `T`.
    pub fn get<T: Any>(&self, key: &str) -> Result<&T, GetError> {
        self.entries
            .get(key)
            .ok_or(GetError::NotFound)?
            .as_ref()
            .as_any_ref()
            .downcast_ref::<T>()
            .ok_or(GetError::InvalidType)
    }

    /// Removes the value stored under `key` and returns it as a `T`.
    ///
    /// The value is only removed when it has the requested type; on
    /// [`GetError::InvalidType`] the store is left unchanged.
    ///
    /// # Errors
    ///
    /// [`GetError::NotFound`] when the key is absent, and
    /// [`GetError::InvalidType`] when the stored value is not a `T`.
    pub fn remove<T: Any>(&mut self, key: &str) -> Result<T, GetError> {
        let stored_type_matches = self
            .entries
            .get(key)
            .ok_or(GetError::NotFound)?
            .as_ref()
            .as_any_ref()
            .is::<T>();

        if !stored_type_matches {
            return Err(GetError::InvalidType);
        }

        let entry: Box<dyn Any> = self.entries.remove(key).ok_or(GetError::NotFound)?;

        entry
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| GetError::InvalidType)
    }

    /// Mutably borrows the value stored under `key` as a `T`.
    ///
    /// # Errors
    ///
    /// [`GetError::NotFound`] when the key is absent, and
    /// [`GetError::InvalidType`] when the stored value is not a `T`.
    pub fn get_mut<T: Any>(&mut self, key: &str) -> Result<&mut T, GetError> {
        self.entries
            .get_mut(key)
            .ok_or(GetError::NotFound)?
            .as_mut()
            .as_any_mut()
            .downcast_mut::<T>()
            .ok_or(GetError::InvalidType)
    }

    /// Reports whether any value, of any type, is stored under `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Moves every entry of `other` into this store. Entries of `other` replace
    /// entries with the same key.
    pub fn extend(&mut self, other: GenericStore) {
        self.entries.extend(other.entries);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME_KEY: &str = "name";
    const COUNT_KEY: &str = "count";
    const FLAG_KEY: &str = "flag";
    const MISSING_KEY: &str = "missing";
    const NAME: &str = "Alice";
    const COUNT: u32 = 10;
    const OTHER_COUNT: u32 = 20;

    fn store_with_count() -> GenericStore {
        let mut store = GenericStore::default();
        store.insert(COUNT_KEY, COUNT);
        store
    }

    #[test]
    fn get_after_insert_returns_value() {
        let mut store = GenericStore::default();
        store.insert(NAME_KEY, String::from(NAME));

        let result = store.get::<String>(NAME_KEY);

        assert_eq!(result.unwrap(), NAME);
    }

    #[test]
    fn get_with_wrong_type_returns_invalid_type() {
        let store = store_with_count();

        let result = store.get::<String>(COUNT_KEY);

        assert!(matches!(result, Err(GetError::InvalidType)));
    }

    #[test]
    fn get_missing_key_returns_not_found() {
        let store = GenericStore::default();

        let result = store.get::<String>(MISSING_KEY);

        assert!(matches!(result, Err(GetError::NotFound)));
    }

    #[test]
    fn get_mut_change_is_visible_through_get() {
        let mut store = store_with_count();

        *store.get_mut::<u32>(COUNT_KEY).unwrap() = OTHER_COUNT;

        assert_eq!(*store.get::<u32>(COUNT_KEY).unwrap(), OTHER_COUNT);
    }

    #[test]
    fn contains_key_missing_key_returns_false() {
        let store = GenericStore::default();

        let result = store.contains_key(MISSING_KEY);

        assert!(!result);
    }

    #[test]
    fn contains_key_inserted_key_returns_true() {
        let store = store_with_count();

        let result = store.contains_key(COUNT_KEY);

        assert!(result);
    }

    #[test]
    fn clone_copies_every_entry() {
        let mut store = store_with_count();
        store.insert(NAME_KEY, String::from(NAME));

        let cloned = store.clone();

        assert_eq!(cloned.get::<String>(NAME_KEY).unwrap(), NAME);
        assert_eq!(*cloned.get::<u32>(COUNT_KEY).unwrap(), COUNT);
    }

    #[test]
    fn get_with_mixed_value_types_returns_each_value() {
        let mut store = store_with_count();
        store.insert(NAME_KEY, String::from(NAME));
        store.insert(FLAG_KEY, true);

        let name = store.get::<String>(NAME_KEY);
        let count = store.get::<u32>(COUNT_KEY);
        let flag = store.get::<bool>(FLAG_KEY);

        assert_eq!(name.unwrap(), NAME);
        assert_eq!(*count.unwrap(), COUNT);
        assert!(*flag.unwrap());
    }

    #[test]
    fn insert_existing_key_with_new_type_replaces_the_type() {
        let mut store = store_with_count();

        store.insert(COUNT_KEY, String::from(NAME));

        assert!(matches!(store.get::<u32>(COUNT_KEY), Err(GetError::InvalidType)));
        assert_eq!(store.get::<String>(COUNT_KEY).unwrap(), NAME);
    }

    #[test]
    fn remove_existing_key_returns_value_and_removes_it() {
        let mut store = store_with_count();

        let result = store.remove::<u32>(COUNT_KEY);

        assert_eq!(result.unwrap(), COUNT);
        assert!(!store.contains_key(COUNT_KEY));
    }

    #[test]
    fn remove_with_wrong_type_returns_invalid_type_and_keeps_value() {
        let mut store = store_with_count();

        let result = store.remove::<String>(COUNT_KEY);

        assert!(matches!(result, Err(GetError::InvalidType)));
        assert_eq!(*store.get::<u32>(COUNT_KEY).unwrap(), COUNT);
    }

    #[test]
    fn remove_missing_key_returns_not_found() {
        let mut store = GenericStore::default();

        let result = store.remove::<u32>(MISSING_KEY);

        assert!(matches!(result, Err(GetError::NotFound)));
    }

    #[test]
    fn extend_adds_entries_and_replaces_same_keys() {
        let mut store = store_with_count();
        let mut other = GenericStore::default();
        other.insert(COUNT_KEY, OTHER_COUNT);
        other.insert(NAME_KEY, String::from(NAME));

        store.extend(other);

        assert_eq!(*store.get::<u32>(COUNT_KEY).unwrap(), OTHER_COUNT);
        assert_eq!(store.get::<String>(NAME_KEY).unwrap(), NAME);
    }
}
