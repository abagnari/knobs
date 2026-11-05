use std::collections::HashMap;
use std::f32::consts::TAU;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Sense, Stroke, Ui, pos2, vec2};
use knobs::graph::snapshot::GraphSnapshot;
use knobs::graph::{GraphEventHandler, MenuEvent};
use knobs::options::Options;
use knobs::{
    Canvas, ComputeState, ConstantInputType, GenericStore, Graph, NodeBlueprintDefinition,
    PortDefinition, PortInputType, PortOutputType, PortTypeInfo, RuntimeState, input_port,
    output_port,
};
use serde::{Deserialize, Serialize};

const APP_NAME: &str = "knobs signal lab";
const WINDOW_SIZE: [f32; 2] = [1440.0, 810.0];
const DEMO_GRAPH: &str = include_str!("signal_lab.ron");
const SAVE_FILE: &str = "signal_lab_saved.ron";
const SIDE_PANEL_WIDTH: f32 = 240.0;

const INPUT: &str = "input";
const OUTPUT: &str = "output";
const FREQUENCY: &str = "frequency";
const AMPLITUDE: &str = "amplitude";
const SHAPE: &str = "shape";
const FIRST: &str = "a";
const SECOND: &str = "b";
const BALANCE: &str = "balance";
const GAIN: &str = "gain";
const WINDOW: &str = "window";
const PEAK: &str = "peak";
const RMS: &str = "rms";
const TITLE: &str = "title";
const TRACE: &str = "trace";
const LABEL: &str = "label";

const SAMPLE_COUNT: usize = 256;
const MIN_FREQUENCY: f32 = 0.5;
const MAX_FREQUENCY: f32 = 8.0;
const DEFAULT_FREQUENCY: f32 = 2.0;
const MIN_AMPLITUDE: f32 = 0.0;
const MAX_AMPLITUDE: f32 = 2.0;
const DEFAULT_AMPLITUDE: f32 = 1.0;
const MIN_BALANCE: f32 = 0.0;
const MAX_BALANCE: f32 = 1.0;
const DEFAULT_BALANCE: f32 = 0.5;
const MIN_GAIN: f32 = 0.0;
const MAX_GAIN: f32 = 4.0;
const DEFAULT_GAIN: f32 = 1.0;
const MIN_WINDOW: i32 = 1;
const MAX_WINDOW: i32 = 32;
const DEFAULT_WINDOW: i32 = 8;
const SMOOTHING_DELAY: Duration = Duration::from_millis(800);
const DEFAULT_TITLE: &str = "Scope";
const DEFAULT_NUMBER: f32 = 0.0;

const HALF_CYCLE: f32 = 0.5;
const FULL_SCALE: f32 = 1.0;
const TRIANGLE_SLOPE: f32 = 4.0;
const SAW_SLOPE: f32 = 2.0;
const DISPLAY_PRECISION: usize = 3;

const NODE_WIDTH: f32 = 200.0;
const SCOPE_NODE_WIDTH: f32 = 240.0;
const SCOPE_HEIGHT: f32 = 110.0;
const SCOPE_RANGE: f32 = 2.0;
const SCOPE_PADDING: f32 = 6.0;
const SCOPE_TITLE_SIZE: f32 = 11.0;
const SCOPE_CORNER_RADIUS: f32 = 4.0;
const SCOPE_TRACE_WIDTH: f32 = 1.5;
const SCOPE_AXIS_WIDTH: f32 = 1.0;
const SCOPE_BACKGROUND: Color32 = Color32::from_rgb(18, 22, 28);
const SCOPE_AXIS: Color32 = Color32::from_rgb(52, 60, 72);
const SCOPE_TRACE: Color32 = Color32::from_rgb(110, 220, 190);
const SCOPE_TEXT: Color32 = Color32::from_rgb(150, 160, 175);
const SAMPLES_KEY: &str = "samples";
const TITLE_KEY: &str = "title";

const NUMBER_COLOR: Color32 = Color32::from_rgb(240, 170, 80);
const SIGNAL_COLOR: Color32 = Color32::from_rgb(110, 220, 190);
const TEXT_COLOR: Color32 = Color32::from_rgb(170, 150, 230);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    Signal,
    Text,
}

impl PortTypeInfo for Kind {
    fn color(&self) -> Color32 {
        match self {
            Kind::Number => NUMBER_COLOR,
            Kind::Signal => SIGNAL_COLOR,
            Kind::Text => TEXT_COLOR,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f32),
    Signal(Arc<[f32]>),
    Text(String),
}

struct NoInlineText;

impl TryFrom<&Value> for String {
    type Error = NoInlineText;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        match value {
            Value::Number(number) => Ok(format!("{number:.DISPLAY_PRECISION$}")),
            Value::Text(text) => Ok(text.clone()),
            Value::Signal(_) => Err(NoInlineText),
        }
    }
}

