#![warn(missing_docs)]

//! A node graph editor for [egui] with a built-in evaluation runtime.
//!
//! knobs draws an editable graph of nodes and connections, and it also evaluates
//! that graph. Each node kind is described by a [`NodeBlueprintDefinition`]: its
//! ports, its place in the creation menu and a [`compute`] function that turns
//! input values into output values. The editor keeps track of which results are
//! still valid, recomputes only what an edit made stale, and can run slow nodes
//! on worker threads when enabled in [`Options`](options::Options).
//!
//! # Concepts
//!
//! * **Blueprints** describe node kinds. They are usually an enum with one variant
//!   per kind, listed by [`NodeBlueprintDefinition::all`].
//! * **Ports** are declared with [`input_port`] and [`output_port`]. Their type `P`
//!   implements [`PortTypeInfo`] and decides which connections are allowed.
//!   Input ports can be fed by a connection, by a constant edited on the node
//!   ([`ConstantInputType`]), or by either, as chosen with [`PortInputType`].
//!   Output ports choose with [`PortOutputType`] whether they accept
//!   connections, show their value as inline text, or both.
//! * **Values** of type `D` travel along connections. Every `D` must be buildable
//!   from a [`ConstantInputType`] so that constants and connected values reach
//!   [`compute`] the same way, through [`RuntimeState::inputs_map`]. `&D` must
//!   also implement `TryInto<String>`: output ports whose [`PortOutputType`] has
//!   a constant part show this text inline once the node is computed, and a
//!   failed conversion shows nothing.
//! * **Focus** drives evaluation. Nothing is computed until the user focuses a
//!   node with its eye button. The graph then evaluates the focused node and every
//!   upstream node whose result is missing or stale, and reports the focused
//!   node's viewer output through [`graph::GraphEventHandler`].
//! * **Snapshots** capture the graph as a [`graph::snapshot::GraphSnapshot`], which
//!   converts to and from RON text for saving and loading.
//!
//! # Example
//!
//! A two-node library: a constant and a node that doubles its input.
//!
//! ```
//! use std::convert::Infallible;
//! use std::sync::Arc;
//!
//! use egui::Color32;
//! use knobs::graph::GraphEventHandler;
//! use knobs::options::Options;
//! use knobs::{
//!     ComputeState, ConstantInputType, Graph, NodeBlueprintDefinition, PortDefinition,
//!     PortInputType, PortOutputType, PortTypeInfo, RuntimeState, input_port, output_port,
//! };
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! enum Kind {
//!     Number,
//! }
//!
//! impl PortTypeInfo for Kind {
//!     fn color(&self) -> Color32 {
//!         Color32::LIGHT_BLUE
//!     }
//! }
//!
//! #[derive(Debug, Clone, PartialEq)]
//! struct Value(f32);
//!
//! impl TryFrom<&Value> for String {
//!     type Error = Infallible;
//!
//!     fn try_from(Value(value): &Value) -> Result<Self, Self::Error> {
//!         Ok(value.to_string())
//!     }
//! }
//!
//! impl From<&ConstantInputType> for Value {
//!     fn from(constant: &ConstantInputType) -> Self {
//!         match constant {
//!             ConstantInputType::Float { value, .. } => Value(*value),
//!             _ => Value(0.0),
//!         }
//!     }
//! }
//!
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
//! enum Blueprint {
//!     Constant,
//!     Double,
//! }
//!
//! impl NodeBlueprintDefinition<Kind, Value> for Blueprint {
//!     fn all() -> &'static [Self] {
//!         &[Blueprint::Constant, Blueprint::Double]
//!     }
//!
//!     fn name(&self) -> &'static str {
//!         match self {
//!             Blueprint::Constant => "Constant",
//!             Blueprint::Double => "Double",
//!         }
//!     }
//!
//!     fn ports(&self) -> Vec<PortDefinition<Kind, Value>> {
//!         let input = match self {
//!             Blueprint::Constant => PortInputType::constant_only(ConstantInputType::default_float()),
//!             Blueprint::Double => PortInputType::connection_only(),
//!         };
//!
//!         vec![
//!             input_port("in", "In", Kind::Number, input, true),
//!             output_port("out", "Out", Kind::Number, PortOutputType::ConnectionAndConstant { text_value: None }, true),
//!         ]
//!     }
//!
//!     fn categories(&self) -> Vec<&'static str> {
//!         vec!["Math"]
//!     }
//!
//!     fn compute(&mut self, state: &mut RuntimeState<Value>) -> anyhow::Result<ComputeState<Value>> {
//!         let Value(input) = state.inputs_map.get("in")?;
//!         let output = match self {
//!             Blueprint::Constant => *input,
//!             Blueprint::Double => input * 2.0,
//!         };
//!
//!         Ok(ComputeState::done(vec![("out".to_string(), Arc::new(Value(output)))]))
//!     }
//!
//!     fn port_for_viewer(&self) -> Option<String> {
//!         Some("out".to_string())
//!     }
//! }
//!
//! struct PrintFocusedValue;
//!
//! impl GraphEventHandler<Value> for PrintFocusedValue {
//!     fn on_focused_output_change(&self, data: Option<Arc<Value>>) -> anyhow::Result<()> {
//!         println!("Focused node now shows {data:?}");
//!         Ok(())
//!     }
//! }
//!
//! type Editor = Graph<Kind, Value, Blueprint, PrintFocusedValue>;
//!
//! fn create_editor() -> Editor {
//!     Graph::new(Options::default(), PrintFocusedValue)
//! }
//!
//! fn draw_editor(ui: &mut egui::Ui, editor: &mut Editor) {
//!     editor.show(ui);
//! }
//! ```
//!
//! [`compute`]: NodeBlueprintDefinition::compute

pub mod graph;
mod helpers;
pub mod options;
mod store;

pub use graph::node::{ComputeState, NodeBlueprintDefinition};
pub use graph::Graph;
pub use graph::node::port::{
    ConstantInputType, EnablerId, EnablerSignal, EnablerSpy, OptionalFloat, OptionalInteger,
    PortDefinition, PortInputType, PortTypeInfo, enabler_id, input_port, output_port,
    PortOutputType, PortLinkageType,
};
pub use graph::node::state::RuntimeState;
pub use graph::canvas::Canvas;
pub use store::{GenericStore, GetError};
