//! Port declarations, input and output kinds, and constant values.

use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::sync::Arc;

use egui::Color32;
use rand::Rng;

use crate::graph::connection::DragConnection;
use crate::graph::node::NodeId;
use crate::graph::snapshot::{ConstantValue, PortSnapshot, SnapshotNodeId, SnapshotPortId};
use crate::helpers::random_in_range;

/// Identifies one port of one node.
#[derive(Default, Copy, Clone, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub struct PortId {
    node_id: NodeId,
    id: u32,
}

impl Debug for PortId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "PortId({:?}:{})", self.node_id, self.id)
    }
}

impl PortId {
    /// The node the port belongs to.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub(crate) fn index(&self) -> u32 {
        self.id
    }
}

pub(crate) fn port_id(node_id: NodeId, id: u32) -> PortId {
    PortId { node_id, id }
}

#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub(crate) enum PortDirection {
    Input,
    Output,
}

/// Names the port of a node that gives off [`EnablerSignal`]s.
#[derive(PartialEq, Clone, Copy, Debug, Hash, Eq)]
pub struct EnablerId(usize);

/// Creates a [`EnablerId`] with the provided `id` attribute.
pub fn enabler_id(id: usize) -> EnablerId {
    EnablerId(id)
}

/// A condition given off by the port declaring the wrapped [`EnablerId`].
///
/// The signals a port gives off follow from its shape. A signal the port cannot
/// give off reads as off, and a constant only signals while it is the value of
/// its port, that is while the port is unconnected.
#[derive(PartialEq, Clone, Copy, Debug, Hash, Eq)]
pub enum EnablerSignal {
    /// The port has a connection.
    Present(EnablerId),
    /// The Boolean constant of the port is ticked.
    Checked(EnablerId),
    /// The Selection constant of the port has the option at this index selected.
    Selected(EnablerId, usize),
}

impl EnablerSignal {
    /// The port giving off the signal.
    pub fn id(&self) -> EnablerId {
        match self {
            Self::Present(id) | Self::Checked(id) | Self::Selected(id, _) => *id,
        }
    }
}

/// Decides, from the signals of its node, whether the constant of a port is editable.
#[derive(PartialEq, Clone, Debug)]
pub enum EnablerSpy {
    /// Holds while the state of `when` equals `enabled`.
    When {
        /// The signal listened to.
        when: EnablerSignal,
        /// The state of the signal that lets this spy hold.
        enabled: bool,
    },
    /// Holds while every spy holds.
    All(Vec<EnablerSpy>),
    /// Holds while at least one spy holds.
    Any(Vec<EnablerSpy>),
}

impl EnablerSpy {
    /// Holds while `signal` is on.
    pub fn when(signal: EnablerSignal) -> Self {
        Self::When {
            when: signal,
            enabled: true,
        }
    }

    /// Holds while `signal` is off.
    pub fn unless(signal: EnablerSignal) -> Self {
        Self::When {
            when: signal,
            enabled: false,
        }
    }

    /// Holds while both `self` and `other` hold. Chained calls stay flat.
    pub fn and(self, other: Self) -> Self {
        Self::All([self.into_all(), other.into_all()].concat())
    }

    /// Holds while `self` or `other` holds. Chained calls stay flat.
    pub fn or(self, other: Self) -> Self {
        Self::Any([self.into_any(), other.into_any()].concat())
    }

    /// Whether the spy holds, reading each signal state from `resolve`.
    pub fn evaluate(&self, resolve: &impl Fn(EnablerSignal) -> bool) -> bool {
        match self {
            Self::When { when, enabled } => resolve(*when) == *enabled,
            Self::All(spies) => spies.iter().all(|spy| spy.evaluate(resolve)),
            Self::Any(spies) => spies.iter().any(|spy| spy.evaluate(resolve)),
        }
    }

    pub(crate) fn signals(&self) -> Vec<EnablerSignal> {
        match self {
            Self::When { when, .. } => vec![*when],
            Self::All(spies) | Self::Any(spies) => spies.iter().flat_map(Self::signals).collect(),
        }
    }

    fn into_all(self) -> Vec<Self> {
        match self {
            Self::All(spies) => spies,
            spy => vec![spy],
        }
    }

    fn into_any(self) -> Vec<Self> {
        match self {
            Self::Any(spies) => spies,
            spy => vec![spy],
        }
    }

    fn checked_or_present(id: EnablerId) -> Self {
        Self::when(EnablerSignal::Checked(id)).or(Self::when(EnablerSignal::Present(id)))
    }
}

/// The ways an output port can expose its value.
#[derive(PartialEq, Clone, Debug)]
pub enum PortOutputType {
    /// Feeds exclusively an input port.
    ConnectionOnly,
    /// Shows the output value as an inline label.
    ConstantOnly {
        /// Text shown as the port's result. The graph fills it in from the
        /// computed value and replaces anything passed in, so pass `None`.
        text_value: Option<String>
    },
    /// Feeds an input port and shows the output value as an inline label.
    ConnectionAndConstant {
        /// Text shown as the port's result. The graph fills it in from the
        /// computed value and replaces anything passed in, so pass `None`.
        text_value: Option<String>
    },
}

/// The ways an input port can receive its value.
#[derive(PartialEq, Clone, Debug)]
pub enum PortInputType {
    /// Fed exclusively by a connection from an upstream output port.
    ConnectionOnly {
        /// The connected output port, or `None` while the port is unconnected.
        data_source: Option<PortId>,
        /// Makes the port give off [`EnablerSignal::Present`] under this ID.
        enables_id: Option<EnablerId>,
    },
    /// Fed exclusively by a constant edited in place on the node.
    ConstantOnly {
        /// Current value of the constant and the editor used for it.
        constant_type: ConstantInputType,
        /// Makes the port give off the signals of its constant under this ID.
        enables_id: Option<EnablerId>,
        /// Keeps the constant editable only while the spy holds.
        enabled_by: Option<EnablerSpy>,
    },
    /// Accepts both, the constant supplying the value while no connection
    /// exists.
    ConnectionAndConstant {
        /// Value used while the port is unconnected, and the editor used for it.
        constant_type: ConstantInputType,
        /// The connected output port, or `None` while the port is unconnected.
        data_source: Option<PortId>,
        /// Makes the port give off [`EnablerSignal::Present`] and the signals of its
        /// constant under this ID.
        enables_id: Option<EnablerId>,
        /// Keeps the constant editable only while the spy holds and the port is unconnected.
        enabled_by: Option<EnablerSpy>,
    },
}

