use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::graph::GraphEventHandler;
use crate::graph::node::port::PortOutputType;
use crate::graph::node::threading::POLL_INTERVAL;
use crate::options::Options;
use crate::{
    ComputeState, ConstantInputType, NodeBlueprintDefinition, PortDefinition, PortInputType,
    PortTypeInfo, RuntimeState, input_port, output_port,
};

pub(crate) const INPUT: &str = "in";
pub(crate) const OUTPUT: &str = "out";
pub(crate) const INPUT_PORT_INDEX: u32 = 0;
pub(crate) const OUTPUT_PORT_INDEX: u32 = 1;
pub(crate) const SOURCE_VALUE: f32 = 2.0;
pub(crate) const INCREMENT: f32 = 1.0;
pub(crate) const GATE_KEY: &str = "gate";
pub(crate) const PROBE_KEY: &str = "probe";
pub(crate) const LIFETIME_KEY: &str = "lifetime";
pub(crate) const RENDEZVOUS_KEY: &str = "rendezvous";
pub(crate) const TEST_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const SOLE_HOLDER: usize = 1;

const VALUE_MIN: f32 = -1000.0;
const VALUE_MAX: f32 = 1000.0;
const NO_OFFSET: f32 = 0.0;

static DROPPED_COMPUTE_THREADS: Mutex<Vec<ThreadId>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadedPort {
    Number,
}

