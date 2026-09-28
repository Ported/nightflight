//! A plan view: where the sources are, seen from above.
//!
//! Not the 3D scene the plan eventually wants — that is Bevy, later. But for
//! *performing*, a plan view is arguably better than a perspective one: you can
//! see every source at once and read its angle off the page, which is exactly
//! the question you are asking when you decide where to throw something next.
//!
//! The radial scale is deliberately not linear. A helicopter starts 42 m away
//! and lands on your head, and a linear scale would spend its whole range on the
//! approach and leave the interesting last two metres in a single pixel. So
//! `r / (r + 8)`: one metre sits at an eighth of the radius, eight metres at
//! half, forty at five sixths. Everything is on screen and the close detail is
//! where the resolution is.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use engine::telemetry::Telemetry;

/// Metres that map to half the drawing radius.
const HALF_SCALE: f32 = 8.0;

const HEAD: Color32 = Color32::from_rgb(110, 120, 140);
const GRID: Color32 = Color32::from_rgb(48, 52, 62);
const SOURCE: Color32 = Color32::from_rgb(120, 200, 255);
const CENTRED: Color32 = Color32::from_rgb(150, 150, 160);

pub fn show(ui: &mut egui::Ui, telemetry: &Telemetry, names: &[&'static str]) {
    let size = ui.available_size().min(Vec2::splat(420.0));
    let (response, painter) = ui.allocate_painter(Vec2::splat(size.min_elem()), Sense::hover());
    let rect = response.rect;
    let centre = rect.center();
    let radius = rect.width().min(rect.height()) * 0.46;

    // Distance rings, labelled where they fall.
    for metres in [1.0f32, 2.0, 4.0, 8.0, 20.0] {
        let r = radius * metres / (metres + HALF_SCALE);
        painter.circle_stroke(centre, r, Stroke::new(1.0, GRID));
        painter.text(
            centre + Vec2::new(r + 2.0, -2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{metres:.0} m"),
            egui::FontId::proportional(9.0),
            GRID,
        );
    }
    // Ahead is up. The cross marks the axes so left and right are unambiguous.
    painter.line_segment(
        [
            Pos2::new(centre.x, centre.y - radius),
            Pos2::new(centre.x, centre.y + radius),
        ],
        Stroke::new(1.0, GRID),
    );
    painter.line_segment(
        [
            Pos2::new(centre.x - radius, centre.y),
            Pos2::new(centre.x + radius, centre.y),
        ],
        Stroke::new(1.0, GRID),
    );
    painter.text(
        Pos2::new(centre.x, rect.top() + 2.0),
        egui::Align2::CENTER_TOP,
        "ahead",
        egui::FontId::proportional(10.0),
        HEAD,
    );

    // The listener: a head with a nose, so the view has a direction.
    painter.circle_stroke(centre, 9.0, Stroke::new(1.5, HEAD));
    painter.line_segment(
        [
            Pos2::new(centre.x, centre.y - 9.0),
            Pos2::new(centre.x, centre.y - 14.0),
        ],
        Stroke::new(1.5, HEAD),
    );

    for (index, part) in telemetry.parts[..telemetry.part_count as usize]
        .iter()
        .enumerate()
    {
        let name = names.get(index).copied().unwrap_or("?");
        // A part that is not placed is drawn at the centre, greyed: it is in the
        // middle of your head, which is exactly where a kick belongs.
        let (x, z) = if part.placed {
            (part.position[0], part.position[2])
        } else {
            (0.0, 0.0)
        };
        let distance = (x * x + z * z).sqrt();
        let scaled = radius * distance / (distance + HALF_SCALE);
        let at = if distance > 1e-3 {
            centre + Vec2::new(x / distance * scaled, z / distance * scaled)
        } else {
            centre + Vec2::new(0.0, 14.0 + 11.0 * index as f32)
        };

        // Size and brightness follow the level, so the picture pulses with the
        // music rather than just mapping it.
        let level = part.level.clamp(0.0, 1.0).sqrt();
        let colour = if part.placed { SOURCE } else { CENTRED };
        let faded = colour.gamma_multiply(if part.muted {
            0.25
        } else {
            0.35 + 0.65 * level
        });
        painter.circle_filled(at, 3.0 + 9.0 * level, faded);
        painter.text(
            at + Vec2::new(0.0, -8.0 - 9.0 * level),
            egui::Align2::CENTER_BOTTOM,
            name,
            egui::FontId::proportional(10.0),
            faded,
        );
    }

    // Height, as a second little strip: the plan view cannot show it, and the
    // pad voices are deliberately spread vertically.
    let strip = Rect::from_min_size(
        Pos2::new(rect.left() + 4.0, rect.bottom() - 18.0),
        Vec2::new(rect.width() - 8.0, 14.0),
    );
    painter.rect_stroke(strip, 2.0, Stroke::new(1.0, GRID), egui::StrokeKind::Inside);
    for part in telemetry.parts[..telemetry.part_count as usize].iter() {
        if !part.placed {
            continue;
        }
        // +/- 4 m of height across the strip.
        let t = (part.position[1] / 8.0 + 0.5).clamp(0.0, 1.0);
        let x = strip.left() + t * strip.width();
        painter.line_segment(
            [
                Pos2::new(x, strip.top() + 2.0),
                Pos2::new(x, strip.bottom() - 2.0),
            ],
            Stroke::new(2.0, SOURCE.gamma_multiply(0.8)),
        );
    }
    painter.text(
        Pos2::new(strip.left() + 3.0, strip.center().y),
        egui::Align2::LEFT_CENTER,
        "below",
        egui::FontId::proportional(8.0),
        GRID,
    );
    painter.text(
        Pos2::new(strip.right() - 3.0, strip.center().y),
        egui::Align2::RIGHT_CENTER,
        "above",
        egui::FontId::proportional(8.0),
        GRID,
    );
}
