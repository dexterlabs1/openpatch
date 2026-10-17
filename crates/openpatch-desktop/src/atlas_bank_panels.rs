// Dense banks retain their stable row/socket grid and raw bipolar coefficients.
fn atlas_bank_rotary_kind(kind: &str) -> bool {
    matches!(kind, "attenuverter8" | "cv_mix")
}
fn atlas_bank_row(name: &str) -> usize {
    name.trim_start_matches(|c: char| !c.is_ascii_digit())
        .parse::<usize>()
        .unwrap_or(1)
        .saturating_sub(1)
}
fn atlas_bank_control(kind: &str, name: &str) -> (Pos2, f32) {
    (
        pos2(88.0, utility_row_y(kind, atlas_bank_row(name))),
        if kind == "cv_mix" { 26.0 } else { 14.0 },
    )
}
fn atlas_bank_value_center(kind: &str, name: &str) -> Pos2 {
    pos2(
        if kind == "cv_mix" { 162.0 } else { 138.0 },
        utility_row_y(kind, atlas_bank_row(name)),
    )
}
fn atlas_bank_display_scale(patch: &Patch, node: &Node, name: &str) -> f32 {
    if utility_source_row(patch, node, atlas_bank_row(name)).is_some() {
        100.0
    } else {
        10.0
    }
}
fn atlas_bank_value(value: f32, scale: f32) -> String {
    format!(
        "{:+.1}{}",
        value * scale,
        if scale == 100.0 { "%" } else { " V" }
    )
}