impl PortInputType {
    /// An unconnected port that only accepts a connection.
    pub fn connection_only() -> Self {
        Self::ConnectionOnly {
            data_source: None,
            enables_id: None,
        }
    }

    /// An unconnected port that only accepts a connection and enables others.
    pub fn connection_only_with_enabler(enables_id: EnablerId) -> Self {
        Self::ConnectionOnly {
            data_source: None,
            enables_id: Some(enables_id),
        }
    }

    /// A port holding `constant_type`, which cannot be connected.
    pub fn constant_only(constant_type: ConstantInputType) -> Self {
        Self::ConstantOnly {
            constant_type,
            enables_id: None,
            enabled_by: None,
        }
    }

    /// A port holding `constant_type`, which cannot be connected, giving off its signals
    /// under `enables_id`.
    pub fn constant_only_with_enabler(
        constant_type: ConstantInputType,
        enables_id: EnablerId,
    ) -> Self {
        Self::ConstantOnly {
            constant_type,
            enables_id: Some(enables_id),
            enabled_by: None,
        }
    }

    /// A port holding `constant_type`, which cannot be connected, editable while `enabled_by` holds.
    pub fn constant_only_enabled_by(
        constant_type: ConstantInputType,
        enabled_by: EnablerSpy,
    ) -> Self {
        Self::ConstantOnly {
            constant_type,
            enables_id: None,
            enabled_by: Some(enabled_by),
        }
    }

    /// A port holding `constant_type`, which cannot be connected, editable while the port
    /// declaring `enabled_by_id` is ticked or connected.
    pub fn constant_only_enabled_by_id(
        constant_type: ConstantInputType,
        enabled_by_id: EnablerId,
    ) -> Self {
        Self::constant_only_enabled_by(constant_type, EnablerSpy::checked_or_present(enabled_by_id))
    }

    /// An unconnected port that falls back to `constant_type` until a connection
    /// is made.
    pub fn connection_and_constant(constant_type: ConstantInputType) -> Self {
        Self::ConnectionAndConstant {
            constant_type,
            data_source: None,
            enables_id: None,
            enabled_by: None,
        }
    }

    /// An unconnected port that falls back to `constant_type` until a connection
    /// is made, giving off its signals under `enables_id`.
    pub fn connection_and_constant_with_enabler(
        constant_type: ConstantInputType,
        enables_id: EnablerId,
    ) -> Self {
        Self::ConnectionAndConstant {
            constant_type,
            data_source: None,
            enables_id: Some(enables_id),
            enabled_by: None,
        }
    }

    /// An unconnected port that falls back to `constant_type` until a connection
    /// is made, its constant editable while `enabled_by` holds.
    pub fn connection_and_constant_enabled_by(
        constant_type: ConstantInputType,
        enabled_by: EnablerSpy,
    ) -> Self {
        Self::ConnectionAndConstant {
            constant_type,
            data_source: None,
            enables_id: None,
            enabled_by: Some(enabled_by),
        }
    }

    /// An unconnected port that falls back to `constant_type` until a connection
    /// is made, its constant editable while the port declaring `enabled_by_id` is
    /// ticked or connected.
    pub fn connection_and_constant_enabled_by_id(
        constant_type: ConstantInputType,
        enabled_by_id: EnablerId,
    ) -> Self {
        Self::connection_and_constant_enabled_by(
            constant_type,
            EnablerSpy::checked_or_present(enabled_by_id),
        )
    }

    pub(crate) fn enables_id(&self) -> Option<EnablerId> {
        self.declared_enabler_ids().next()
    }

    pub(crate) fn signal_state(&self, signal: EnablerSignal) -> bool {
        let constant = self
            .data_source()
            .is_none()
            .then(|| self.constant_type())
            .flatten();

        match signal {
            EnablerSignal::Present(_) => self.data_source().is_some(),
            EnablerSignal::Checked(_) => {
                matches!(
                    constant,
                    Some(ConstantInputType::Boolean { value: true, .. })
                )
            }
            EnablerSignal::Selected(_, index) => {
                matches!(constant, Some(ConstantInputType::Selection { selected, .. }) if *selected == index)
            }
        }
    }

    pub(crate) fn is_constant_enabled(&self, resolve: &impl Fn(EnablerSignal) -> bool) -> bool {
        let holds = |enabled_by: &Option<EnablerSpy>| {
            enabled_by.as_ref().is_none_or(|spy| spy.evaluate(resolve))
        };

        match self {
            Self::ConnectionOnly { .. } => false,
            Self::ConstantOnly { enabled_by, .. } => holds(enabled_by),
            Self::ConnectionAndConstant {
                data_source,
                enabled_by,
                ..
            } => data_source.is_none() && holds(enabled_by),
        }
    }

    fn enabled_by(&self) -> Option<&EnablerSpy> {
        match self {
            Self::ConnectionOnly { .. } => None,
            Self::ConstantOnly { enabled_by, .. }
            | Self::ConnectionAndConstant { enabled_by, .. } => enabled_by.as_ref(),
        }
    }

    fn data_source(&self) -> Option<PortId> {
        match self {
            Self::ConnectionOnly { data_source, .. }
            | Self::ConnectionAndConstant { data_source, .. } => *data_source,
            Self::ConstantOnly { .. } => None,
        }
    }

