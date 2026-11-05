use std::ops::{Div, Mul};

use egui::{Pos2, Rect, Vec2, vec2};

use crate::graph::snapshot::CanvasSnapshot;

const MIN_ZOOM: f32 = 0.7;
const MAX_ZOOM: f32 = 3.5;
const SCROLL_SENSITIVITY: f32 = 0.001;

/// Represents the view window of the interface. Saves and keeps track of pan, zoom, and
/// clipping rect.
#[derive(Clone)]
pub struct Canvas {
    pan: Vec2,
    zoom: f32,
    clip_rect: Rect,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            pan: Vec2::ZERO,
            zoom: 1.0,
            clip_rect: Rect::NOTHING,
        }
    }
}

impl Canvas {
    pub(super) fn zoom(&mut self, mouse_position: Pos2, clip_rect: Rect, zoom_delta: f32) {
        let old_zoom = self.zoom;

        // World/canvas position currently underneath the mouse.
        let mouse_world = ((mouse_position - clip_rect.min - self.pan) / old_zoom).to_pos2();

        self.zoom = (old_zoom * (1.0 + zoom_delta * SCROLL_SENSITIVITY)).clamp(MIN_ZOOM, MAX_ZOOM);

        // Move the canvas so the same world point stays under the mouse.
        self.pan = (mouse_position - clip_rect.min) - mouse_world.to_vec2() * self.zoom;

        self.clip_rect = clip_rect;
    }

    pub(super) fn snapshot(&self) -> CanvasSnapshot {
        CanvasSnapshot {
            pan: (self.pan.x, self.pan.y),
            zoom: self.zoom,
        }
    }

    pub(super) fn from_snapshot(snapshot: CanvasSnapshot) -> Self {
        Self {
            pan: vec2(snapshot.pan.0, snapshot.pan.1),
            zoom: snapshot.zoom,
            ..Default::default()
        }
    }

    pub(super) fn pan(&mut self, pan_delta: Vec2) {
        self.pan += pan_delta;
    }

    pub(super) fn world_to_screen_coordinates(&self, world: Pos2) -> Pos2 {
        self.clip_rect.min + self.pan + world.to_vec2() * self.zoom
    }

    pub(super) fn screen_to_world_coordinates(&self, screen: Pos2) -> Pos2 {
        ((screen - self.clip_rect.min - self.pan) / self.zoom).to_pos2()
    }

    /// Rescales an element's size based on the current Canvas' zoom value.
    pub fn zoomed<T>(&self, element: T) -> <T as Mul<f32>>::Output
    where
        T: Mul<f32>,
    {
        element * self.zoom
    }

    pub(super) fn zoomed_out<T>(&self, element: T) -> <T as Div<f32>>::Output
    where
        T: Div<f32>,
    {
        element / self.zoom
    }

    pub(super) fn current_zoom(&self) -> f32 {
        self.zoom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELEMENT_SIZE: f32 = 8.0;
    const ZOOM: f32 = 2.0;

    #[test]
    fn zoomed_at_default_zoom_returns_element_unchanged() {
        let canvas = Canvas::default();

        let result = canvas.zoomed(ELEMENT_SIZE);

        assert_eq!(result, ELEMENT_SIZE);
    }

    #[test]
    fn zoomed_with_zoom_multiplies_element_by_zoom() {
        let canvas = Canvas::from_snapshot(CanvasSnapshot {
            pan: (0.0, 0.0),
            zoom: ZOOM,
        });

        let result = canvas.zoomed(ELEMENT_SIZE);

        assert_eq!(result, ELEMENT_SIZE * ZOOM);
    }
}
