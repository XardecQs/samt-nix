//! Physics-based motion helpers (springs) plus staggered progress, all gated by
//! the "reduce motion" preference.
//!
//! egui has no built-in springs, so a small mass-spring-damper is integrated
//! per [`egui::Id`] and its state kept in the context's temporary data store.

use eframe::egui;
use egui::Id;

const MOTION_CFG: &str = "gta_mo_motion_cfg";

/// Spring stiffness / damping tuned for a snappy, non-oscillating feel.
const STIFFNESS: f32 = 260.0;
const DAMPING: f32 = 30.0;

#[derive(Clone, Copy)]
struct SpringState {
    value: f32,
    velocity: f32,
    last: f64,
}

/// Stores the global motion config in the context (called from `apply_style`).
pub fn set_reduce(ctx: &egui::Context, reduce: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::new(MOTION_CFG), reduce));
}

pub fn reduce(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new(MOTION_CFG)).unwrap_or(false))
}

/// Advances a spring towards `target` and returns its current value in 0..1.
/// When "reduce motion" is on, it snaps to the target.
pub fn spring(ctx: &egui::Context, id: Id, target: f32) -> f32 {
    if reduce(ctx) {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                SpringState {
                    value: target,
                    velocity: 0.0,
                    last: 0.0,
                },
            )
        });
        return target;
    }

    let now = ctx.input(|i| i.time);
    let mut st = ctx
        .data(|d| d.get_temp::<SpringState>(id))
        .unwrap_or(SpringState {
            value: target,
            velocity: 0.0,
            last: now,
        });
    let dt = ((now - st.last).clamp(0.0, 0.05)) as f32;
    st.last = now;

    let steps = 4;
    let h = dt / steps as f32;
    for _ in 0..steps {
        let accel = (target - st.value) * STIFFNESS - st.velocity * DAMPING;
        st.velocity += accel * h;
        st.value += st.velocity * h;
    }
    if (target - st.value).abs() < 0.001 && st.velocity.abs() < 0.02 {
        st.value = target;
        st.velocity = 0.0;
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, st));
    st.value
}

/// Cubic ease-out for staggered reveals.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

pub fn lerp_pos(a: egui::Pos2, b: egui::Pos2, t: f32) -> egui::Pos2 {
    egui::pos2(lerp(a.x, b.x, t), lerp(a.y, b.y, t))
}

/// Start time for the current list "epoch". When `epoch` changes, the clock
/// resets so rows animate in again.
pub fn epoch_start(ctx: &egui::Context, epoch: u64) -> f64 {
    let now = ctx.input(|i| i.time);
    let key = Id::new("gta_mo_list_epoch");
    match ctx.data(|d| d.get_temp::<(u64, f64)>(key)) {
        Some((e, start)) if e == epoch => start,
        _ => {
            ctx.data_mut(|d| d.insert_temp(key, (epoch, now)));
            now
        }
    }
}

/// Eased 0..1 progress for row `index`, delayed by a few ms per row.
pub fn stagger(now: f64, start: f64, index: usize, reduce: bool) -> f32 {
    if reduce {
        return 1.0;
    }
    let delay = index as f64 * 0.018;
    let dur = 0.20;
    ease_out((((now - start - delay) / dur) as f32).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_snaps_when_reduced() {
        let ctx = egui::Context::default();
        set_reduce(&ctx, true);
        let v = spring(&ctx, Id::new("t"), 1.0);
        assert_eq!(v, 1.0);
    }

    #[test]
    fn stagger_is_clamped() {
        assert_eq!(stagger(0.0, 0.0, 100, true), 1.0);
        assert!(stagger(0.0, 0.0, 0, false) <= 1.0);
        assert_eq!(stagger(10.0, 0.0, 0, false), 1.0);
    }
}