impl From<&ConstantInputType> for Value {
    fn from(constant: &ConstantInputType) -> Self {
        match constant {
            ConstantInputType::Float { value, .. } => Value::Number(*value),
            ConstantInputType::Integer { value, .. } => Value::Number(*value as f32),
            ConstantInputType::Selection { selected, .. } => Value::Number(*selected as f32),
            ConstantInputType::String(text) => Value::Text(text.clone()),
            _ => Value::Number(DEFAULT_NUMBER),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waveform {
    Sine,
    Square,
    Triangle,
    Saw,
}

impl Waveform {
    const ALL: [Waveform; 4] = [
        Waveform::Sine,
        Waveform::Square,
        Waveform::Triangle,
        Waveform::Saw,
    ];

    fn name(self) -> &'static str {
        match self {
            Waveform::Sine => "Sine",
            Waveform::Square => "Square",
            Waveform::Triangle => "Triangle",
            Waveform::Saw => "Saw",
        }
    }

    fn from_index(index: f32) -> Self {
        Self::ALL
            .get(index as usize)
            .copied()
            .unwrap_or(Waveform::Sine)
    }

    fn sample(self, phase: f32) -> f32 {
        let cycle = phase.fract();

        match self {
            Waveform::Sine => (cycle * TAU).sin(),
            Waveform::Square if cycle < HALF_CYCLE => FULL_SCALE,
            Waveform::Square => -FULL_SCALE,
            Waveform::Triangle => FULL_SCALE - TRIANGLE_SLOPE * (cycle - HALF_CYCLE).abs(),
            Waveform::Saw => SAW_SLOPE * cycle - FULL_SCALE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Blueprint {
    Oscillator,
    Mix,
    Gain,
    Smooth,
    Measure,
    Scope,
}

impl NodeBlueprintDefinition<Kind, Value> for Blueprint {
    fn all() -> &'static [Self] {
        &[
            Blueprint::Oscillator,
            Blueprint::Mix,
            Blueprint::Gain,
            Blueprint::Smooth,
            Blueprint::Measure,
            Blueprint::Scope,
        ]
    }

    fn name(&self) -> &'static str {
        match self {
            Blueprint::Oscillator => "Oscillator",
            Blueprint::Mix => "Mix",
            Blueprint::Gain => "Gain",
            Blueprint::Smooth => "Smooth (slow)",
            Blueprint::Measure => "Measure",
            Blueprint::Scope => "Scope",
        }
    }

    fn ports(&self) -> Vec<PortDefinition<Kind, Value>> {
        match self {
            Blueprint::Oscillator => vec![
                number_input(
                    FREQUENCY,
                    "Frequency",
                    DEFAULT_FREQUENCY,
                    MIN_FREQUENCY,
                    MAX_FREQUENCY,
                ),
                number_input(
                    AMPLITUDE,
                    "Amplitude",
                    DEFAULT_AMPLITUDE,
                    MIN_AMPLITUDE,
                    MAX_AMPLITUDE,
                ),
                input_port(SHAPE, "Shape", Kind::Number, waveform_selection(), false),
                signal_output(),
            ],
            Blueprint::Mix => vec![
                signal_input(FIRST, "A"),
                signal_input(SECOND, "B"),
                number_input(
                    BALANCE,
                    "Balance",
                    DEFAULT_BALANCE,
                    MIN_BALANCE,
                    MAX_BALANCE,
                ),
                signal_output(),
            ],
            Blueprint::Gain => vec![
                signal_input(INPUT, "Signal"),
                number_input(GAIN, "Gain", DEFAULT_GAIN, MIN_GAIN, MAX_GAIN),
                signal_output(),
            ],
            Blueprint::Smooth => vec![
                signal_input(INPUT, "Signal"),
                input_port(
                    WINDOW,
                    "Window",
                    Kind::Number,
                    PortInputType::constant_only(ConstantInputType::bounded_integer(
                        DEFAULT_WINDOW,
                        MIN_WINDOW,
                        MAX_WINDOW,
                    )),
                    false,
                ),
                signal_output(),
            ],
            Blueprint::Measure => vec![
                signal_input(INPUT, "Signal"),
                number_output(PEAK, "Peak"),
                number_output(RMS, "RMS"),
            ],
            Blueprint::Scope => vec![
                signal_input(INPUT, "Signal"),
                input_port(
                    TITLE,
                    "Title",
                    Kind::Text,
                    PortInputType::constant_only(ConstantInputType::String(
                        DEFAULT_TITLE.to_string(),
                    )),
                    false,
                ),
                output_port(
                    TRACE,
                    "Trace",
                    Kind::Signal,
                    PortOutputType::ConnectionOnly,
                    false,
                ),
                output_port(
                    LABEL,
                    "Label",
                    Kind::Text,
                    PortOutputType::ConnectionOnly,
                    false,
                ),
            ],
        }
    }

    fn width(&self) -> f32 {
        match self {
            Blueprint::Scope => SCOPE_NODE_WIDTH,
            _ => NODE_WIDTH,
        }
    }

    fn multithreading(&self) -> bool {
        matches!(self, Blueprint::Smooth)
    }

    fn categories(&self) -> Vec<&'static str> {
        match self {
            Blueprint::Oscillator => vec!["Sources"],
            Blueprint::Mix | Blueprint::Gain | Blueprint::Smooth => vec!["Processing"],
            Blueprint::Measure | Blueprint::Scope => vec!["Analysis"],
        }
    }

    fn compute(&mut self, state: &mut RuntimeState<Value>) -> anyhow::Result<ComputeState<Value>> {
        let outputs = match self {
            Blueprint::Oscillator => vec![(OUTPUT, oscillate(state)?)],
            Blueprint::Mix => vec![(OUTPUT, mix(state)?)],
            Blueprint::Gain => vec![(OUTPUT, gain(state)?)],
            Blueprint::Smooth => vec![(OUTPUT, smooth(state)?)],
            Blueprint::Measure => measure(state)?,
            Blueprint::Scope => vec![
                (TRACE, Value::Signal(signal(state, INPUT)?)),
                (LABEL, Value::Text(text(state, TITLE)?)),
            ],
        };

        Ok(ComputeState::done(
            outputs
                .into_iter()
                .map(|(name, value)| (name.to_string(), Arc::new(value)))
                .collect(),
        ))
    }

    fn port_for_viewer(&self) -> Option<String> {
        let port = match self {
            Blueprint::Measure => PEAK,
            Blueprint::Scope => TRACE,
            _ => OUTPUT,
        };

        Some(port.to_string())
    }

    fn prepare_custom_ui_data(&self, outputs: &HashMap<String, Arc<Value>>) -> GenericStore {
        let mut store = GenericStore::default();

        if let Some(Value::Signal(samples)) = outputs.get(TRACE).map(Arc::as_ref) {
            store.insert(SAMPLES_KEY, samples.clone());
        }

        if let Some(Value::Text(title)) = outputs.get(LABEL).map(Arc::as_ref) {
            store.insert(TITLE_KEY, title.clone());
        }

        store
    }

    fn custom_ui(&self, ui: &mut Ui, canvas: &Canvas, state: &mut GenericStore) {
        if matches!(self, Blueprint::Scope) {
            draw_scope(ui, canvas, state);
        }
    }

    fn custom_ui_height(&self) -> f32 {
        match self {
            Blueprint::Scope => SCOPE_HEIGHT,
            _ => DEFAULT_NUMBER,
        }
    }
}

fn number_input(
    name: &str,
    label: &str,
    default: f32,
    min_bound: f32,
    max_bound: f32,
) -> PortDefinition<Kind, Value> {
    input_port(
        name,
        label,
        Kind::Number,
        PortInputType::connection_and_constant(ConstantInputType::bounded_float(
            default, min_bound, max_bound,
        )),
        false,
    )
}

fn signal_input(name: &str, label: &str) -> PortDefinition<Kind, Value> {
    input_port(
        name,
        label,
        Kind::Signal,
        PortInputType::connection_only(),
        true,
    )
}

fn signal_output() -> PortDefinition<Kind, Value> {
    output_port(
        OUTPUT,
        "Signal",
        Kind::Signal,
        PortOutputType::ConnectionOnly,
        true,
    )
}

fn number_output(name: &str, label: &str) -> PortDefinition<Kind, Value> {
    output_port(
        name,
        label,
        Kind::Number,
        PortOutputType::ConnectionAndConstant { text_value: None },
        true,
    )
}

fn waveform_selection() -> PortInputType {
    let names: Vec<String> = Waveform::ALL
        .iter()
        .map(|waveform| waveform.name().to_string())
        .collect();

    PortInputType::constant_only(ConstantInputType::selection(names))
}

fn number(state: &RuntimeState<Value>, name: &str) -> anyhow::Result<f32> {
    match state.inputs_map.get(name)? {
        Value::Number(number) => Ok(*number),
        other => anyhow::bail!("Input {name} expects a number, got {other:?}"),
    }
}

fn signal(state: &RuntimeState<Value>, name: &str) -> anyhow::Result<Arc<[f32]>> {
    match state.inputs_map.get(name)? {
        Value::Signal(samples) => Ok(samples.clone()),
        other => anyhow::bail!("Input {name} expects a signal, got {other:?}"),
    }
}

fn text(state: &RuntimeState<Value>, name: &str) -> anyhow::Result<String> {
    match state.inputs_map.get(name)? {
        Value::Text(text) => Ok(text.clone()),
        other => anyhow::bail!("Input {name} expects text, got {other:?}"),
    }
}

fn oscillate(state: &RuntimeState<Value>) -> anyhow::Result<Value> {
    let frequency = number(state, FREQUENCY)?;
    let amplitude = number(state, AMPLITUDE)?;
    let waveform = Waveform::from_index(number(state, SHAPE)?);

    let samples = (0..SAMPLE_COUNT)
        .map(|index| {
            let time = index as f32 / SAMPLE_COUNT as f32;
            amplitude * waveform.sample(frequency * time)
        })
        .collect();

    Ok(Value::Signal(samples))
}

fn mix(state: &RuntimeState<Value>) -> anyhow::Result<Value> {
    let first = signal(state, FIRST)?;
    let second = signal(state, SECOND)?;
    let balance = number(state, BALANCE)?;

    let samples = first
        .iter()
        .zip(second.iter())
        .map(|(a, b)| a * (FULL_SCALE - balance) + b * balance)
        .collect();

    Ok(Value::Signal(samples))
}

fn gain(state: &RuntimeState<Value>) -> anyhow::Result<Value> {
    let input = signal(state, INPUT)?;
    let gain = number(state, GAIN)?;

    Ok(Value::Signal(
        input.iter().map(|sample| sample * gain).collect(),
    ))
}

fn smooth(state: &RuntimeState<Value>) -> anyhow::Result<Value> {
    let input = signal(state, INPUT)?;
    let window = number(state, WINDOW)? as usize;

    thread::sleep(SMOOTHING_DELAY);

    let samples = (0..input.len())
        .map(|index| {
            let start = index.saturating_sub(window);
            let slice = &input[start..=index];
            slice.iter().sum::<f32>() / slice.len() as f32
        })
        .collect();

    Ok(Value::Signal(samples))
}

fn measure(state: &RuntimeState<Value>) -> anyhow::Result<Vec<(&'static str, Value)>> {
    let input = signal(state, INPUT)?;
    let peak = input
        .iter()
        .fold(DEFAULT_NUMBER, |peak, sample| peak.max(sample.abs()));
    let mean_square = input.iter().map(|sample| sample * sample).sum::<f32>() / input.len() as f32;

    Ok(vec![
        (PEAK, Value::Number(peak)),
        (RMS, Value::Number(mean_square.sqrt())),
    ])
}

fn draw_scope(ui: &mut Ui, canvas: &Canvas, state: &mut GenericStore) {
    let size = vec2(ui.available_width(), canvas.zoomed(SCOPE_HEIGHT));
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect.shrink(canvas.zoomed(SCOPE_PADDING));

    painter.rect_filled(rect, canvas.zoomed(SCOPE_CORNER_RADIUS), SCOPE_BACKGROUND);
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(SCOPE_AXIS_WIDTH, SCOPE_AXIS),
    );

    let title = state.get::<String>(TITLE_KEY).cloned().unwrap_or_default();
    painter.text(
        rect.left_top() + vec2(canvas.zoomed(SCOPE_PADDING), canvas.zoomed(SCOPE_PADDING)),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(canvas.zoomed(SCOPE_TITLE_SIZE)),
        SCOPE_TEXT,
    );

    let Ok(samples) = state.get::<Arc<[f32]>>(SAMPLES_KEY) else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Focus to evaluate",
            FontId::proportional(canvas.zoomed(SCOPE_TITLE_SIZE)),
            SCOPE_TEXT,
        );
        return;
    };

    let last_index = samples.len().saturating_sub(1).max(1) as f32;
    let points: Vec<Pos2> = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let x = rect.left() + rect.width() * index as f32 / last_index;
            let normalized = (sample / SCOPE_RANGE).clamp(-HALF_CYCLE, HALF_CYCLE);
            pos2(x, rect.center().y - normalized * rect.height())
        })
        .collect();

    painter.line(
        points,
        Stroke::new(canvas.zoomed(SCOPE_TRACE_WIDTH), SCOPE_TRACE),
    );
}

