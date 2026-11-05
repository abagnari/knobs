//! The data a node receives while it is evaluated.

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Context;

use crate::{ConstantInputType, GenericStore};

/// Everything a node is given for one evaluation.
#[derive(Debug, Clone)]
pub struct RuntimeState<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// Type checked string keyed store serving two purposes.
    ///
    /// The host seeds it with resources a node may need but does not own.
    /// Nodes in turn use it as scratch space that survives across polls: channel ends,
    /// readback buffers, dimensions carried over from the call that started the work.
    /// Lookups are checked against the requested type, so a miss is reported either as
    /// "no such key" or as "key present but of another type", and the first of those
    /// may be used as the normal signal that a node is on its first pass.
    ///
    /// It is cleared whenever the node is invalidated.
    ///
    /// Can be pre-seeded at each evaluation round by calling
    /// [`Graph::add_default_compute_entry`](crate::Graph::add_default_compute_entry).
    pub user_state: GenericStore,
    /// Values available on the node's input ports, keyed by port name.
    ///
    /// An entry is present for every input that is either connected to an
    /// upstream output, configured with a constant (converted into a `D`), or both.
    /// Optional connection-only inputs that are unconnected have no entry.
    pub inputs_map: InputsMap<D>,
}

impl<D> Default for RuntimeState<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    fn default() -> Self {
        Self {
            inputs_map: InputsMap::default(),
            user_state: GenericStore::default(),
        }
    }
}

impl<D> RuntimeState<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    pub(crate) fn clear(&mut self) {
        self.user_state.clear();
        self.inputs_map.clear();
    }
}

/// A collection of named inputs, keyed by name.
///
/// Wraps the underlying map so that callers cannot insert into it or index
/// into it directly. Lookups return a `Result` instead of panicking on a missing key.
///
/// Values are stored behind an [`Arc`] so that a caller can either borrow one
/// for the duration of the lookup ([`InputsMap::get`]) or take a shared owner of
/// it that outlives the collection ([`InputsMap::get_shared`]).
///
/// ```
/// # use std::collections::HashMap;
/// # use std::convert::Infallible;
/// # use std::sync::Arc;
/// # use knobs::ConstantInputType;
/// # use knobs::graph::node::state::InputsMap;
/// #[derive(Debug, Clone, PartialEq)]
/// struct Height(u32);
///
/// impl TryFrom<&Height> for String {
///     type Error = Infallible;
///
///     fn try_from(Height(value): &Height) -> Result<Self, Self::Error> {
///         Ok(value.to_string())
///     }
/// }
///
/// impl From<&ConstantInputType> for Height {
///     fn from(_: &ConstantInputType) -> Self {
///         Height(0)
///     }
/// }
///
/// let inputs = InputsMap::new(HashMap::from([
///     ("Terrain".to_owned(), Arc::new(Height(42))),
/// ]));
///
/// assert_eq!(inputs.get("Terrain").unwrap(), &Height(42));
/// assert!(inputs.get("Missing").is_err());
/// ```
#[derive(PartialEq, Debug, Clone)]
pub struct InputsMap<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    map: HashMap<String, Arc<D>>,
}

