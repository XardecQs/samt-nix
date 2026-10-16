//! Custom edge shape for the dependency graph: a straight/curved line colored by
//! the relation kind (set each frame from the active palette), with the label
//! never drawn and an optional dimming for the focus mode.

use eframe::egui;
use egui::{Color32, Shape, Stroke, Vec2};
use egui_graphs::{DefaultEdgeShape, DisplayEdge, DisplayNode, DrawContext, EdgeProps, Node};
use petgraph::{stable_graph::IndexType, EdgeType};

/// Edge shape used by the dependency graph.
#[derive(Clone, Debug)]
pub struct ModEdgeShape {
    default_impl: DefaultEdgeShape,
    /// Stroke color set every frame from the relation kind + palette.
    pub color: Option<Color32>,
    /// Dimmed when a focus mode is active and this edge is out of the neighborhood.
    pub dimmed: bool,
}

impl<E: Clone> From<EdgeProps<E>> for ModEdgeShape {
    fn from(props: EdgeProps<E>) -> Self {
        Self {
            default_impl: DefaultEdgeShape::from(props),
            color: None,
            dimmed: false,
        }
    }
}

impl ModEdgeShape {
    /// Stroke width in canvas units (kept constant on screen by the caller).
    pub fn set_width(&mut self, width: f32) {
        self.default_impl.width = width;
    }

    /// Arrow tip size in canvas units (kept constant on screen by the caller).
    pub fn set_tip_size(&mut self, size: f32) {
        self.default_impl.tip_size = size;
    }
}

impl<N: Clone, E: Clone, Ty: EdgeType, Ix: IndexType, D: DisplayNode<N, E, Ty, Ix>>
    DisplayEdge<N, E, Ty, Ix, D> for ModEdgeShape
{
    fn shapes(
        &mut self,
        start: &Node<N, E, Ty, Ix, D>,
        end: &Node<N, E, Ty, Ix, D>,
        ctx: &DrawContext,
    ) -> Vec<egui::Shape> {
        let base = self
            .color
            .unwrap_or_else(|| ctx.ctx.style().visuals.widgets.inactive.fg_stroke.color);
        let color = if self.dimmed {
            base.gamma_multiply(0.25)
        } else {
            base
        };
        let width = ctx
            .meta
            .canvas_to_screen_size(self.default_impl.width)
            .max(0.5);
        let stroke = Stroke::new(width, color);

        // Self-loop: approximate with a small circle (rare in the mod graph).
        if start.id() == end.id() {
            let r = ctx
                .meta
                .canvas_to_screen_size(6.0 + 3.0 * self.default_impl.order as f32);
            return vec![Shape::circle_stroke(
                ctx.meta.canvas_to_screen_pos(start.location()),
                r,
                stroke,
            )];
        }

        let dir = (end.location() - start.location()).normalized();
        let p0 = start.display().closest_boundary_point(dir);
        let p1 = end.display().closest_boundary_point(-dir);
        let sp0 = ctx.meta.canvas_to_screen_pos(p0);
        let sp1 = ctx.meta.canvas_to_screen_pos(p1);

        // Slight curve per parallel edge so overlapping relations stay visible.
        let seg = sp1 - sp0;
        let perp = if seg.length() > 0.001 {
            Vec2::new(-seg.y, seg.x).normalized()
        } else {
            Vec2::ZERO
        };
        let off = perp
            * ctx.meta.canvas_to_screen_size(self.default_impl.curve_size)
            * self.default_impl.order as f32
            * 0.5;
        let c1 = sp0 + seg * 0.25 + off;
        let c2 = sp0 + seg * 0.75 + off;
        let cubic = egui::epaint::CubicBezierShape::from_points_stroke(
            [sp0, c1, c2, sp1],
            false,
            Color32::TRANSPARENT,
            stroke,
        );
        let mut shapes = vec![cubic.into()];

        if ctx.is_directed {
            let tip = ctx.meta.canvas_to_screen_size(self.default_impl.tip_size);
            if tip > 0.5 {
                let tangent = (sp1 - c2).normalized();
                let angle = self.default_impl.tip_angle;
                let d1 = rotate(tangent, angle);
                let d2 = rotate(tangent, -angle);
                shapes.push(Shape::convex_polygon(
                    vec![sp1, sp1 - d1 * tip, sp1 - d2 * tip],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        shapes
    }

    fn update(&mut self, state: &EdgeProps<E>) {
        <DefaultEdgeShape as DisplayEdge<N, E, Ty, Ix, D>>::update(&mut self.default_impl, state);
    }

    fn is_inside(
        &self,
        start: &Node<N, E, Ty, Ix, D>,
        end: &Node<N, E, Ty, Ix, D>,
        pos: egui::Pos2,
    ) -> bool {
        <DefaultEdgeShape as DisplayEdge<N, E, Ty, Ix, D>>::is_inside(
            &self.default_impl,
            start,
            end,
            pos,
        )
    }
}

/// Rotates `v` by `angle` radians.
fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(cos * v.x - sin * v.y, sin * v.x + cos * v.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_quarter_turn() {
        let r = rotate(Vec2::new(1.0, 0.0), std::f32::consts::FRAC_PI_2);
        assert!((r.x - 0.0).abs() < 1e-4);
        assert!((r.y - 1.0).abs() < 1e-4);
    }
}