    fn declared_enabler_ids(&self) -> impl Iterator<Item=EnablerId> {
        let port_level = match self {
            Self::ConnectionOnly { enables_id, .. }
            | Self::ConstantOnly { enables_id, .. }
            | Self::ConnectionAndConstant { enables_id, .. } => *enables_id,
        };
        let constant_level = self
            .constant_type()
            .and_then(|constant| *constant.enables_id().0);

        port_level.into_iter().chain(constant_level)
    }

    pub(crate) fn constant_type(&self) -> Option<&ConstantInputType> {
        match self {
            Self::ConstantOnly { constant_type, .. } => Some(constant_type),
            Self::ConnectionAndConstant { constant_type, .. } => Some(constant_type),
            Self::ConnectionOnly { .. } => None,
        }
    }

    pub(crate) fn constant_type_mut(&mut self) -> Option<&mut ConstantInputType> {
        match self {
            Self::ConstantOnly { constant_type, .. } => Some(constant_type),
            Self::ConnectionAndConstant { constant_type, .. } => Some(constant_type),
            Self::ConnectionOnly { .. } => None,
        }
    }
}

pub(crate) fn resolve_signal<'a>(
    input_types: impl IntoIterator<Item=&'a PortInputType>,
    signal: EnablerSignal,
) -> bool {
    input_types
        .into_iter()
        .find(|input_type| input_type.enables_id() == Some(signal.id()))
        .is_some_and(|input_type| input_type.signal_state(signal))
}

pub(crate) fn validate_enablers<'a>(
    input_types: impl IntoIterator<Item=&'a PortInputType> + Clone,
) -> anyhow::Result<()> {
    const SINGLE_ID: usize = 1;
    let mut declared = HashSet::new();

    for input_type in input_types.clone() {
        if input_type.declared_enabler_ids().count() > SINGLE_ID {
            anyhow::bail!(
                "Port declares an EnablerId on both itself and its constant: {input_type:?}"
            );
        }

        if let Some(id) = input_type.enables_id()
            && !declared.insert(id)
        {
            anyhow::bail!("Duplicate EnablerId: {id:?}");
        }
    }

    for spy in input_types
        .into_iter()
        .filter_map(PortInputType::enabled_by)
    {
        if let Some(signal) = spy
            .signals()
            .into_iter()
            .find(|signal| !declared.contains(&signal.id()))
        {
            anyhow::bail!("No port declares the EnablerId of {signal:?}");
        }
    }

    Ok(())
}

/// A literal value entered directly on an input port, along with the kind of
/// widget used to edit it.
///
/// The variant also acts as the default value of the port until the user
/// changes it, and is converted into `D` through its `From<&ConstantInputType>`
/// implementation before it reaches
/// [`RuntimeState::inputs_map`](crate::RuntimeState::inputs_map).
#[non_exhaustive]
#[derive(PartialEq, Clone, Debug)]
pub enum ConstantInputType {
    /// A decimal number edited with a drag value.
    Float {
        /// Current value.
        value: f32,
        /// Smallest value the editor allows.
        min_bound: f32,
        /// Largest value the editor allows.
        max_bound: f32,
        /// Scale applied post-value retrieval by the UI
        scale_display: f32,
    },
    /// A whole number edited with a drag value.
    Integer {
        /// Current value.
        value: i32,
        /// Smallest value the editor allows.
        min_bound: i32,
        /// Largest value the editor allows.
        max_bound: i32,
        /// Steps on increment
        steps: i32,
    },
    /// Free text edited in a single line text field, typically a file path. The
    /// text is not checked against the file system.
    Path(String),
    /// Free text edited in a single line text field, literally just a string.
    String(String),
    /// One entry chosen from a fixed list with a combo box.
    Selection {
        /// Entries offered to the user, in display order.
        options: Arc<[String]>,
        /// Index of the chosen entry in `options`.
        selected: usize,
    },
    /// A float input with a checkbox to govern whether it is editable or not, returning an
    /// `Option<f32>`
    OptionalFloat {
        /// Functionally the same as `Option<f32>`
        value_option: OptionalFloat,
        /// Smallest value the editor allows.
        min_bound: f32,
        /// Largest value the editor allows.
        max_bound: f32,
    },
    /// An integer input with a checkbox to govern whether it is editable or not, returning an
    /// `Option<i32>`
    OptionalInteger {
        /// Functionally the same as `Option<i32>`
        value_option: OptionalInteger,
        /// Smallest value the editor allows.
        min_bound: i32,
        /// Largest value the editor allows.
        max_bound: i32,
    },
    /// An integer that is randomizable. This does not need any more attributes than a normal
    /// Integer, but it needs to be separated to be matched later
    RandomizableUnsignedInteger {
        /// Current value.
        value: u32,
        /// Smallest value the editor allows.
        min_bound: u32,
        /// Largest value the editor allows.
        max_bound: u32,
    },
    /// A boolean implemented as a checkbox in the UI
    Boolean {
        /// Current value.
        value: bool,
        /// What this value enables.
        enables_id: Option<EnablerId>,
    },
}

impl ConstantInputType {
    /// Returns the [`EnablerId`] declared by the constant, if any, together with
    /// its current state: the ticked value for `Boolean`, `true` otherwise.
    pub fn enables_id(&self) -> (&Option<EnablerId>, bool) {
        match self {
            ConstantInputType::Float { .. }
            | ConstantInputType::Integer { .. }
            | ConstantInputType::Path(_)
            | ConstantInputType::String(_)
            | ConstantInputType::Selection { .. }
            | ConstantInputType::OptionalFloat { .. }
            | ConstantInputType::OptionalInteger { .. }
            | ConstantInputType::RandomizableUnsignedInteger { .. } => (&None, true),
            ConstantInputType::Boolean { enables_id, value } => (enables_id, *value),
        }
    }
}

