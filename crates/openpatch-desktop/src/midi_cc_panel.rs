// CC adapters retain the established control and test surface.
const MIDI_CC_PARAMS: [&str; 16] = [
    "cc1", "cc2", "cc3", "cc4", "cc5", "cc6", "cc7", "cc8", "cc9", "cc10", "cc11", "cc12", "cc13",
    "cc14", "cc15", "cc16",
];
#[cfg(test)]
fn midi_cc_value_rect(index: usize) -> Rect {
    midi_assignment_value_rect(index)
}
fn midi_cc_output_position(index: usize) -> Pos2 {
    midi_assignment_output_position(index)
}
fn midi_cc_snapshot_valid(state: &Value) -> bool {
    midi_assignment_snapshot_valid(state)
        && state["values"].as_array().is_some_and(|v| {
            v.len() == 128 && v.iter().all(|n| n.as_u64().is_some_and(|n| n <= 127))
        })
}
#[cfg(test)]
fn midi_cc_edit(node: &Node, index: usize, assignment: i16, revision: u64) -> ParameterEdit {
    midi_assignment_edit(node, index, assignment, revision, MidiAssignmentPanel::Cc)
}
impl Rack {
    fn midi_cc_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
    ) -> Option<RackAction> {
        self.midi_assignment_panel(ui, node, rect, revision, MidiAssignmentPanel::Cc)
    }
}