impl PortTypeInfo for ThreadedPort {
    fn color(&self) -> Color32 {
        Color32::WHITE
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Number(pub(crate) f32);

impl From<&Number> for String {
    fn from(Number(value): &Number) -> Self {
        value.to_string()
    }
}

impl From<&ConstantInputType> for Number {
    fn from(constant: &ConstantInputType) -> Self {
        match constant {
            ConstantInputType::Float { value, .. } => Number(*value),
            _ => Number::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Gate(Arc<AtomicBool>);

impl Gate {
    pub(crate) fn open(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn is_open(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ThreadProbe(Arc<Mutex<Vec<ThreadId>>>);

impl ThreadProbe {
    pub(crate) fn threads(&self) -> Vec<ThreadId> {
        self.0.lock().unwrap().clone()
    }

    fn record_current(&self) {
        self.0.lock().unwrap().push(thread::current().id());
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Lifetime(Arc<()>);

impl Lifetime {
    pub(crate) fn holders(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Rendezvous {
    arrived: Arc<AtomicUsize>,
    expected: usize,
}

impl Rendezvous {
    pub(crate) fn new(expected: usize) -> Self {
        Self {
            arrived: Arc::new(AtomicUsize::default()),
            expected,
        }
    }

    fn arrive_and_wait(&self) -> bool {
        self.arrived.fetch_add(1, Ordering::SeqCst);

        wait_until(|| self.arrived.load(Ordering::SeqCst) >= self.expected)
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingHandler {
    outputs: Arc<Mutex<Vec<Option<Arc<Number>>>>>,
}

impl RecordingHandler {
    pub(crate) fn last_output(&self) -> Option<Arc<Number>> {
        self.outputs.lock().unwrap().last().cloned().flatten()
    }

    pub(crate) fn call_count(&self) -> usize {
        self.outputs.lock().unwrap().len()
    }
}

impl GraphEventHandler<Number> for RecordingHandler {
    fn on_focused_output_change(&self, data: Option<Arc<Number>>) -> anyhow::Result<()> {
        self.outputs.lock().unwrap().push(data);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ThreadedBlueprint {
    Source,
    Increment,
    InlineIncrement,
    Gated,
    Meeting,
    Failing,
    Panicking,
    Erroring,
    Stopping,
}

impl NodeBlueprintDefinition<ThreadedPort, Number> for ThreadedBlueprint {
    fn all() -> &'static [Self] {
        &[
            ThreadedBlueprint::Source,
            ThreadedBlueprint::Increment,
            ThreadedBlueprint::InlineIncrement,
            ThreadedBlueprint::Gated,
            ThreadedBlueprint::Meeting,
            ThreadedBlueprint::Failing,
            ThreadedBlueprint::Panicking,
            ThreadedBlueprint::Erroring,
            ThreadedBlueprint::Stopping,
        ]
    }

    fn name(&self) -> &'static str {
        match self {
            ThreadedBlueprint::Source => "Source",
            ThreadedBlueprint::Increment => "Increment",
            ThreadedBlueprint::InlineIncrement => "InlineIncrement",
            ThreadedBlueprint::Gated => "Gated",
            ThreadedBlueprint::Meeting => "Meeting",
            ThreadedBlueprint::Failing => "Failing",
            ThreadedBlueprint::Panicking => "Panicking",
            ThreadedBlueprint::Erroring => "Erroring",
            ThreadedBlueprint::Stopping => "Stopping",
        }
    }

    fn ports(&self) -> Vec<PortDefinition<ThreadedPort, Number>> {
        let input_type = match self {
            ThreadedBlueprint::Source => PortInputType::constant_only(
                ConstantInputType::bounded_float(SOURCE_VALUE, VALUE_MIN, VALUE_MAX),
            ),
            _ => PortInputType::connection_only(),
        };

        vec![
            input_port(INPUT, INPUT, ThreadedPort::Number, input_type, true),
            output_port(
                OUTPUT,
                OUTPUT,
                ThreadedPort::Number,
                PortOutputType::ConnectionOnly,
                true,
            ),
        ]
    }

    fn multithreading(&self) -> bool {
        !matches!(self, ThreadedBlueprint::InlineIncrement)
    }

    fn categories(&self) -> Vec<&'static str> {
        vec![""]
    }

    fn compute(
        &mut self,
        state: &mut RuntimeState<Number>,
    ) -> anyhow::Result<ComputeState<Number>> {
        if let Ok(probe) = state.user_state.get::<ThreadProbe>(PROBE_KEY) {
            probe.record_current();
        }

        match self {
            ThreadedBlueprint::Source => offset_input(state, NO_OFFSET),
            ThreadedBlueprint::Increment | ThreadedBlueprint::InlineIncrement => {
                offset_input(state, INCREMENT)
            }
            ThreadedBlueprint::Gated => {
                if state.user_state.get::<Gate>(GATE_KEY)?.is_open() {
                    return offset_input(state, NO_OFFSET);
                }

                Ok(ComputeState::Running)
            }
            ThreadedBlueprint::Meeting => {
                if !state
                    .user_state
                    .get::<Rendezvous>(RENDEZVOUS_KEY)?
                    .arrive_and_wait()
                {
                    anyhow::bail!("Peer worker never reached the rendezvous");
                }

                offset_input(state, NO_OFFSET)
            }
            ThreadedBlueprint::Failing => anyhow::bail!("Failing blueprint always fails"),
            ThreadedBlueprint::Panicking => panic!("Panicking blueprint always panics"),
            ThreadedBlueprint::Erroring => Ok(ComputeState::Error),
            ThreadedBlueprint::Stopping => Ok(ComputeState::Stopped),
        }
    }

    fn drop_compute(&mut self) -> anyhow::Result<()> {
        DROPPED_COMPUTE_THREADS
            .lock()
            .unwrap()
            .push(thread::current().id());

        Ok(())
    }

    fn port_for_viewer(&self) -> Option<String> {
        Some(OUTPUT.to_string())
    }
}

fn offset_input(state: &RuntimeState<Number>, offset: f32) -> anyhow::Result<ComputeState<Number>> {
    let Number(input) = state.inputs_map.get(INPUT)?;

    Ok(ComputeState::done(vec![(
        OUTPUT.to_string(),
        Arc::new(Number(input + offset)),
    )]))
}

pub(crate) fn threaded_options() -> Options {
    Options::default().with_multithreading(true)
}

pub(crate) fn inline_options() -> Options {
    Options::default()
}

pub(crate) fn wait_until(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + TEST_TIMEOUT;

    while Instant::now() < deadline {
        if condition() {
            return true;
        }

        thread::sleep(POLL_INTERVAL);
    }

    condition()
}

pub(crate) fn compute_was_dropped_on_any(threads: &[ThreadId]) -> bool {
    DROPPED_COMPUTE_THREADS
        .lock()
        .unwrap()
        .iter()
        .any(|thread_id| threads.contains(thread_id))
}
