// Material detail follows the one shared cable curve. Bounded world-space
// stitching disappears at thumbnail scales rather than becoming screen noise.
#[allow(clippy::too_many_arguments)]
fn paint_cable_surface(
    painter: &egui::Painter,
    curve: [Pos2; 4],
    color: Color32,
    zoom: f32,
    width: f32,
    finish: crate::cable_presentation::CableFinish,
    opacity: f32,
) {
    use crate::cable_presentation::CableFinish;
    if !Rect::from_points(&curve)
        .expand(width)
        .intersects(painter.clip_rect())
    {
        return;
    }
    let spacing = if finish == CableFinish::Woven {
        7.
    } else {
        18.
    } * zoom;
    let mut distance = 0.;
    let mut next_mark = spacing * 0.5;
    let mut previous = curve[0];
    let mut marks = 0;
    let mut mesh = egui::Mesh::default();
    let mut stroke = |a: Pos2, b: Pos2, ink: Color32, thickness: f32| {
        let v = b - a;
        if v.length_sq() < 0.0001 {
            return;
        }
        let side = vec2(-v.y, v.x).normalized() * thickness * 0.5;
        let start = mesh.vertices.len() as u32;
        for position in [a - side, a + side, b + side, b - side] {
            mesh.colored_vertex(position, ink.gamma_multiply(opacity));
        }
        mesh.add_triangle(start, start + 1, start + 2);
        mesh.add_triangle(start, start + 2, start + 3);
    };
    for step in 1..=96 {
        let point = cable_curve_prefix(curve, step as f32 / 96.)[3];
        let delta = point - previous;
        let length = delta.length();
        if length > 0.0001 {
            let along = delta / length;
            let normal = vec2(-along.y, along.x);
            while next_mark <= distance + length && marks < 256 {
                let center = previous + along * (next_mark - distance);
                if painter.clip_rect().expand(width).contains(center) {
                    let half = width * 0.39;
                    if finish == CableFinish::Woven {
                        let thread = Color32::from_rgba_unmultiplied(176, 207, 192, 85);
                        let edge = color.gamma_multiply(0.48);
                        stroke(
                            center - normal * half - along * 2. * zoom,
                            center + normal * half + along * 2. * zoom,
                            edge,
                            1.0 * zoom,
                        );
                        stroke(
                            center - normal * half + along * 2. * zoom,
                            center + normal * half - along * 2. * zoom,
                            thread,
                            0.65 * zoom,
                        );
                    } else {
                        stroke(
                            center - along * 3.5 * zoom - normal * width * 0.1,
                            center + along * 3.5 * zoom - normal * width * 0.1,
                            Color32::from_rgb(235, 226, 194),
                            1.0 * zoom,
                        );
                        stroke(
                            center + along * 4. * zoom - normal * width * 0.1,
                            center + along * 5.5 * zoom - normal * width * 0.1,
                            color.gamma_multiply(0.35),
                            1.0 * zoom,
                        );
                    }
                }
                next_mark += spacing;
                marks += 1;
            }
        }
        distance += length;
        previous = point;
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}