/// Optional float type derived from a combo Checkbox + DragValue
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OptionalFloat {
    /// Enabled value from the Checkbox
    pub enabled: bool,
    /// Float value from the DragValue
    pub value: f32,
}

impl OptionalFloat {
    /// Returns `None` if the value is not enabled, and `Some(f32)` if the value is enabled.
    pub fn get(&self) -> Option<f32> {
        self.enabled.then_some(self.value)
    }
}

/// Optional integer type derived from a combo Checkbox + DragValue
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OptionalInteger {
    /// Enabled value from the Checkbox
    pub enabled: bool,
    /// Integer value from the DragValue
    pub value: i32,
}

impl OptionalInteger {
    /// Returns `None` if the value is not enabled, and `Some(i32)` if the value is enabled.
    pub fn get(&self) -> Option<i32> {
        self.enabled.then_some(self.value)
    }
}

impl ConstantInputType {
    /// A float starting at `0.0` with no practical bounds.
    pub fn default_float() -> Self {
        Self::Float {
            value: 0.0,
            min_bound: f32::MIN,
            max_bound: f32::MAX,
            scale_display: 1.0,
        }
    }

    /// A float limited to `min_bound..=max_bound`, starting at `default` clamped
    /// into that range.
    ///
    /// # Panics
    ///
    /// Panics unless `max_bound` is greater than `min_bound`.
    pub fn bounded_float(default: f32, min_bound: f32, max_bound: f32) -> Self {
        assert!(max_bound > min_bound);

        Self::Float {
            value: default.clamp(min_bound, max_bound),
            min_bound,
            max_bound,
            scale_display: 1.0,
        }
    }

    /// An optional float limited to `min_bound..=max_bound`, starting at `default` clamped
    /// into that range, and an enabled state from `default_enabled`.
    ///
    /// # Panics
    ///
    /// Panics unless `max_bound` is greater than `min_bound`.
    pub fn bounded_optional_float(
        default_enabled: bool,
        default: f32,
        min_bound: f32,
        max_bound: f32,
    ) -> Self {
        assert!(max_bound > min_bound);

        Self::OptionalFloat {
            value_option: OptionalFloat {
                value: default.clamp(min_bound, max_bound),
                enabled: default_enabled,
            },
            min_bound,
            max_bound,
        }
    }

    /// An integer starting at `0` spanning the whole `i32` range.
    pub fn default_integer() -> Self {
        ConstantInputType::Integer {
            value: 0,
            min_bound: i32::MIN,
            max_bound: i32::MAX,
            steps: 1,
        }
    }

    /// An integer starting at `0` spanning the whole `u32` range.
    pub fn default_randomizable_unsigned_integer() -> Self {
        ConstantInputType::RandomizableUnsignedInteger {
            value: 0,
            min_bound: u32::MIN,
            max_bound: u32::MAX,
        }
    }

    /// An integer starting at a random value spanning the whole `u32` range.
    pub fn default_random_unsigned_integer() -> Self {
        Self::default_random_unsigned_integer_with(&mut rand::rng())
    }

    fn default_random_unsigned_integer_with(rng: &mut impl Rng) -> Self {
        ConstantInputType::RandomizableUnsignedInteger {
            value: random_in_range(rng, u32::MIN, u32::MAX),
            min_bound: u32::MIN,
            max_bound: u32::MAX,
        }
    }

    /// An integer limited to `min_bound..=max_bound`, starting at `default`
    /// clamped into that range.
    ///
    /// # Panics
    ///
    /// Panics unless `max_bound` is greater than `min_bound`.
    pub fn bounded_integer(default: i32, min_bound: i32, max_bound: i32) -> Self {
        assert!(max_bound > min_bound);

        ConstantInputType::Integer {
            value: default.clamp(min_bound, max_bound),
            min_bound,
            max_bound,
            steps: 1,
        }
    }

    /// An integer limited to `min_bound..=max_bound`, starting at `default`
    /// clamped into that range, with each drag increment scaled by `steps`.
    ///
    /// # Panics
    ///
    /// Panics unless `max_bound` is greater than `min_bound`.
    pub fn bounded_integer_with_steps(
        default: i32,
        min_bound: i32,
        max_bound: i32,
        steps: i32,
    ) -> Self {
        assert!(max_bound > min_bound);

        ConstantInputType::Integer {
            value: default.clamp(min_bound, max_bound),
            min_bound,
            max_bound,
            steps,
        }
    }

    /// An optional integer limited to `min_bound..=max_bound`, starting at `default` clamped
    /// into that range, and an enabled state from `default_enabled`.
    ///
    /// # Panics
    ///
    /// Panics unless `max_bound` is greater than `min_bound`.
    pub fn bounded_optional_integer(
        default_enabled: bool,
        default: i32,
        min_bound: i32,
        max_bound: i32,
    ) -> Self {
        assert!(max_bound > min_bound);

        Self::OptionalInteger {
            value_option: OptionalInteger {
                value: default.clamp(min_bound, max_bound),
                enabled: default_enabled,
            },
            min_bound,
            max_bound,
        }
    }

    /// A checkbox starting at `default`.
    pub fn boolean(default: bool) -> Self {
        Self::Boolean {
            value: default,
            enables_id: None,
        }
    }

    /// Helper function to build a [`ConstantInputType::Boolean`] with the provided `default` and
    /// [`EnablerId`] value.
    pub fn bounded_boolean_with_enabler(default: bool, enabler_id: EnablerId) -> Self {
        Self::Boolean {
            value: default,
            enables_id: Some(enabler_id),
        }
    }

    /// An empty text value.
    pub fn default_path() -> Self {
        Self::Path("".to_string())
    }

    /// A choice among `options`, starting with the first entry selected.
    ///
    /// # Panics
    ///
    /// Panics when `options` is empty.
    pub fn selection(options: impl Into<Arc<[String]>>) -> Self {
        let options = options.into();
        assert!(!options.is_empty());

        Self::Selection {
            options,
            selected: 0,
        }
    }
}