impl<D> InputsMap<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// Takes ownership of `map` and exposes it as a read-only collection.
    pub fn new(map: HashMap<String, Arc<D>>) -> Self {
        Self { map }
    }

    /// Builds a collection by converting each constant into a value.
    pub fn from_constants(constants: HashMap<String, ConstantInputType>) -> Self {
        Self::new(
            constants
                .iter()
                .map(|(name, constant)| (name.clone(), Arc::new(D::from(constant))))
                .collect(),
        )
    }

    /// Borrows the value stored under `name`.
    ///
    /// The returned reference is tied to `self`, so it cannot outlive the
    /// collection. Use [`InputsMap::get_shared`] when the value needs to live
    /// longer.
    ///
    /// # Errors
    ///
    /// Returns an error if no value is stored under `name`.
    pub fn get(&self, name: &str) -> anyhow::Result<&D> {
        self.map
            .get(name)
            .map(|value| &**value)
            .with_context(|| format!("Missing input `{name}`"))
    }

    /// Returns a shared owner of the value stored under `name`.
    ///
    /// Clones the [`Arc`], not the value, so the result is independent of the
    /// collection's lifetime.
    ///
    /// # Errors
    ///
    /// Returns an error if no value is stored under `name`.
    pub fn get_shared(&self, name: &str) -> anyhow::Result<Arc<D>> {
        self.map
            .get(name)
            .cloned()
            .with_context(|| format!("Missing input `{name}`"))
    }

    /// Reports whether a value is stored under `name`.
    ///
    /// Useful for optional inputs, where a missing key is not an error.
    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    /// Iterates over the names of every stored input, in arbitrary order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// Returns the number of stored inputs.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Reports whether no inputs are stored.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Clears the underlying map.
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