#[derive(Clone, Default)]
struct FocusedOutput {
    value: Arc<Mutex<Option<Arc<Value>>>>,
}

impl FocusedOutput {
    fn describe(&self) -> String {
        let Ok(value) = self.value.lock() else {
            return "Unavailable".to_string();
        };

        match value.as_deref() {
            None => "Nothing focused yet".to_string(),
            Some(Value::Signal(samples)) => format!("Signal of {} samples", samples.len()),
            Some(other) => String::try_from(other).unwrap_or_default(),
        }
    }
}

impl GraphEventHandler<Value> for FocusedOutput {
    fn on_focused_output_change(&self, data: Option<Arc<Value>>) -> anyhow::Result<()> {
        let mut value = self
            .value
            .lock()
            .map_err(|_| anyhow::anyhow!("The focused output lock is poisoned"))?;
        *value = data;

        Ok(())
    }
}

type Editor = Graph<Kind, Value, Blueprint, FocusedOutput>;

struct SignalLab {
    editor: Editor,
    focused: FocusedOutput,
    status: String,
}

impl SignalLab {
    fn new() -> anyhow::Result<Self> {
        let focused = FocusedOutput::default();
        let editor = open_graph(DEMO_GRAPH, focused.clone())?;

        Ok(Self {
            editor,
            focused,
            status: "Demo graph loaded".to_string(),
        })
    }

