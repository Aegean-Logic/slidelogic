//! The others in a co-editing session (`Session::collab_others`): the shapes each one has
//! selected, outlined on the slide in their colour with their name, and a dot with their
//! initials on the thumbnail of the slide they're on.

use deckcraft_engine::collab::Other;
use deckcraft_engine::geom::Point;
use deckcraft_engine::model::SlideId;
use deckcraft_engine::{DocState, Target};
use egui::{Align2, Color32, Pos2, Rect, Stroke, pos2, vec2};

use crate::canvas::Xf;

/// The most dots on one thumbnail; more show as "+n".
const MAX_DOTS: usize = 3;

pub fn color(o: &Other) -> Color32 {
    let [r, g, b] = o.color;
    Color32::from_rgb(r, g, b)
}

/// Up to two initials of a name, upper-case ("Ada Lovelace" → "AL").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let first = words.first().and_then(|w| w.chars().next());
    let last = if words.len() > 1 { words.last().and_then(|w| w.chars().next()) } else { None };
    let s: String = first.into_iter().chain(last).flat_map(char::to_uppercase).collect();
    if s.is_empty() { "?".into() } else { s }
}

/// The others' selections on the slide being edited: an outline of each shape in the person's
/// colour, and their name over the first one.
pub fn paint_canvas(st: &DocState, others: &[Other], painter: &egui::Painter, xf: Xf) {
    if st.selection.target != Target::Slides || others.is_empty() {
        return;
    }
    let Some(slide) = st.current_slide() else { return };
    for o in others.iter().filter(|o| o.slide == Some(slide.id)) {
        let col = color(o);
        let mut label_at: Option<Pos2> = None;
        for id in &o.shapes {
            let Some(sh) = st.shape(*id) else { continue };
            let x = deckcraft_engine::cmd::xfrm_of(&st.doc, &st.selection, sh);
            let a = x.affine();
            let corners = [Point::new(0.0, 0.0), Point::new(x.w, 0.0), Point::new(x.w, x.h), Point::new(0.0, x.h)].map(|q| xf.to_screen(a * q));
            if corners.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
                continue;
            }
            let outline = Rect::from_points(&corners).expand(3.0);
            painter.rect_stroke(outline, 2.0, Stroke::new(2.0, col), egui::StrokeKind::Outside);
            label_at = Some(label_at.map_or(outline.left_top(), |p| pos2(p.x.min(outline.left()), p.y.min(outline.top()))));
        }
        if let Some(at) = label_at {
            name_tag(painter, at, &o.name, col);
        }
    }
}

/// `name` on a tab in `col`, sitting on top of the point `at` (a box's top-left corner).
fn name_tag(painter: &egui::Painter, at: Pos2, name: &str, col: Color32) {
    let font = egui::FontId::proportional(11.0);
    let galley = painter.layout_no_wrap(name.to_string(), font, Color32::WHITE);
    let size = galley.size() + vec2(8.0, 4.0);
    let rect = Rect::from_min_size(pos2(at.x, at.y - size.y - 1.0), size);
    painter.rect_filled(rect, 2.0, col);
    painter.galley(rect.min + vec2(4.0, 2.0), galley, Color32::WHITE);
}

/// Dots with the initials of the others on slide `slide`, along the top of its thumbnail `img`.
pub fn paint_thumb(painter: &egui::Painter, img: Rect, others: &[Other], slide: Option<SlideId>) {
    let Some(slide) = slide else { return };
    let here: Vec<&Other> = others.iter().filter(|o| o.slide == Some(slide)).collect();
    if here.is_empty() {
        return;
    }
    let r = 8.0;
    let mut c = pos2(img.right() - r - 2.0, img.top() + r + 2.0);
    for o in here.iter().take(MAX_DOTS) {
        painter.circle(c, r, color(o), Stroke::new(1.0, Color32::WHITE));
        painter.text(c, Align2::CENTER_CENTER, initials(&o.name), egui::FontId::proportional(8.0), Color32::WHITE);
        c.x -= 2.0 * r + 2.0;
    }
    if here.len() > MAX_DOTS {
        painter.circle(c, r, Color32::from_gray(90), Stroke::new(1.0, Color32::WHITE));
        painter.text(c, Align2::CENTER_CENTER, format!("+{}", here.len() - MAX_DOTS), egui::FontId::proportional(8.0), Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_names() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("grace"), "G");
        assert_eq!(initials("  Jean  Paul Sartre "), "JS");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn others_are_drawn_on_the_slide_and_its_thumbnail() {
        let mut s = deckcraft_engine::Session::new();
        deckcraft_engine::sample::open_sample(&mut s).unwrap();
        s.collab_start(1, 0).unwrap();
        let st = s.doc().unwrap();
        let slide = st.current_slide().unwrap();
        let id = slide.shapes[0].id;
        let others = vec![Other::new(2, "Bob Brown", [200, 30, 30], Some(slide.id), vec![id])];
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let count = |others: &[Other]| {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                let painter = ui.painter().clone();
                let xf = Xf { origin: pos2(0.0, 0.0), scale: 0.5 };
                paint_canvas(st, others, &painter, xf);
                paint_thumb(&painter, Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 90.0)), others, Some(slide.id));
            });
            out.textures_delta.clear();
            out.shapes.len()
        };
        let none = count(&[]);
        // An outline, a name tag (its box and text), a dot and its initials.
        assert!(count(&others) >= none + 5, "{none} {}", count(&others));
        // Someone on another slide isn't drawn here.
        let elsewhere = vec![Other::new(3, "Cy", [0, 0, 200], Some(SlideId(999_999)), vec![id])];
        assert_eq!(count(&elsewhere), none);
    }
}