/// Presentation details a port type has to provide to the editor.
pub trait PortTypeInfo: Eq + Copy + Clone + Debug {
    /// Color used to draw ports and connections of this type.
    fn color(&self) -> Color32;
}

/// Linkage type for the port.
#[derive(PartialEq, Clone, Debug)]
pub enum PortLinkageType {
    /// Input linkage type, directly expressed by a [`PortInputType`]
    Input(PortInputType),
    /// Output linkage type, directly expressed by a [`PortOutputType`]
    Output(PortOutputType),
}

/// Static description of one port of a node.
pub struct PortDefinition<P, D>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    /// The key under which the port's value is looked up at runtime.
    pub(crate) name: String,
    /// Label shown next to the port.
    pub(crate) label: String,
    /// `true` for an input port, `false` for an output port.
    pub(crate) is_input: bool,
    /// Data kind carried by the port, which decides connection compatibility.
    pub(crate) port_type: P,
    /// How the port is linked: `Input` with its [`PortInputType`] or `Output`
    /// with its [`PortOutputType`].
    pub(crate) linkage_type: PortLinkageType,
    /// Whether the port must receive a value for the node to be evaluated.
    pub(crate) required: bool,
    /// Whether the port is drawn on the node.
    pub(crate) visible: bool,
    _marker: PhantomData<D>,
}

/// Declares an input port for [`NodeBlueprintDefinition::ports`].
///
/// `input_type` decides whether the port is fed by a connection, a constant or
/// either. When `required` is `true`, an unconnected connection-only port stops
/// the node from being evaluated and puts it in the error state; ports with a
/// constant always have a value, so the flag has no effect on them.
///
/// [`NodeBlueprintDefinition::ports`]: crate::NodeBlueprintDefinition::ports
pub fn input_port<P, D>(
    name: impl Into<String>,
    label: impl Into<String>,
    port_type: P,
    input_type: PortInputType,
    required: bool,
) -> PortDefinition<P, D>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    PortDefinition {
        name: name.into(),
        label: label.into(),
        is_input: true,
        port_type,
        linkage_type: PortLinkageType::Input(input_type),
        required,
        visible: true,
        _marker: PhantomData,
    }
}

/// Declares an output port for [`NodeBlueprintDefinition::ports`].
///
/// `name` is the key [`ComputeState::Done`] uses for the port's value. A port
/// with `visible` set to `false` is not drawn and cannot be connected, which
/// suits outputs that only feed the viewer through
/// [`port_for_viewer`](crate::NodeBlueprintDefinition::port_for_viewer).
///
/// `output_type` decides whether the port can be connected, shows its value as
/// inline text, or both. The text is filled in by the graph from the computed
/// value through `TryInto<String>`.
///
/// [`NodeBlueprintDefinition::ports`]: crate::NodeBlueprintDefinition::ports
/// [`ComputeState::Done`]: crate::ComputeState::Done
pub fn output_port<P, D>(
    name: impl Into<String>,
    label: impl Into<String>,
    port_type: P,
    output_type: PortOutputType,
    visible: bool,
) -> PortDefinition<P, D>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
{
    PortDefinition {
        name: name.into(),
        label: label.into(),
        is_input: false,
        port_type,
        linkage_type: PortLinkageType::Output(output_type),
        required: false,
        visible,
        _marker: PhantomData,
    }
}

pub(crate) struct Port<P: PortTypeInfo + Eq + Copy + Clone + Debug> {
    pub(crate) id: PortId,
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) direction: PortDirection,
    pub(crate) port_type: P,
    pub(crate) linkage_type: PortLinkageType,
    pub(crate) required: bool,
    pub(crate) visible: bool,
}

impl<P> PartialEq for Port<P>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
{
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.label == other.label
            && self.direction == other.direction
            && self.port_type == other.port_type
            && self.linkage_type == other.linkage_type
            && self.required == other.required
    }
}

impl<P> Debug for Port<P>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Port")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("label", &self.label)
            .field("direction", &self.direction)
            .field("port_type", &self.port_type)
            .field("linkage_type", &self.linkage_type)
            .field("required", &self.required)
            .finish()
    }
}

impl<P> Port<P>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
{
    pub(crate) fn from_definition<D>(port_id: PortId, definition: PortDefinition<P, D>) -> Self
    where
        D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
        for<'a> &'a D: TryInto<String>,
    {
        Self {
            id: port_id,
            name: definition.name,
            label: definition.label,
            direction: if definition.is_input {
                PortDirection::Input
            } else {
                PortDirection::Output
            },
            port_type: definition.port_type,
            linkage_type: definition.linkage_type,
            required: definition.required,
            visible: definition.visible,
        }
    }

    pub(in crate::graph) fn snapshot(&self, node_id: SnapshotNodeId) -> PortSnapshot {
        PortSnapshot {
            id: SnapshotPortId::new(node_id, self.id.index()),
            name: self.name.clone(),
            constant: match &self.linkage_type {
                PortLinkageType::Input(PortInputType::ConstantOnly { constant_type, .. }) => {
                    Some(ConstantValue::from(constant_type))
                }
                PortLinkageType::Input(PortInputType::ConnectionAndConstant {
                                           constant_type,
                                           ..
                                       }) => Some(ConstantValue::from(constant_type)),
                _ => None,
            },
        }
    }

    pub(crate) fn is_connection_compatible(&self, connection: &DragConnection<P>) -> bool {
        if connection.from_input && matches!(self.direction, PortDirection::Input)
            || !connection.from_input && matches!(self.direction, PortDirection::Output)
        {
            return false;
        }

        if connection.from_type != self.port_type {
            return false;
        }

        true
    }

    pub(crate) fn is_input(&self) -> bool {
        self.direction == PortDirection::Input
    }