    fn replace_graph(&mut self, source: &str, status: &str) {
        self.focused = FocusedOutput::default();

        match open_graph(source, self.focused.clone()) {
            Ok(editor) => {
                self.editor = editor;
                self.status = status.to_string();
            }
            Err(error) => self.status = format!("Could not open the graph: {error}"),
        }
    }

    fn handle_menu_event(&mut self, event: MenuEvent) {
        match event {
            MenuEvent::New => {
                self.focused = FocusedOutput::default();
                self.editor = Graph::new(
                    Options::default().with_multithreading(true),
                    self.focused.clone(),
                );
                self.status = "New empty graph".to_string();
            }
            MenuEvent::Save => {
                let saved = self
                    .editor
                    .snapshot()
                    .and_then(|snapshot| snapshot.to_ron())
                    .and_then(|ron| fs::write(SAVE_FILE, ron).map_err(anyhow::Error::from));

                self.status = match saved {
                    Ok(()) => format!("Saved to {SAVE_FILE}"),
                    Err(error) => format!("Could not save: {error}"),
                };
            }
            MenuEvent::Load => match fs::read_to_string(SAVE_FILE) {
                Ok(source) => self.replace_graph(&source, &format!("Loaded {SAVE_FILE}")),
                Err(error) => self.status = format!("Could not read {SAVE_FILE}: {error}"),
            },
        }
    }

