//! Custom node shape for the dependency graph: a circle with a readable,
//! optionally backed label rendered at a constant screen font size.

use eframe::egui;
use egui::{Color32, FontFamily, FontId, Pos2, Shape, Stroke, Vec2};
use egui_graphs::{DisplayNode, DrawContext, NodeProps};
use petgraph::{stable_graph::IndexType, EdgeType};

/// Node shape used by the graph views.
#[derive(Clone, Debug)]
pub struct ModNodeShape {
    pub pos: Pos2,
    pub selected: bool,
    pub dragged: bool,
    pub hovered: bool,
    pub color: Option<Color32>,
    pub label_text: String,
    /// Node radius in canvas units (grows with the zoom).
    pub radius: f32,
    /// Label font size in screen pixels (constant across zoom levels).
    pub label_font: f32,
    /// Draw a background chip behind the label for readability.
    pub label_bg: bool,
}

impl Default for ModNodeShape {
    fn default() -> Self {
        Self {
            pos: Pos2::ZERO,
            selected: false,
            dragged: false,
            hovered: false,
            color: None,
            label_text: String::new(),
            radius: 5.0,
            label_font: 12.0,
            label_bg: true,
        }
    }
}

impl<N: Clone> From<NodeProps<N>> for ModNodeShape {
    fn from(props: NodeProps<N>) -> Self {
        Self {
            pos: props.location(),
            selected: props.selected,
            dragged: props.dragged,
            hovered: props.hovered,
            color: props.color(),
            label_text: props.label.to_string(),
            ..Default::default()
        }
    }
}

impl<N: Clone, E: Clone, Ty: EdgeType, Ix: IndexType> DisplayNode<N, E, Ty, Ix> for ModNodeShape {
    fn is_inside(&self, pos: Pos2) -> bool {
        (pos - self.pos).length() <= self.radius
    }

    fn closest_boundary_point(&self, dir: Vec2) -> Pos2 {
        self.pos + dir.normalized() * self.radius
    }

    fn shapes(&mut self, ctx: &DrawContext) -> Vec<Shape> {
        let center = ctx.meta.canvas_to_screen_pos(self.pos);
        let radius = ctx.meta.canvas_to_screen_size(self.radius);
        let color = self.color.unwrap_or_else(|| {
            if self.selected || self.dragged || self.hovered {
                ctx.ctx.style().visuals.widgets.active.fg_stroke.color
            } else {
                ctx.ctx.style().visuals.widgets.inactive.fg_stroke.color
            }
        });

        let mut shapes = Vec::with_capacity(3);
        shapes.push(Shape::circle_filled(center, radius, color));
        if self.selected || self.hovered || self.dragged {
            let stroke_color = ctx.ctx.style().visuals.selection.stroke.color;
            shapes.push(Shape::circle_stroke(
                center,
                radius + 1.5,
                Stroke::new(1.5, stroke_color),
            ));
        }

        if self.label_text.is_empty() {
            return shapes;
        }

        let text_color = ctx.ctx.style().visuals.text_color();
        let galley = ctx.ctx.fonts(|f| {
            f.layout_no_wrap(
                self.label_text.clone(),
                FontId::new(self.label_font, FontFamily::Monospace),
                text_color,
            )
        });
        let size = galley.size();
        let pos = Pos2::new(center.x - size.x / 2.0, center.y - radius - size.y - 4.0);
        if self.label_bg {
            let pad = Vec2::new(4.0, 2.0);
            let rect = egui::Rect::from_min_size(pos - pad, size + pad * 2.0);
            let fill = ctx.ctx.style().visuals.window_fill;
            shapes.push(Shape::rect_filled(rect, egui::CornerRadius::same(4), fill));
        }
        shapes.push(egui::epaint::TextShape::new(pos, galley, text_color).into());
        shapes
    }

    fn update(&mut self, state: &NodeProps<N>) {
        self.pos = state.location();
        self.selected = state.selected;
        self.dragged = state.dragged;
        self.hovered = state.hovered;
        self.label_text = state.label.to_string();
        self.color = state.color();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_graphs::DisplayNode;
    use petgraph::Directed;

    type Node = ModNodeShape;

    #[test]
    fn geometry_is_a_circle() {
        let node = ModNodeShape {
            pos: Pos2::new(10.0, 10.0),
            radius: 5.0,
            ..Default::default()
        };
        assert!(<Node as DisplayNode<(), (), Directed, u32>>::is_inside(
            &node,
            Pos2::new(12.0, 10.0)
        ));
        assert!(!<Node as DisplayNode<(), (), Directed, u32>>::is_inside(
            &node,
            Pos2::new(20.0, 10.0)
        ));
        assert_eq!(
            <Node as DisplayNode<(), (), Directed, u32>>::closest_boundary_point(
                &node,
                Vec2::new(1.0, 0.0)
            ),
            Pos2::new(15.0, 10.0)
        );
    }
}