    pub(crate) fn is_output(&self) -> bool {
        self.direction == PortDirection::Output
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::*;

    const FLOAT_MIN: f32 = -10.0;
    const FLOAT_MAX: f32 = 10.0;
    const FLOAT_INSIDE: f32 = 2.5;
    const FLOAT_BELOW: f32 = -20.0;
    const FLOAT_ABOVE: f32 = 20.0;

    const INTEGER_MIN: i32 = -10;
    const INTEGER_MAX: i32 = 10;
    const INTEGER_INSIDE: i32 = 3;
    const INTEGER_BELOW: i32 = -20;
    const INTEGER_ABOVE: i32 = 20;
    const INTEGER_STEPS: i32 = 5;

    const DEFAULT_FLOAT: f32 = 0.0;
    const DEFAULT_INTEGER: i32 = 0;
    const FIRST_OPTION_INDEX: usize = 0;

    const ENABLED_STATES: [bool; 2] = [true, false];

    const SEED: u64 = 7;
    const OTHER_SEED: u64 = 11;
    const RANDOM_SAMPLES: usize = 4;

    fn float(value: f32) -> ConstantInputType {
        ConstantInputType::Float {
            value,
            min_bound: FLOAT_MIN,
            max_bound: FLOAT_MAX,
            scale_display: 1.0,
        }
    }

    fn integer(value: i32) -> ConstantInputType {
        ConstantInputType::Integer {
            value,
            min_bound: INTEGER_MIN,
            max_bound: INTEGER_MAX,
            steps: 1,
        }
    }

    fn optional_float(enabled: bool, value: f32) -> ConstantInputType {
        ConstantInputType::OptionalFloat {
            value_option: OptionalFloat { enabled, value },
            min_bound: FLOAT_MIN,
            max_bound: FLOAT_MAX,
        }
    }

    fn optional_integer(enabled: bool, value: i32) -> ConstantInputType {
        ConstantInputType::OptionalInteger {
            value_option: OptionalInteger { enabled, value },
            min_bound: INTEGER_MIN,
            max_bound: INTEGER_MAX,
        }
    }

    fn full_range_unsigned(value: u32) -> ConstantInputType {
        ConstantInputType::RandomizableUnsignedInteger {
            value,
            min_bound: u32::MIN,
            max_bound: u32::MAX,
        }
    }

    fn unsigned_value(constant: &ConstantInputType) -> u32 {
        match constant {
            ConstantInputType::RandomizableUnsignedInteger { value, .. } => *value,
            other => panic!("Expected a randomizable unsigned integer, got {other:?}"),
        }
    }

    fn options(names: &[&str]) -> Arc<[String]> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn default_float_starts_at_zero_with_full_range() {
        let result = ConstantInputType::default_float();

        assert_eq!(
            result,
            ConstantInputType::Float {
                value: DEFAULT_FLOAT,
                min_bound: f32::MIN,
                max_bound: f32::MAX,
                scale_display: 1.0,
            }
        );
    }

    #[test]
    fn bounded_float_with_default_inside_bounds_keeps_default() {
        let result = ConstantInputType::bounded_float(FLOAT_INSIDE, FLOAT_MIN, FLOAT_MAX);

        assert_eq!(result, float(FLOAT_INSIDE));
    }

    #[test]
    fn bounded_float_with_default_on_a_bound_keeps_default() {
        let result = ConstantInputType::bounded_float(FLOAT_MAX, FLOAT_MIN, FLOAT_MAX);

        assert_eq!(result, float(FLOAT_MAX));
    }

    #[test]
    fn bounded_float_with_default_below_min_clamps_to_min() {
        let result = ConstantInputType::bounded_float(FLOAT_BELOW, FLOAT_MIN, FLOAT_MAX);

        assert_eq!(result, float(FLOAT_MIN));
    }

    #[test]
    fn bounded_float_with_default_above_max_clamps_to_max() {
        let result = ConstantInputType::bounded_float(FLOAT_ABOVE, FLOAT_MIN, FLOAT_MAX);

        assert_eq!(result, float(FLOAT_MAX));
    }

    #[test]
    #[should_panic(expected = "max_bound > min_bound")]
    fn bounded_float_with_equal_bounds_panics() {
        ConstantInputType::bounded_float(FLOAT_MIN, FLOAT_MIN, FLOAT_MIN);
    }

    #[test]
    #[should_panic(expected = "max_bound > min_bound")]
    fn bounded_float_with_inverted_bounds_panics() {
        ConstantInputType::bounded_float(FLOAT_INSIDE, FLOAT_MAX, FLOAT_MIN);
    }

    #[test]
    fn bounded_optional_float_with_default_inside_bounds_keeps_default_and_enabled_state() {
        let results = ENABLED_STATES.map(|enabled| {
            ConstantInputType::bounded_optional_float(enabled, FLOAT_INSIDE, FLOAT_MIN, FLOAT_MAX)
        });

        assert_eq!(
            results,
            ENABLED_STATES.map(|enabled| optional_float(enabled, FLOAT_INSIDE))
        );
    }

    #[test]
    fn bounded_optional_float_with_default_below_min_clamps_to_min_and_keeps_enabled_state() {
        let results = ENABLED_STATES.map(|enabled| {
            ConstantInputType::bounded_optional_float(enabled, FLOAT_BELOW, FLOAT_MIN, FLOAT_MAX)
        });

        assert_eq!(
            results,
            ENABLED_STATES.map(|enabled| optional_float(enabled, FLOAT_MIN))
        );
    }

    #[test]
    fn bounded_optional_float_with_default_above_max_clamps_to_max_and_keeps_enabled_state() {
        let results = ENABLED_STATES.map(|enabled| {
            ConstantInputType::bounded_optional_float(enabled, FLOAT_ABOVE, FLOAT_MIN, FLOAT_MAX)
        });

        assert_eq!(
            results,
            ENABLED_STATES.map(|enabled| optional_float(enabled, FLOAT_MAX))
        );
    }

    #[test]
    #[should_panic(expected = "max_bound > min_bound")]
    fn bounded_optional_float_with_inverted_bounds_panics() {
        ConstantInputType::bounded_optional_float(true, FLOAT_INSIDE, FLOAT_MAX, FLOAT_MIN);
    }

    #[test]
    fn default_integer_starts_at_zero_with_full_range() {
        let result = ConstantInputType::default_integer();

        assert_eq!(
            result,
            ConstantInputType::Integer {
                value: DEFAULT_INTEGER,
                min_bound: i32::MIN,
                max_bound: i32::MAX,
                steps: 1,
            }
        );
    }

    #[test]
    fn bounded_integer_with_default_inside_bounds_keeps_default() {
        let result = ConstantInputType::bounded_integer(INTEGER_INSIDE, INTEGER_MIN, INTEGER_MAX);

        assert_eq!(result, integer(INTEGER_INSIDE));
    }

    #[test]
    fn bounded_integer_with_steps_with_default_above_max_clamps_and_keeps_steps() {
        let result = ConstantInputType::bounded_integer_with_steps(
            INTEGER_ABOVE,
            INTEGER_MIN,
            INTEGER_MAX,
            INTEGER_STEPS,
        );

        assert_eq!(
            result,
            ConstantInputType::Integer {
                value: INTEGER_MAX,
                min_bound: INTEGER_MIN,
                max_bound: INTEGER_MAX,
                steps: INTEGER_STEPS,
            }
        );
    }

    #[test]
    fn bounded_integer_with_default_on_a_bound_keeps_default() {
        let result = ConstantInputType::bounded_integer(INTEGER_MIN, INTEGER_MIN, INTEGER_MAX);

        assert_eq!(result, integer(INTEGER_MIN));
    }

    #[test]
    fn bounded_integer_with_default_below_min_clamps_to_min() {
        let result = ConstantInputType::bounded_integer(INTEGER_BELOW, INTEGER_MIN, INTEGER_MAX);

        assert_eq!(result, integer(INTEGER_MIN));
    }

    #[test]
    fn bounded_integer_with_default_above_max_clamps_to_max() {
        let result = ConstantInputType::bounded_integer(INTEGER_ABOVE, INTEGER_MIN, INTEGER_MAX);

        assert_eq!(result, integer(INTEGER_MAX));
    }

    #[test]
    #[should_panic(expected = "max_bound > min_bound")]
    fn bounded_integer_with_equal_bounds_panics() {
        ConstantInputType::bounded_integer(INTEGER_MIN, INTEGER_MIN, INTEGER_MIN);
    }

    #[test]
    #[should_panic(expected = "max_bound > min_bound")]
    fn bounded_integer_with_inverted_bounds_panics() {
        ConstantInputType::bounded_integer(INTEGER_INSIDE, INTEGER_MAX, INTEGER_MIN);
    }

    #[test]
    fn bounded_optional_integer_with_default_inside_bounds_keeps_default_and_enabled_state() {
        let results = ENABLED_STATES.map(|enabled| {
            ConstantInputType::bounded_optional_integer(
                enabled,
                INTEGER_INSIDE,
                INTEGER_MIN,
                INTEGER_MAX,
            )
        });

        assert_eq!(
            results,
            ENABLED_STATES.map(|enabled| optional_integer(enabled, INTEGER_INSIDE))
        );
    }

    #[test]
    fn bounded_optional_integer_with_default_below_min_clamps_to_min_and_keeps_enabled_state() {
        let results = ENABLED_STATES.map(|enabled| {
            ConstantInputType::bounded_optional_integer(
                enabled,
                INTEGER_BELOW,
                INTEGER_MIN,
                INTEGER_MAX,
            )
        });

        assert_eq!(
            results,
            ENABLED_STATES.map(|enabled| optional_integer(enabled, INTEGER_MIN))
        );
    }

    #[test]
    fn bounded_optional_integer_with_default_above_max_clamps_to_max_and_keeps_enabled_state() {
        let results = ENABLED_STATES.map(|enabled| {
            ConstantInputType::bounded_optional_integer(
                enabled,
                INTEGER_ABOVE,
                INTEGER_MIN,
                INTEGER_MAX,
            )
        });

        assert_eq!(
            results,
            ENABLED_STATES.map(|enabled| optional_integer(enabled, INTEGER_MAX))
        );
    }

    #[test]
    #[should_panic(expected = "max_bound > min_bound")]
    fn bounded_optional_integer_with_inverted_bounds_panics() {
        ConstantInputType::bounded_optional_integer(true, INTEGER_INSIDE, INTEGER_MAX, INTEGER_MIN);
    }

    #[test]
    fn default_unsigned_integer_starts_at_zero_with_full_range() {
        let result = ConstantInputType::default_randomizable_unsigned_integer();

        assert_eq!(result, full_range_unsigned(u32::MIN));
    }

    #[test]
    fn default_random_unsigned_integer_spans_the_full_range() {
        let result = ConstantInputType::default_random_unsigned_integer();

        assert_eq!(result, full_range_unsigned(unsigned_value(&result)));
    }

    #[test]
    fn default_random_unsigned_integer_varies_between_calls() {
        let values: Vec<u32> = (0..RANDOM_SAMPLES)
            .map(|_| unsigned_value(&ConstantInputType::default_random_unsigned_integer()))
            .collect();

        assert!(values.iter().any(|value| *value != values[0]));
    }

    #[test]
    fn default_random_unsigned_integer_with_same_seed_yields_same_value() {
        let mut first_rng = StdRng::seed_from_u64(SEED);
        let mut second_rng = StdRng::seed_from_u64(SEED);

        let first = ConstantInputType::default_random_unsigned_integer_with(&mut first_rng);
        let second = ConstantInputType::default_random_unsigned_integer_with(&mut second_rng);

        assert_eq!(first, second);
    }

    #[test]
    fn default_random_unsigned_integer_with_different_seeds_yields_different_values() {
        let mut first_rng = StdRng::seed_from_u64(SEED);
        let mut second_rng = StdRng::seed_from_u64(OTHER_SEED);

        let first = ConstantInputType::default_random_unsigned_integer_with(&mut first_rng);
        let second = ConstantInputType::default_random_unsigned_integer_with(&mut second_rng);

        assert_ne!(first, second);
    }

    #[test]
    fn default_path_starts_empty() {
        let result = ConstantInputType::default_path();

        assert_eq!(result, ConstantInputType::Path(String::new()));
    }

    #[test]
    fn selection_starts_with_the_first_option_selected() {
        let names = options(&["first", "second", "third"]);

        let result = ConstantInputType::selection(Arc::clone(&names));

        assert_eq!(
            result,
            ConstantInputType::Selection {
                options: names,
                selected: FIRST_OPTION_INDEX,
            }
        );
    }

    #[test]
    fn selection_accepts_a_vector_of_options() {
        let names = vec!["only".to_string()];

        let result = ConstantInputType::selection(names.clone());

        assert_eq!(
            result,
            ConstantInputType::Selection {
                options: names.into(),
                selected: FIRST_OPTION_INDEX,
            }
        );
    }

    #[test]
    #[should_panic(expected = "!options.is_empty()")]
    fn selection_without_options_panics() {
        ConstantInputType::selection(Vec::<String>::new());
    }
}

#[derive(Debug)]
pub(crate) struct EvaluationPort {
    pub(crate) id: PortId,
    pub(crate) name: String,
    pub(crate) direction: PortDirection,
    pub(crate) input_type: Option<PortInputType>,
    pub(crate) required: bool,
}

impl EvaluationPort {
    pub(crate) fn from_port<P>(port: &Port<P>) -> Self
    where
        P: PortTypeInfo + Eq + Copy + Clone + Debug,
    {
        let input_type = match &port.linkage_type {
            PortLinkageType::Input(input_type) => Some(input_type.clone()),
            PortLinkageType::Output(_) => None,
        };

        Self {
            id: port.id,
            name: port.name.clone(),
            direction: port.direction,
            input_type,
            required: port.required,
        }
    }