impl<D> FromIterator<(String, Arc<D>)> for InputsMap<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    fn from_iter<I: IntoIterator<Item = (String, Arc<D>)>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<D> Default for InputsMap<D>
where
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    fn default() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERRAIN_KEY: &str = "Terrain";
    const MESH_KEY: &str = "Mesh";
    const MISSING_KEY: &str = "Absent";
    const TERRAIN_SAMPLE: f32 = 1.5;
    const MESH_SAMPLE: f32 = 2.5;
    const DEFAULT_FLOAT_SAMPLE: f32 = 0.0;
    const DEFAULT_INTEGER_SAMPLE: f32 = 0.0;
    const EXPECTED_STRONG_COUNT: usize = 2;
    const EXPECTED_LEN: usize = 2;

    #[derive(PartialEq, Clone, Debug)]
    enum TestValue {
        Number(f32),
        Text(String),
        None,
    }

    impl From<&ConstantInputType> for TestValue {
        fn from(constant: &ConstantInputType) -> Self {
            match constant {
                ConstantInputType::Float { value, .. } => Self::Number(*value),
                ConstantInputType::Integer { value, .. } => Self::Number(*value as f32),
                ConstantInputType::Path(path) => Self::Text(path.clone()),
                ConstantInputType::String(string) => Self::Text(string.clone()),
                ConstantInputType::Selection { options, selected } => {
                    Self::Text(options[*selected].clone())
                }
                ConstantInputType::OptionalFloat { .. } => TestValue::None,
                ConstantInputType::OptionalInteger { .. } => TestValue::None,
                ConstantInputType::RandomizableUnsignedInteger { .. } => TestValue::None,
                ConstantInputType::Boolean { .. } => TestValue::None,
            }
        }
    }

    impl TryInto<String> for &TestValue {
        type Error = ();

        fn try_into(self) -> Result<String, Self::Error> {
            match self {
                TestValue::Number(number) => Ok(number.to_string()),
                TestValue::Text(text) => Ok(text.to_string()),
                TestValue::None => Err(()),
            }
        }
    }

    fn value(sample: f32) -> Arc<TestValue> {
        Arc::new(TestValue::Number(sample))
    }

    fn populated_inputs() -> InputsMap<TestValue> {
        InputsMap::new(HashMap::from([
            (TERRAIN_KEY.to_owned(), value(TERRAIN_SAMPLE)),
            (MESH_KEY.to_owned(), value(MESH_SAMPLE)),
        ]))
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn get_existing_key_returns_value() {
        let inputs = populated_inputs();

        let result = inputs.get(TERRAIN_KEY).unwrap();

        assert_eq!(result, &TestValue::Number(TERRAIN_SAMPLE));
    }

    #[test]
    fn get_missing_key_returns_error() {
        let inputs = populated_inputs();

        let result = inputs.get(MISSING_KEY);

        assert!(result.is_err());
    }

    #[test]
    fn get_shared_existing_key_returns_additional_owner() {
        let inputs = populated_inputs();

        let result = inputs.get_shared(TERRAIN_KEY).unwrap();

        assert_eq!(Arc::strong_count(&result), EXPECTED_STRONG_COUNT);
    }

    #[test]
    fn get_shared_missing_key_returns_error() {
        let inputs = populated_inputs();

        let result = inputs.get_shared(MISSING_KEY);

        assert!(result.is_err());
    }

    #[test]
    fn get_shared_outlives_collection() {
        let result = {
            let inputs = populated_inputs();
            inputs.get_shared(TERRAIN_KEY).unwrap()
        };

        assert_eq!(*result, TestValue::Number(TERRAIN_SAMPLE));
    }

    #[test]
    fn from_constants_converts_float_entry() {
        let constants =
            HashMap::from([(TERRAIN_KEY.to_owned(), ConstantInputType::default_float())]);

        let result: InputsMap<TestValue> = InputsMap::from_constants(constants);

        assert_eq!(
            result.get(TERRAIN_KEY).unwrap(),
            &TestValue::Number(DEFAULT_FLOAT_SAMPLE)
        );
    }

    #[test]
    fn from_constants_converts_integer_entry() {
        let constants =
            HashMap::from([(TERRAIN_KEY.to_owned(), ConstantInputType::default_integer())]);

        let result: InputsMap<TestValue> = InputsMap::from_constants(constants);

        assert_eq!(
            result.get(TERRAIN_KEY).unwrap(),
            &TestValue::Number(DEFAULT_INTEGER_SAMPLE)
        );
    }

    #[test]
    fn from_constants_converts_path_entry() {
        let constants =
            HashMap::from([(TERRAIN_KEY.to_owned(), ConstantInputType::default_path())]);

        let result: InputsMap<TestValue> = InputsMap::from_constants(constants);

        assert_eq!(
            result.get(TERRAIN_KEY).unwrap(),
            &TestValue::Text(String::new())
        );
    }

    #[test]
    fn from_constants_empty_map_returns_empty_collection() {
        let result: InputsMap<TestValue> = InputsMap::from_constants(HashMap::new());

        assert!(result.is_empty());
    }

    #[test]
    fn contains_existing_key_returns_true() {
        let inputs = populated_inputs();

        let result = inputs.contains(TERRAIN_KEY);

        assert!(result);
    }

    #[test]
    fn contains_missing_key_returns_false() {
        let inputs = populated_inputs();

        let result = inputs.contains(MISSING_KEY);

        assert!(!result);
    }

    #[test]
    fn names_returns_every_stored_key() {
        let inputs = populated_inputs();

        let mut result: Vec<&str> = inputs.names().collect();
        result.sort_unstable();

        assert_eq!(result, vec![MESH_KEY, TERRAIN_KEY]);
    }

    #[test]
    fn len_populated_collection_returns_entry_count() {
        let inputs = populated_inputs();

        let result = inputs.len();

        assert_eq!(result, EXPECTED_LEN);
    }

    #[test]
    fn is_empty_populated_collection_returns_false() {
        let inputs = populated_inputs();

        let result = inputs.is_empty();

        assert!(!result);
    }

    #[test]
    fn is_empty_empty_collection_returns_true() {
        let inputs: InputsMap<TestValue> = InputsMap::new(HashMap::new());

        let result = inputs.is_empty();

        assert!(result);
    }

    #[test]
    fn clone_produces_equal_collection() {
        let inputs = populated_inputs();

        let result = inputs.clone();

        assert_eq!(result, inputs);
    }

    #[test]
    fn from_iter_collects_entries() {
        let entries = vec![(TERRAIN_KEY.to_owned(), value(TERRAIN_SAMPLE))];

        let result: InputsMap<TestValue> = entries.into_iter().collect();

        assert!(result.contains(TERRAIN_KEY));
    }

    #[test]
    fn inputs_map_is_send_and_sync() {
        assert_send_sync::<InputsMap<TestValue>>();
    }
}
