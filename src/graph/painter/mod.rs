//! Drawing of nodes, ports and connections.

use std::collections::HashMap;
use std::fmt::Debug;

use egui::containers::menu::{MenuBar, MenuConfig};
use egui::epaint::RectShape;
use egui::layers::ShapeIdx;
use egui::{Align, Align2, Area, AsIdSalt, Button, Checkbox, Color32, ComboBox, CornerRadius, DragValue, FontId, Frame, Id, InnerResponse, IntoAtoms, Label, Layout, Margin, Panel, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::NodeBlueprintDefinition;
use crate::graph::canvas::Canvas;
use crate::graph::connection::DragConnection;
use crate::graph::node::port::{
    ConstantInputType, Port, PortId, PortInputType, PortLinkageType, PortOutputType, PortTypeInfo,
};
use crate::graph::node::{DataValidityState, NODE_MINIMUM_SIZE, Node};
use crate::graph::painter::style::{PainterTheme, StylePainter, UNZOOMED};
use crate::graph::{NodeState, PortState};
use crate::helpers::{bezier_controls, cubic_bezier, grid_line_positions, random_in_range};
use crate::options::Options;

pub(crate) mod style;

const MENU_BAR_PANEL_ID: &str = "graph_menu_bar";
const CANVAS_UI_ID: &str = "graph_canvas";

const PORT_INPUT_BUFFER_SALT: &str = "port_input_buffer";
const CUSTOM_UI_SALT: &str = "node_custom_ui";
const DEBUG_STROKE_WIDTH: f32 = 1.0;
const FLOAT_DECIMALS: usize = 3;
const INTEGER_DECIMALS: usize = 0;
const INTEGER_DRAG_SPEED: f64 = 1.0;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(super) enum NodeSelectedState {
    Selected,
    NotSelected,
}

#[derive(Debug, Clone)]
pub(super) struct Painter {
    pub(super) style_painter: StylePainter,
    options: Options,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum PortSide {
    Input,
    Output,
    Parameter,
    InlineOutput,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(super) enum PanelPlacement {
    Top,
    Bottom,
    Left,
    Right,
}

impl Painter {
    pub(super) fn new(options: Options, theme: impl Into<PainterTheme>) -> Self {
        Self {
            style_painter: StylePainter::new(theme),
            options,
        }
    }

    pub(super) fn theme(&self) -> PainterTheme {
        *self.style_painter.theme()
    }

    pub(super) fn show_scoped<R>(
        &self,
        ui: &mut Ui,
        zoom: f32,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        self.style_painter.scoped(ui, zoom, add_contents)
    }

    pub(super) fn scoped_child(&self, ui: &mut Ui, zoom: f32) -> Ui {
        let style = self.style_painter.zoomed_style(ui.style(), zoom);

        ui.new_child(
            UiBuilder::new()
                .id_salt(CANVAS_UI_ID)
                .style(style)
                .layout(*ui.layout()),
        )
    }

    pub(super) fn show_menu_bar<R>(
        &self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let menu_style = self.theme().menu_style_modifier();

        Panel::top(MENU_BAR_PANEL_ID)
            .frame(self.show_panel_frame())
            .show(ui, |ui| {
                self.show_scoped(ui, UNZOOMED, |ui| {
                    MenuBar::new()
                        .style(menu_style.clone())
                        .config(MenuConfig::new().style(menu_style))
                        .ui(ui, add_contents)
                        .inner
                })
            })
    }

    pub(super) fn show_panel<R>(
        &self,
        ui: &mut Ui,
        placement: PanelPlacement,
        id: impl AsIdSalt,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let panel = match placement {
            PanelPlacement::Top => Panel::top(id),
            PanelPlacement::Bottom => Panel::bottom(id),
            PanelPlacement::Left => Panel::left(id),
            PanelPlacement::Right => Panel::right(id),
        };

        panel
            .frame(self.show_panel_frame())
            .show(ui, |ui| self.show_scoped(ui, UNZOOMED, add_contents))
    }

    pub(super) fn show_panel_frame(&self) -> Frame {
        Frame::new()
            .fill(self.theme().panel_background_color)
            .stroke(self.theme().scaled_border_stroke(UNZOOMED))
            .inner_margin(Margin::from(self.theme().panel_padding))
    }

    pub(super) fn show_grid(&self, ui: &Ui, canvas: &Canvas, rect: Rect) {
        let theme = self.theme();
        let stroke = Stroke::new(theme.grid_thickness, theme.grid_color);
        let world_min = canvas.screen_to_world_coordinates(rect.min);
        let world_max = canvas.screen_to_world_coordinates(rect.max);
        let painter = ui.painter();

        for x in grid_line_positions(world_min.x, world_max.x, theme.grid_spacing) {
            let screen_x = canvas.world_to_screen_coordinates(pos2(x, world_min.y)).x;
            painter.line_segment(
                [pos2(screen_x, rect.min.y), pos2(screen_x, rect.max.y)],
                stroke,
            );
        }

        for y in grid_line_positions(world_min.y, world_max.y, theme.grid_spacing) {
            let screen_y = canvas.world_to_screen_coordinates(pos2(world_min.x, y)).y;
            painter.line_segment(
                [pos2(rect.min.x, screen_y), pos2(rect.max.x, screen_y)],
                stroke,
            );
        }
    }

    pub(super) fn show_temporary_connection(
        &self,
        ui: &mut Ui,
        canvas: &Canvas,
        from_point: Pos2,
        to_point: Pos2,
    ) {
        ui.painter().extend(self.draw_bezier(
            canvas,
            from_point,
            to_point,
            Stroke::new(
                canvas.zoomed(self.theme().temporary_connection_thickness),
                self.theme().temporary_connection_color,
            ),
            true,
        ));
    }

    pub(super) fn show_connection(
        &self,
        ui: &mut Ui,
        canvas: &Canvas,
        curve_shape_idx: ShapeIdx,
        from_point: Pos2,
        to_point: Pos2,
        selected: bool,
    ) -> Rect {
        let (curve_shape, handle_rect) = self.draw_bezier_with_handle(
            canvas,
            from_point,
            to_point,
            Stroke::new(
                canvas.zoomed(if selected {
                    self.theme().connection_selected_thickness
                } else {
                    self.theme().connection_thickness
                }),
                self.theme().connection_color,
            ),
            false,
        );

        ui.painter().set(curve_shape_idx, curve_shape);

        handle_rect
    }

    pub(super) fn show_connection_handle(
        &self,
        ui: &mut Ui,
        canvas: &Canvas,
        handle_rect: Rect,
        handle_response: Response,
        handle_shape_idx: ShapeIdx,
        selected: bool,
    ) {
        let radius: f32 = if handle_response.hovered() || selected {
            canvas.zoomed(self.theme().port_radius_hover)
        } else {
            canvas.zoomed(self.theme().port_radius)
        };

        ui.painter().set(
            handle_shape_idx,
            Shape::circle_filled(handle_rect.center(), radius, self.theme().connection_color),
        );
    }

    pub fn draw_bezier(
        &self,
        canvas: &Canvas,
        start: Pos2,
        end: Pos2,
        stroke: Stroke,
        dashed_line: bool,
    ) -> Vec<Shape> {
        let (control1, control2) = bezier_controls(start, end);

        let mut points = Vec::with_capacity(33);

        for i in 0..=32 {
            let t = i as f32 / 32.0;
            points.push(cubic_bezier(start, control1, control2, end, t));
        }

        if dashed_line {
            Shape::dashed_line(&points, stroke, canvas.zoomed(3.0), canvas.zoomed(6.0))
        } else {
            vec![Shape::line(points, stroke)]
        }
    }

    pub fn draw_bezier_with_handle(
        &self,
        canvas: &Canvas,
        start: Pos2,
        end: Pos2,
        stroke: Stroke,
        dashed_line: bool,
    ) -> (Vec<Shape>, Rect) {
        let (control1, control2) = bezier_controls(start, end);

        let mut points = Vec::with_capacity(33);
        let mut handle_center: Pos2 = Pos2::ZERO;

        for i in 0..=32 {
            let t = i as f32 / 32.0;
            let point = cubic_bezier(start, control1, control2, end, t);
            if i == 16 {
                handle_center = point;
            }
            points.push(point);
        }

        let handle_rect = Rect::from_center_size(
            handle_center,
            vec2(
                canvas.zoomed(self.theme().port_radius) * 1.5,
                canvas.zoomed(self.theme().port_radius) * 1.5,
            ),
        );

        if dashed_line {
            (
                Shape::dashed_line(&points, stroke, canvas.zoomed(3.0), canvas.zoomed(6.0)),
                handle_rect,
            )
        } else {
            (vec![Shape::line(points, stroke)], handle_rect)
        }
    }

    pub(super) fn show_context_menu_area(&self, id: &str, position: Pos2) -> Area {
        Area::new(Id::unique(id))
            .fixed_pos(position)
            .order(egui::Order::Foreground)
    }

    pub(super) fn show_context_submenu_area(&self, id: &str, position: Pos2) -> Area {
        Area::new(Id::unique(id))
            .fixed_pos(pos2(
                position.x + self.theme().menu_button_size.x,
                position.y,
            ))
            .order(egui::Order::Foreground)
    }

    pub(super) fn show_context_menu_frame(&self) -> Frame {
        Frame::new()
            .fill(self.theme().node_background_color)
            .stroke(self.theme().scaled_border_stroke(UNZOOMED))
            .corner_radius(self.theme().scaled_corner_radius(UNZOOMED))
            .inner_margin(Margin::ZERO)
    }

    pub(super) fn show_context_menu_button<'a>(&self, atoms: impl IntoAtoms<'a>) -> Button<'a> {
        Button::new(atoms)
            .fill(self.theme().node_background_color)
            .stroke(self.theme().scaled_border_stroke(UNZOOMED))
            .corner_radius(self.theme().scaled_corner_radius(UNZOOMED))
            .min_size(self.theme().menu_button_size)
    }

    pub(crate) fn show_node<P, D, K>(
        &mut self,
        ui: &mut Ui,
        canvas: &Canvas,
        node: &mut Node<P, D, K>,
        selected_state: NodeSelectedState,
        validity_state: DataValidityState,
        dragged_connection: &Option<DragConnection<P>>,
    ) -> (NodeState, Option<PortId>)
    where
        P: PortTypeInfo + Eq + Copy + Clone + Debug,
        D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
        for<'a> &'a D: TryInto<String>,
        K: NodeBlueprintDefinition<P, D>,
    {
        let node_screen_position = canvas.world_to_screen_coordinates(node.position);
        let size = NODE_MINIMUM_SIZE.max(vec2(node.width(), ui.available_height()));
        log::trace!(
            "[{:?}] Drawing at position {:?} from world position {:?}",
            node.id,
            node_screen_position,
            node.position
        );

        let mut child_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    node_screen_position,
                    canvas.zoomed(size),
                ))
                .layout(*ui.layout())
                .scope_id(ui.scope_id().with(node.id)),
        );

        let (states, hovered_port) = self.show_node_content(
            &mut child_ui,
            canvas,
            node,
            selected_state,
            validity_state,
            dragged_connection,
        );

        (states, hovered_port)
    }

    fn show_node_content<P, D, K>(
        &mut self,
        ui: &mut Ui,
        canvas: &Canvas,
        node: &mut Node<P, D, K>,
        selected_state: NodeSelectedState,
        validity_state: DataValidityState,
        dragged_connection: &Option<DragConnection<P>>,
    ) -> (NodeState, Option<PortId>)
    where
        P: PortTypeInfo + Eq + Copy + Clone + Debug,
        D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
        for<'a> &'a D: TryInto<String>,
        K: NodeBlueprintDefinition<P, D>,
    {
        let node_id = node.id;
        let mut hovered_port = None;
        let mut focus_button_response = None;
        let mut outer_rect = ui.available_rect_before_wrap();
        let outer_rect_id = ui.scope_id().with((node_id, "window"));
        let mut ports_states: HashMap<PortId, PortState> = HashMap::new();
        let mut selected_box_id: Option<ShapeIdx> = None;

        // This is here to pre-allocate the outer_rect in a layer below the ports inputs, later on
        // there will be another call to ui.interact which will update the rect
        let _ = ui.interact(outer_rect, outer_rect_id, Sense::empty());

        if self.options.debug_painter {
            ui.painter().rect_stroke(
                outer_rect,
                CornerRadius::ZERO,
                Stroke::new(1.0, Color32::RED),
                StrokeKind::Outside,
            );
        }

        if selected_state == NodeSelectedState::Selected {
            selected_box_id = Some(ui.painter().add(Shape::Noop));
        }

        ui.vertical(|ui| {
            let header_rect = Rect::from_min_size(
                ui.available_rect_before_wrap().min,
                vec2(
                    ui.available_width(),
                    canvas.zoomed(self.theme().node_header_height),
                ),
            );

            let bg_shape_id = ui.painter().add(Shape::Noop);

            ui.painter().rect_filled(
                header_rect,
                CornerRadius::ZERO,
                self.theme().node_highlight_color,
            );

            ui.horizontal(|ui| {
                ui.add_sized(
                    vec2(
                        ui.available_width(),
                        canvas.zoomed(self.theme().node_header_height),
                    ),
                    Label::new(
                        RichText::new(&node.name)
                            .size(canvas.zoomed(self.theme().font_size))
                            .color(self.theme().font_color)
                            .background_color(self.theme().node_highlight_color),
                    )
                        .selectable(false),
                );
            });

            let margin = canvas.zoomed(vec2(10.0, 5.0));

            let ports_rect = ui.available_rect_before_wrap().shrink2(margin);

            let input_rect = Rect::from_min_size(
                ports_rect.min,
                vec2(ports_rect.size().x / 2.0, ports_rect.size().y),
            );

            let (input_rect, inputs_states, hovered_input_port) = self.show_node_ports(
                ui,
                input_rect,
                canvas,
                node,
                PortSide::Input,
                dragged_connection,
            );

            ports_states.extend(inputs_states);

            if self.options.debug_painter {
                ui.painter().rect_stroke(
                    input_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, Color32::YELLOW),
                    StrokeKind::Middle,
                );
            }

            let output_rect = Rect::from_min_size(
                pos2(input_rect.max.x, ports_rect.min.y),
                vec2(ports_rect.size().x / 2.0, ports_rect.size().y),
            );

            let (output_rect, outputs_states, hovered_output_port) = self.show_node_ports(
                ui,
                output_rect,
                canvas,
                node,
                PortSide::Output,
                dragged_connection,
            );

            ports_states.extend(outputs_states);

            if self.options.debug_painter {
                ui.painter().rect_stroke(
                    output_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, Color32::ORANGE),
                    StrokeKind::Middle,
                );
            }

            let output_rect = Rect::from_min_size(
                pos2(input_rect.max.x, output_rect.bottom()),
                vec2(ports_rect.size().x / 2.0, ports_rect.size().y),
            );

            let (output_rect, outputs_states, _) = self.show_node_ports(
                ui,
                output_rect,
                canvas,
                node,
                PortSide::InlineOutput,
                dragged_connection,
            );

            ports_states.extend(outputs_states);

            if self.options.debug_painter {
                ui.painter().rect_stroke(
                    output_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, Color32::ORANGE),
                    StrokeKind::Middle,
                );
            }

            let parameters_rect = Rect::from_min_size(
                pos2(
                    input_rect.min.x,
                    input_rect.bottom().max(output_rect.bottom()),
                ),
                vec2(ports_rect.size().x, ui.available_height()),
            );

            let (parameters_rect, parameters_states, _) = self.show_node_ports(
                ui,
                parameters_rect,
                canvas,
                node,
                PortSide::Parameter,
                dragged_connection,
            );

            ports_states.extend(parameters_states);

            if self.options.debug_painter {
                ui.painter().rect_stroke(
                    parameters_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, Color32::PURPLE),
                    StrokeKind::Middle,
                );
            }

            let custom_ui_rect = self.show_custom_ui(ui, canvas, parameters_rect, margin, node);

            let mut bottom_bar_rect = Rect::from_min_size(
                custom_ui_rect.left_bottom() + vec2(0.0, margin.y),
                vec2(custom_ui_rect.size().x, ui.available_height()),
            );

            if self.options.show_focus_button {
                (bottom_bar_rect, focus_button_response) =
                    self.show_focus_button(ui, bottom_bar_rect, canvas, node);
            }

            outer_rect.set_bottom(bottom_bar_rect.bottom() + margin.y);

            ui.painter().set(
                bg_shape_id,
                RectShape::filled(
                    outer_rect,
                    CornerRadius::ZERO,
                    match validity_state {
                        DataValidityState::Error { .. } => self.theme().node_error_background_color,
                        _ => self.theme().node_background_color,
                    },
                ),
            );

            hovered_port = hovered_input_port.or(hovered_output_port);
        });

        if let Some(selected_box_id) = selected_box_id {
            let selected_rect =
                outer_rect.expand(canvas.zoomed(self.theme().node_selected_border_distance));

            ui.painter().set(
                selected_box_id,
                Shape::rect_stroke(
                    selected_rect,
                    CornerRadius::ZERO,
                    Stroke::new(
                        canvas.zoomed(self.theme().node_selected_border_thickness),
                        self.theme().node_selected_border_color,
                    ),
                    StrokeKind::Outside,
                ),
            );
        }

        (
            NodeState {
                rect: outer_rect,
                response: ui.interact(outer_rect, outer_rect_id, Sense::click_and_drag()),
                ports_states,
                focus_button_response,
            },
            hovered_port,
        )
    }

    fn show_node_ports<P, D, K>(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        canvas: &Canvas,
        node: &mut Node<P, D, K>,
        port_side: PortSide,
        dragged_connection: &Option<DragConnection<P>>,
    ) -> (Rect, HashMap<PortId, PortState>, Option<PortId>)
    where
        P: PortTypeInfo + Eq + Copy + Clone + Debug,
        D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
        for<'a> &'a D: TryInto<String>,
        K: NodeBlueprintDefinition<P, D>,
    {
        let margin: f32 = canvas.zoomed(10.0);
        let mut hovered_port: Option<PortId> = None;
        let iter: Box<dyn Iterator<Item=&Port<P>>>;
        let layout: Layout;
        let mut states: HashMap<PortId, PortState> = HashMap::new();

        match port_side {
            PortSide::Input => {
                iter = Box::new(node.input_ports());
                layout = Layout::left_to_right(Align::Center);
            }
            PortSide::Output => {
                iter = Box::new(node.output_ports());
                layout = Layout::right_to_left(Align::Center);
            }
            PortSide::Parameter => {
                iter = Box::new(node.input_parameters());
                layout = Layout::left_to_right(Align::Center);
            }
            PortSide::InlineOutput => {
                iter = Box::new(node.output_inline_ports());
                layout = Layout::right_to_left(Align::Center);
            }
        }

        let mut current_rect = Rect::from_min_size(rect.min, vec2(rect.size().x, 0.0));

        let mut max_label_width: f32 = 0.0;

        for port in iter {
            if !port.visible {
                continue;
            }

            current_rect
                .extend_with_y(current_rect.max.y + canvas.zoomed(self.theme().port_height));

            let port_text = match port.linkage_type {
                PortLinkageType::Output(PortOutputType::ConnectionOnly)
                | PortLinkageType::Input(_) => &port.label,
                PortLinkageType::Output(PortOutputType::ConstantOnly { ref text_value })
                | PortLinkageType::Output(PortOutputType::ConnectionAndConstant {
                                              ref text_value,
                                          }) => {
                    if let Some(text_value) = text_value {
                        if text_value.is_empty() {
                            &port.label
                        } else {
                            &format!("{}: {}", port.label, text_value)
                        }
                    } else {
                        &port.label
                    }
                }
            };

            ui.scope_builder(
                UiBuilder::new().max_rect(current_rect).layout(layout),
                |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(port_text)
                                .color(self.theme().font_color)
                                .background_color(Color32::TRANSPARENT)
                                .size(canvas.zoomed(self.theme().font_size_ports)),
                        )
                            .selectable(false)
                            .halign(Align::Center),
                    );
                    ui.add_space(canvas.zoomed(2.0));

                    max_label_width = max_label_width.max(ui.min_size().x);

                    states.insert(
                        port.id,
                        PortState {
                            color: port.port_type.color(),
                            rect: if port_side != PortSide::Parameter
                                && port_side != PortSide::InlineOutput
                            {
                                Some(current_rect)
                            } else {
                                None
                            },
                            connections_states: HashMap::new(),
                            response: None,
                            input_response: None,
                        },
                    );
                },
            );
            ui.add_space(canvas.zoomed(2.0));

            current_rect = Rect::from_min_size(
                pos2(
                    current_rect.min.x,
                    current_rect.min.y + canvas.zoomed(self.theme().port_height),
                ),
                vec2(rect.size().x, 0.0),
            );
            log::trace!("[{:?}] Drawing with rect {:?}", port.id, current_rect);
        }

        if port_side != PortSide::Parameter && port_side != PortSide::InlineOutput {
            for (port_id, port_state) in states.iter_mut() {
                let dot_rect = Rect::from_center_size(
                    if port_side == PortSide::Input {
                        port_state.rect.unwrap().left_center() - vec2(margin, 0.0)
                    } else {
                        port_state.rect.unwrap().right_center() + vec2(margin, 0.0)
                    },
                    vec2(
                        canvas.zoomed(self.theme().port_radius) * 1.5,
                        canvas.zoomed(self.theme().port_radius) * 1.5,
                    ),
                );

                if let Some(dragged_connection) = dragged_connection
                    && dragged_connection.from_port == *port_id
                    && let Some(pointer_position) = ui.pointer_interact_pos() {
                    self.show_temporary_connection(
                        ui,
                        canvas,
                        dragged_connection.from_position,
                        pointer_position,
                    );
                }

                port_state.rect = Some(dot_rect);
                // Using ui.interact in place of ui.allocate_rect as the latter caused a somewhat
                // rare bug in which sometimes the wrong response was triggered
                port_state.response = Some(ui.interact(
                    dot_rect,
                    ui.scope_id().with((node.id, port_id, "port")),
                    Sense::click_and_drag(),
                ));

                let radius: f32;

                if ui.rect_contains_pointer(dot_rect) {
                    hovered_port = Some(*port_id);
                    match dragged_connection {
                        None => {
                            radius = canvas.zoomed(self.theme().port_radius_hover);
                        }
                        Some(dragged_connection) => {
                            if node.ports()[port_id].is_connection_compatible(dragged_connection)
                            {
                                radius = canvas.zoomed(self.theme().port_radius_hover);
                            } else {
                                radius = canvas.zoomed(self.theme().port_radius);
                            }
                        }
                    }
                } else {
                    radius = canvas.zoomed(self.theme().port_radius);
                }

                ui.painter()
                    .circle_filled(dot_rect.center(), radius, port_state.color);
            }
        }

        if port_side == PortSide::Output || port_side == PortSide::InlineOutput {
            return (
                Rect::from_min_max(rect.min, pos2(current_rect.max.x, current_rect.max.y)),
                states,
                hovered_port,
            );
        }

        current_rect = Rect::from_min_size(
            pos2(rect.min.x + margin + max_label_width, rect.min.y),
            vec2(rect.size().x - (margin + max_label_width), 0.0),
        );

        let constants_enabled: HashMap<PortId, bool> = node
            .ports()
            .keys()
            .map(|port_id| (*port_id, node.is_constant_enabled(port_id)))
            .collect();

        let iter: Box<dyn Iterator<Item=&mut Port<P>>> = match port_side {
            PortSide::Input => Box::new(node.input_ports_mut()),
            PortSide::Output => Box::new(node.output_ports_mut()),
            PortSide::Parameter => Box::new(node.input_parameters_mut()),
            PortSide::InlineOutput => Box::new(node.output_inline_ports_mut()),
        };

        for port in iter {
            let constant_enabled = constants_enabled.get(&port.id).copied().unwrap_or_default();

            current_rect
                .extend_with_y(current_rect.max.y + canvas.zoomed(self.theme().port_height));

            let input_response = ui
                .scope_builder(
                    UiBuilder::new().max_rect(current_rect).layout(layout),
                    |ui| match port.linkage_type {
                        PortLinkageType::Input(PortInputType::ConnectionOnly { .. })
                        | PortLinkageType::Output(_) => None,
                        PortLinkageType::Input(_) => {
                            self.show_port_input(ui, canvas, port, constant_enabled)
                        }
                    },
                )
                .inner;

            if let Some(state) = states.get_mut(&port.id) {
                state.input_response = input_response;
            }

            ui.add_space(canvas.zoomed(2.0));

            current_rect = Rect::from_min_size(
                pos2(
                    current_rect.min.x,
                    current_rect.min.y + canvas.zoomed(self.theme().port_height),
                ),
                vec2(rect.size().x - (margin + max_label_width), 0.0),
            );
        }

        (
            Rect::from_min_max(rect.min, current_rect.max),
            states,
            hovered_port,
        )
    }

    fn show_port_input<P: PortTypeInfo + Eq + Copy + Clone + Debug>(
        &mut self,
        ui: &mut Ui,
        canvas: &Canvas,
        port: &mut Port<P>,
        enabled: bool,
    ) -> Option<Response> {
        const DRAG_SPEED: f64 = 1.0;

        fn show(
            ui: &mut Ui,
            canvas: &Canvas,
            style_painter: &StylePainter,
            id: Id,
            constant_type: &mut ConstantInputType,
            enabled: bool,
        ) -> bool {
            let inline_gap = canvas.zoomed(style_painter.theme().port_spacing).round();
            let zoom = canvas.current_zoom();

            match constant_type {
                ConstantInputType::Float {
                    value,
                    min_bound,
                    max_bound,
                    scale_display: scale,
                } => {
                    let range = *min_bound..=*max_bound;
                    let scale = f64::from(*scale);

                    edit_deferred(ui, id, value, |ui, value| {
                        style_painter.scoped(ui, zoom, |ui| {
                            ui.add_enabled(enabled, |ui: &mut Ui| {
                                ui.add_sized(
                                    ui.available_size(),
                                    DragValue::new(value)
                                        .range(range)
                                        .speed(DRAG_SPEED * scale)
                                        .update_while_editing(false)
                                        .fixed_decimals(FLOAT_DECIMALS)
                                        .custom_formatter(move |raw, decimals| {
                                            egui::emath::format_with_decimals_in_range(
                                                raw / scale,
                                                decimals,
                                            )
                                        })
                                        .custom_parser(move |text| {
                                            text.trim()
                                                .parse::<f64>()
                                                .ok()
                                                .map(|shown| shown * scale)
                                        }),
                                )
                            })
                        })
                    })
                }
                ConstantInputType::Integer {
                    value,
                    min_bound,
                    max_bound,
                    steps,
                } => {
                    let range = *min_bound..=*max_bound;

                    edit_deferred(ui, id, value, |ui, value| {
                        style_painter.scoped(ui, zoom, |ui| {
                            ui.add_enabled(enabled, |ui: &mut Ui| {
                                ui.add_sized(
                                    ui.available_size(),
                                    DragValue::new(value)
                                        .range(range)
                                        .update_while_editing(false)
                                        .fixed_decimals(INTEGER_DECIMALS)
                                        .speed(INTEGER_DRAG_SPEED * *steps as f64),
                                )
                            })
                        })
                    })
                }
                ConstantInputType::Path(value) => {
                    let font_size = canvas.zoomed(style_painter.theme().font_size_ports);

                    edit_deferred(ui, id, value, |ui, value| {
                        ui.add_enabled(enabled, |ui: &mut Ui| {
                            ui.add_sized(
                                ui.available_size(),
                                TextEdit::singleline(value)
                                    .font(FontId::proportional(font_size))
                                    .desired_width(ui.available_width())
                                    .align(Align2::new(Align::Center, Align::Center)),
                            )
                        })
                    })
                }
                ConstantInputType::Selection { options, selected } => {
                    let mut committed = false;

                    style_painter.scoped(ui, zoom, |ui| {
                        ui.add_enabled(enabled, |ui: &mut Ui| {
                            let selected_text = options.get(*selected).map_or("", String::as_str);

                            let inner = ComboBox::from_id_salt(id)
                                .popup_style(style_painter.theme().style_modifier(zoom))
                                .selected_text(selected_text)
                                .width(ui.available_width())
                                .show_ui(ui, |ui| {
                                    let mut changed = false;
                                    for (index, option) in options.iter().enumerate() {
                                        changed |=
                                            ui.selectable_value(selected, index, option).changed();
                                    }
                                    changed
                                });

                            committed = inner.inner.unwrap_or(false);
                            inner.response
                        })
                    });

                    committed
                }
                ConstantInputType::OptionalFloat {
                    value_option: value,
                    min_bound,
                    max_bound,
                } => {
                    let range = *min_bound..=*max_bound;
                    let mut committed = false;

                    style_painter.scoped(ui, zoom, |ui| {
                        inline_row(ui, inline_gap, |ui| {
                            ui.spacing_mut().interact_size.y = ui.spacing().icon_width;

                            committed |= ui
                                .add_enabled(enabled, Checkbox::without_text(&mut value.enabled))
                                .changed();

                            let drag_enabled = enabled && value.enabled;

                            committed |= edit_deferred(ui, id, &mut value.value, |ui, value| {
                                ui.add_enabled(drag_enabled, |ui: &mut Ui| {
                                    ui.add_sized(
                                        ui.available_size(),
                                        DragValue::new(value)
                                            .range(range)
                                            .update_while_editing(false)
                                            .fixed_decimals(FLOAT_DECIMALS),
                                    )
                                })
                            });
                        })
                            .response
                    });

                    committed
                }
                ConstantInputType::OptionalInteger {
                    value_option: value,
                    min_bound,
                    max_bound,
                } => {
                    let range = *min_bound..=*max_bound;
                    let mut committed = false;

                    style_painter.scoped(ui, zoom, |ui| {
                        inline_row(ui, inline_gap, |ui| {
                            ui.spacing_mut().interact_size.y = ui.spacing().icon_width;

                            committed |= ui
                                .add_enabled(enabled, Checkbox::without_text(&mut value.enabled))
                                .changed();

                            let drag_enabled = enabled && value.enabled;

                            committed |= edit_deferred(ui, id, &mut value.value, |ui, value| {
                                ui.add_enabled(drag_enabled, |ui: &mut Ui| {
                                    ui.add_sized(
                                        ui.available_size(),
                                        DragValue::new(value)
                                            .range(range)
                                            .update_while_editing(false)
                                            .fixed_decimals(INTEGER_DECIMALS)
                                            .speed(INTEGER_DRAG_SPEED),
                                    )
                                })
                            });
                        })
                            .response
                    });

                    committed
                }
                ConstantInputType::RandomizableUnsignedInteger {
                    value,
                    min_bound,
                    max_bound,
                } => {
                    let (min, max) = (*min_bound, *max_bound);
                    let mut committed = false;

                    style_painter.scoped(ui, zoom, |ui| {
                        inline_row(ui, inline_gap, |ui| {
                            let button_side = ui.available_height();
                            let drag_width =
                                (ui.available_width() - button_side - ui.spacing().item_spacing.x)
                                    .max(0.0);
                            let drag_size = vec2(drag_width, button_side);

                            committed |= edit_deferred(ui, id, value, |ui, value| {
                                ui.add_enabled(enabled, |ui: &mut Ui| {
                                    ui.add_sized(
                                        drag_size,
                                        DragValue::new(value)
                                            .range(min..=max)
                                            .update_while_editing(false)
                                            .fixed_decimals(INTEGER_DECIMALS)
                                            .speed(INTEGER_DRAG_SPEED),
                                    )
                                })
                            });

                            let randomize = ui.add_enabled(enabled, |ui: &mut Ui| {
                                ui.add_sized(Vec2::splat(button_side), Button::new(RANDOMIZE_ICON))
                            });

                            if randomize.clicked() {
                                *value = random_in_range(&mut rand::rng(), min, max);
                                ui.data_mut(|data| data.remove_temp::<u32>(id));
                                committed = true;
                            }
                        })
                            .response
                    });

                    committed
                }
                ConstantInputType::Boolean { value, .. } => {
                    let response = style_painter.scoped(ui, zoom, |ui| {
                        ui.add_enabled(enabled, |ui: &mut Ui| {
                            ui.add_sized(ui.available_size(), Checkbox::without_text(value))
                        })
                    });

                    response.changed()
                }
                ConstantInputType::String(value) => {
                    let font_size = canvas.zoomed(style_painter.theme().font_size_ports);

                    edit_deferred(ui, id, value, |ui, value| {
                        ui.add_enabled(enabled, |ui: &mut Ui| {
                            ui.add_sized(
                                ui.available_size(),
                                TextEdit::singleline(value)
                                    .font(FontId::proportional(font_size))
                                    .desired_width(ui.available_width())
                                    .align(Align2::new(Align::Center, Align::Center)),
                            )
                        })
                    })
                }
            }
        }

        let id = ui.scope_id().with((port.id, PORT_INPUT_BUFFER_SALT));
        let style_painter = &self.style_painter;

        let constant_type = match port.linkage_type {
            PortLinkageType::Input(ref mut input_type) => {
                input_type.constant_type_mut()?
            }
            PortLinkageType::Output(_) => return None,
        };

        let InnerResponse {
            inner: committed,
            mut response,
        } = ui.scope(|ui| show(ui, canvas, style_painter, id, constant_type, enabled));

        if committed {
            response.mark_changed();
        }

        Some(response)
    }

    fn show_custom_ui<P, D, K>(
        &self,
        ui: &mut Ui,
        canvas: &Canvas,
        above: Rect,
        margin: Vec2,
        node: &mut Node<P, D, K>,
    ) -> Rect
    where
        P: PortTypeInfo + Eq + Copy + Clone + Debug,
        D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
        for<'a> &'a D: TryInto<String>,
        K: NodeBlueprintDefinition<P, D>,
    {
        let available_rect = Rect::from_min_max(
            above.left_bottom() + vec2(0.0, margin.y),
            pos2(above.right(), ui.max_rect().bottom()),
        );

        let content_rect = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(available_rect)
                    .scope_id(Id::unique((CUSTOM_UI_SALT, node.id))),
                |ui| {
                    ui.set_clip_rect(available_rect.intersect(ui.clip_rect()));
                    node.show_custom_ui(ui, canvas);
                },
            )
            .response
            .rect;

        let rect = Rect::from_min_max(
            available_rect.min,
            pos2(
                available_rect.right(),
                content_rect
                    .bottom()
                    .max(available_rect.min.y + canvas.zoomed(node.custom_ui_height())),
            ),
        );

        if self.options.debug_painter {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::ZERO,
                Stroke::new(DEBUG_STROKE_WIDTH, Color32::GREEN),
                StrokeKind::Middle,
            );
        }

        rect
    }

    fn show_focus_button<P, D, K>(
        &self,
        ui: &mut Ui,
        mut rect: Rect,
        canvas: &Canvas,
        node: &mut Node<P, D, K>,
    ) -> (Rect, Option<Response>)
    where
        P: PortTypeInfo + Eq + Copy + Clone + Debug,
        D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
        for<'a> &'a D: TryInto<String>,
        K: NodeBlueprintDefinition<P, D>,
    {
        rect = Rect::from_min_size(rect.min, vec2(rect.width(), canvas.zoomed(20.0)));
        let mut response: Option<Response> = None;

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                let mut button_response = ui.add(
                    Button::selectable(
                        node.focused,
                        RichText::new("  👁  ")
                            .size(canvas.zoomed(self.theme().font_size_ports))
                            .color(self.theme().font_color)
                            .background_color(Color32::TRANSPARENT),
                    )
                        .stroke(Stroke::new(
                            canvas.zoomed(self.theme().textbox_border_thickness),
                            self.theme().textbox_border_color,
                        ))
                        .frame_when_inactive(true),
                );

                if button_response.clicked() {
                    node.focused = !node.focused;
                    button_response.mark_changed();
                }

                response = Some(button_response);
            },
        );

        (rect, response)
    }
}

const RANDOMIZE_ICON: &str = "🎲";

fn edit_deferred<T, W>(ui: &mut Ui, id: Id, value: &mut T, add: W) -> bool
where
    T: Clone + Default + PartialEq + Send + Sync + 'static,
    W: FnOnce(&mut Ui, &mut T) -> Response,
{
    let mut buffer = ui
        .data_mut(|data| data.get_temp::<T>(id))
        .unwrap_or_else(|| value.clone());

    let response = add(ui, &mut buffer);

    let committed = response.lost_focus() || response.drag_stopped();
    let editing = response.has_focus() || response.dragged();

    if editing && !committed {
        ui.data_mut(|data| data.insert_temp(id, buffer));
        return false;
    }

    ui.data_mut(|data| data.remove_temp::<T>(id));

    if committed && buffer != *value {
        *value = buffer;
        return true;
    }

    false
}

fn inline_row<R>(
    ui: &mut Ui,
    gap: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = gap;
        add_contents(ui)
    })
}