    pub(crate) fn is_input(&self) -> bool {
        self.direction == PortDirection::Input
    }
}

#[cfg(test)]
mod evaluation_port_tests {
    use super::*;

    const FIRST_PORT_INDEX: u32 = 0;
    const PORT_NAME: &str = "in";
    const PORT_LABEL: &str = "In";
    const CONSTANT_VALUE: f32 = 3.0;
    const CONSTANT_MIN: f32 = -10.0;
    const CONSTANT_MAX: f32 = 10.0;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestPort {
        Number,
    }

    impl PortTypeInfo for TestPort {
        fn color(&self) -> Color32 {
            Color32::WHITE
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct TestValue;

    impl From<&ConstantInputType> for TestValue {
        fn from(_: &ConstantInputType) -> Self {
            TestValue
        }
    }

    struct NotInlineable;

    impl TryFrom<&TestValue> for String {
        type Error = NotInlineable;

        fn try_from(_: &TestValue) -> Result<Self, Self::Error> {
            Err(NotInlineable)
        }
    }

    fn constant() -> ConstantInputType {
        ConstantInputType::bounded_float(CONSTANT_VALUE, CONSTANT_MIN, CONSTANT_MAX)
    }

    fn constant_port() -> Port<TestPort> {
        let id = port_id(NodeId::default(), FIRST_PORT_INDEX);
        let definition = input_port::<TestPort, TestValue>(
            PORT_NAME,
            PORT_LABEL,
            TestPort::Number,
            PortInputType::constant_only(constant()),
            true,
        );

        Port::from_definition(id, definition)
    }

    #[test]
    fn constant_type_constant_only_returns_constant() {
        let input_type = PortInputType::constant_only(constant());

        let result = input_type.constant_type();

        assert_eq!(result, Some(&constant()));
    }

    #[test]
    fn constant_type_connection_and_constant_returns_constant() {
        let input_type = PortInputType::connection_and_constant(constant());

        let result = input_type.constant_type();

        assert_eq!(result, Some(&constant()));
    }

    #[test]
    fn constant_type_connection_only_returns_none() {
        let input_type = PortInputType::connection_only();

        let result = input_type.constant_type();

        assert!(result.is_none());
    }

    #[test]
    fn from_port_keeps_port_id() {
        let port = constant_port();

        let result = EvaluationPort::from_port(&port);

        assert_eq!(result.id, port.id);
    }

    fn output_port_with_text() -> Port<TestPort> {
        let id = port_id(NodeId::default(), FIRST_PORT_INDEX);
        let definition = output_port::<TestPort, TestValue>(
            PORT_NAME,
            PORT_LABEL,
            TestPort::Number,
            PortOutputType::ConstantOnly { text_value: None },
            true,
        );

        Port::from_definition(id, definition)
    }

    #[test]
    fn from_port_input_port_keeps_input_type() {
        let port = constant_port();

        let result = EvaluationPort::from_port(&port);

        assert_eq!(
            result.input_type,
            Some(PortInputType::constant_only(constant()))
        );
    }

    #[test]
    fn from_port_output_port_has_no_input_type() {
        let port = output_port_with_text();

        let result = EvaluationPort::from_port(&port);

        assert!(result.input_type.is_none());
    }

    #[test]
    fn is_output_output_port_returns_true() {
        let port = output_port_with_text();

        let result = port.is_output();

        assert!(result);
    }

    #[test]
    fn is_output_input_port_returns_false() {
        let port = constant_port();

        let result = port.is_output();

        assert!(!result);
    }

    #[test]
    fn is_input_input_port_returns_true() {
        let port = EvaluationPort::from_port(&constant_port());

        let result = port.is_input();

        assert!(result);
    }
}