    fn show_side_panel(&self, ui: &mut Ui) {
        ui.heading("Signal lab");
        ui.add_space(SCOPE_PADDING);
        ui.label("Right click the canvas to add nodes, drag between ports to connect them.");
        ui.label("Press a node's eye button to focus it: the graph evaluates it and everything it depends on.");
        ui.add_space(SCOPE_PADDING);
        ui.separator();
        ui.strong("Focused output");
        ui.monospace(self.focused.describe());
        ui.separator();
        ui.small(&self.status);
    }
}

impl eframe::App for SignalLab {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut reset = false;

        let menu_output = self
            .editor
            .show_menu_bar(ui, |ui| {
                ui.menu_button("Demo", |ui| {
                    reset = ui.button("Reset demo graph").clicked();
                });
            })
            .inner;

        if let Some(event) = menu_output.event {
            self.handle_menu_event(event);
        }

        if reset {
            self.replace_graph(DEMO_GRAPH, "Demo graph restored");
        }

        egui::Panel::right("details")
            .exact_size(SIDE_PANEL_WIDTH)
            .show(ui, |ui| self.show_side_panel(ui));

        egui::CentralPanel::default().show(ui, |ui| self.editor.show(ui));
    }
}

fn open_graph(source: &str, focused: FocusedOutput) -> anyhow::Result<Editor> {
    let snapshot = GraphSnapshot::from_ron(source)?;
    let options = Options::default().with_multithreading(true);

    Graph::from_snapshot(snapshot, options, focused)
}

fn main() -> eframe::Result {
    let app = SignalLab::new().map_err(|error| eframe::Error::AppCreation(error.into()))?;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size(WINDOW_SIZE),
        ..Default::default()
    };

    eframe::run_native(APP_NAME, native_options, Box::new(|_| Ok(Box::new(app))))
}
