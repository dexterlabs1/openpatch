// MIDI note gates share the assignment surface, with actual per-cell gate lamps.
const MIDI_GATE_PARAMS: [&str; 16] = [
    "note1", "note2", "note3", "note4", "note5", "note6", "note7", "note8", "note9", "note10",
    "note11", "note12", "note13", "note14", "note15", "note16",
];
const MIDI_GATE_DEFAULT_NOTES: [i16; 16] = [
    48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39,
];
fn midi_gate_note_name(note: i16) -> String {
    if !(0..=127).contains(&note) {
        return "—".into();
    }
    format!(
        "{}{}",
        ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"][note as usize % 12],
        note / 12 - 1
    )
}
fn midi_gate_panic_rect() -> Rect {
    Rect::from_min_size(pos2(320.0, 346.0), vec2(138.0, 23.0))
}
#[cfg(test)]
fn midi_gate_edit(node: &Node, index: usize, assignment: i16, revision: u64) -> ParameterEdit {
    midi_assignment_edit(node, index, assignment, revision, MidiAssignmentPanel::Gate)
}
impl Rack {
    fn midi_gate_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
    ) -> Option<RackAction> {
        self.midi_assignment_panel(ui, node, rect, revision, MidiAssignmentPanel::Gate)
    }
    fn midi_gate_activity_lamp(
        &self,
        painter: &egui::Painter,
        node: &Node,
        index: usize,
        center: Pos2,
        z: f32,
    ) {
        let brightness = self
            .module_lights
            .get(&node.id)
            .filter(|s| index < usize::from(s.count))
            .map_or(0.0, |s| s.brightness[index][0])
            .clamp(0.0, 1.0);
        painter.circle_filled(center, 3.5 * z, Color32::from_rgb(49, 57, 53));
        self.lighting.circle(
            painter,
            center,
            2.3 * z,
            Color32::from_rgb(104, 208, 148).gamma_multiply(0.08 + 0.92 * brightness),
            brightness,
            0.08,
        );
    }
}
