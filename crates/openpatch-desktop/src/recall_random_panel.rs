// Shared rack/browser face. Printed marks describe real controls and scale
// weights; output lamps come from the callback, never configured values.
#[derive(Default)]
struct RecallRandomUi {
    drafts: BTreeMap<String, RecallScaleDraft>,
    feedback: BTreeMap<String, String>,
}
struct RecallScaleDraft {
    revision: u64,
    data: openpatch_core::RecallRandomData,
    slot: usize,
}
const RECALL_RANDOM_KNOBS: [&str; 13] = [
    "t_rate",
    "t_bias",
    "t_jitter",
    "t_pulse_width",
    "t_pulse_spread",
    "deja_vu",
    "length",
    "x_spread",
    "x_bias",
    "x_steps",
    "y_spread",
    "y_bias",
    "y_steps",
];
fn recall_random_data(node: &Node) -> &openpatch_core::RecallRandomData {
    static DEFAULT: std::sync::OnceLock<openpatch_core::RecallRandomData> =
        std::sync::OnceLock::new();
    match &node.content {
        Some(openpatch_core::ModuleContent::RecallRandom(data)) => data.as_ref(),
        _ => DEFAULT.get_or_init(Default::default),
    }
}
fn recall_random_control(name: &str) -> (Pos2, f32) {
    let i = RECALL_RANDOM_KNOBS
        .iter()
        .position(|n| *n == name)
        .expect("Recall rotary");
    (
        pos2(
            58.0 + (i % 7) as f32 * 111.0,
            if i < 7 { 132.0 } else { 236.0 },
        ),
        28.0,
    )
}
fn recall_random_value_center(name: &str) -> Pos2 {
    let (p, _) = recall_random_control(name);
    p + vec2(0.0, 44.0)
}
fn recall_random_port(input: bool, index: usize) -> Pos2 {
    pos2(
        if input {
            40.0 + index as f32 * 73.0
        } else {
            70.0 + index as f32 * 111.0
        },
        if input { 325.0 } else { 372.0 },
    )
}
fn recall_random_label(name: &str) -> &str {
    match name {
        "t_rate" => "RATE",
        "t_bias" => "T BIAS",
        "t_jitter" => "JITTER",
        "t_pulse_width" => "PULSE WIDTH",
        "t_pulse_spread" => "WIDTH SPREAD",
        "deja_vu" => "DEJA VU",
        "length" => "LOOP LENGTH",
        "x_spread" => "X SPREAD",
        "x_bias" => "X BIAS",
        "x_steps" => "X STEPS",
        "y_spread" => "Y SPREAD",
        "y_bias" => "Y BIAS",
        "y_steps" => "Y STEPS",
        other => other,
    }
}
fn paint_recall_random_control_print(painter: &egui::Painter, rect: Rect, name: &str, z: f32) {
    let (p, _) = recall_random_control(name);
    painter.text(
        rect.min + (p.to_vec2() - vec2(0.0, 43.0)) * z,
        Align2::CENTER_CENTER,
        recall_random_label(name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * z),
        INK,
    );
}
fn recall_parameter_menu(
    ui: &mut egui::Ui,
    node: &Node,
    module: Option<&Value>,
    names: &[&str],
    revision: u64,
) -> Option<RackAction> {
    let mut action = None;
    for name in names {
        let Some(spec) = module.and_then(|m| m["params"].get(*name)) else {
            continue;
        };
        let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
        let max = spec["max"].as_f64().unwrap_or(1.0) as f32;
        let mut value = node
            .params
            .get(*name)
            .copied()
            .unwrap_or(spec["default"].as_f64().unwrap_or(0.0) as f32);
        let old = value;
        ui.horizontal(|ui| {
            ui.label(name.replace('_', " ").to_uppercase());
            if let Some(labels) = spec["labels"].as_array() {
                let selected = labels
                    .get((value - min).round() as usize)
                    .and_then(Value::as_str)
                    .unwrap_or("Choose");
                egui::ComboBox::from_id_salt((&node.id, name))
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, label) in labels.iter().enumerate() {
                            ui.selectable_value(
                                &mut value,
                                min + i as f32,
                                label.as_str().unwrap_or("?"),
                            );
                        }
                    });
            } else {
                ui.add(
                    egui::DragValue::new(&mut value)
                        .range(min..=max)
                        .speed(spec["step"].as_f64().unwrap_or(0.01)),
                );
            }
        });
        if value != old {
            action = Some(RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":name,"value":value,"expected_revision":revision}),
            ));
        }
    }
    action
}
impl Rack {
    pub(crate) fn recall_capture_feedback(&mut self, id: &str, text: String) {
        self.recall_ui.feedback.insert(id.into(), text);
    }
    #[allow(clippy::too_many_arguments)]
    fn recall_random_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let painter = ui.painter().clone();
        let p = |x, y| rect.min + vec2(x, y) * z;
        let font = |n| crate::panel_skin::print_font(painter.ctx(), n * z);
        let mut action = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_max(p(392.0,12.0),p(793.0,40.0))),|ui|{
            ui.add_enabled_ui(interactive,|ui|{ui.spacing_mut().button_padding=vec2(3.0*z,2.0*z);ui.spacing_mut().interact_size=vec2(12.0*z,18.0*z);ui.spacing_mut().item_spacing.x=4.0*z;ui.horizontal(|ui|{
                ui.menu_button(egui::RichText::new("RHYTHM").font(font(9.0)),|ui|{if let Some(a)=recall_parameter_menu(ui,node,module,&["seed","t_model","t_range","t_repeat"],revision){action=Some(a);}});
                ui.menu_button(egui::RichText::new("X / Y").font(font(9.0)),|ui|{if let Some(a)=recall_parameter_menu(ui,node,module,&["x_repeat","x_clock","x_control_mode","x_range","x_register","y_control_mode","y_range","y_ratio"],revision){action=Some(a);}});
                let menu=ui.menu_button(egui::RichText::new("SCALES / RECORD").font(font(9.0)),|ui|{
                    let data=recall_random_data(node);
                    let draft=self.recall_ui.drafts.entry(node.id.clone()).or_insert_with(||RecallScaleDraft{revision,slot:usize::from(data.active_slot),data:data.clone()});
                    ui.label("Six weighted scales · voltage is V/oct");
                    let stale=draft.revision!=revision;
                    if stale{ui.colored_label(egui::Color32::LIGHT_RED,"The patch changed. Reload before applying.");}
                    ui.horizontal(|ui|{let mut active=draft.data.active_slot+1;let mut slot=draft.slot+1;ui.label("Active scale");ui.add(egui::DragValue::new(&mut active).range(1..=6));ui.label("Edit scale");ui.add(egui::DragValue::new(&mut slot).range(1..=6));draft.data.active_slot=active-1;draft.slot=slot-1;});
                    let scale=&mut draft.data.scales[draft.slot];
                    ui.horizontal(|ui|{ui.label("Repeat interval (V)");ui.add(egui::DragValue::new(&mut scale.base_interval).speed(0.01));});
                    let mut remove=None;
                    egui::ScrollArea::vertical().max_height(235.0).show(ui,|ui|{egui::Grid::new(("recall-scale",&node.id)).show(ui,|ui|{
                        ui.label("Degree voltage");ui.label("Weight 0–255");ui.end_row();
                        let can_remove=scale.degrees.len()>1;
                        for(index,degree)in scale.degrees.iter_mut().enumerate(){ui.add(egui::DragValue::new(&mut degree.voltage).speed(1.0/1200.0).max_decimals(5));ui.add(egui::DragValue::new(&mut degree.weight).range(0..=255));if ui.add_enabled(can_remove,egui::Button::new("Remove")).clicked(){remove=Some(index);}ui.end_row();}
                    });});
                    if let Some(index)=remove{scale.degrees.remove(index);}
                    if ui.add_enabled(scale.degrees.len()<16,egui::Button::new("Add degree")).clicked(){let voltage=scale.degrees.last().map_or(0.0,|d|d.voltage+1.0/12.0);scale.degrees.push(openpatch_core::RecallRandomDegree{voltage,weight:255});}
                    let valid=draft.data.validate();if let Err(error)=&valid{ui.colored_label(egui::Color32::LIGHT_RED,error);}
                    if ui.add_enabled(!stale&&valid.is_ok(),egui::Button::new("Apply scales")).clicked(){action=Some(RackAction::Command("set_module_content",json!({"id":node.id,"content":openpatch_core::ModuleContent::RecallRandom(draft.data.clone().into()),"expected_revision":draft.revision})));ui.close();}
                    ui.separator();ui.label("Record notes from REGISTER CV while audio runs.");
                    ui.horizontal(|ui|{for(label,command)in[("Record","begin"),("Stop","stop"),("Cancel","cancel"),("Apply capture","commit")]{if ui.button(label).clicked(){action=Some(RackAction::Command("capture_recall_random_scale",json!({"id":node.id,"action":command,"slot":draft.slot,"expected_revision":revision})));}}});
                    ui.label("Stop, then apply. Editing the patch makes a recording stale.");
                    if let Some(text)=self.recall_ui.feedback.get(&node.id){ui.label(text);}
                    if ui.button("Reload saved scales").clicked(){self.recall_ui.drafts.remove(&node.id);}
                });
                if menu.inner.is_none(){self.recall_ui.drafts.remove(&node.id);}
            });});
        });
        painter.text(
            p(25.0, 72.0),
            Align2::LEFT_CENTER,
            "T · RHYTHM / REPEATABLE LOOPS",
            font(10.0),
            INK,
        );
        let data = recall_random_data(node);
        let scale = &data.scales[usize::from(data.active_slot)];
        painter.rect_filled(
            Rect::from_min_max(p(680.0, 197.0), p(789.0, 285.0)),
            3.0 * z,
            Color32::from_rgb(17, 28, 23),
        );
        painter.text(
            p(734.0, 209.0),
            Align2::CENTER_CENTER,
            format!("SCALE {}", data.active_slot + 1),
            font(9.0),
            Color32::from_rgb(191, 197, 174),
        );
        for (i, degree) in scale.degrees.iter().enumerate() {
            let x = 690.0 + i as f32 * 6.0;
            let height = 27.0 * f32::from(degree.weight) / 255.0;
            painter.line_segment(
                [p(x, 258.0), p(x, 258.0 - height)],
                Stroke::new(3.0 * z, Color32::from_rgb(206, 161, 72)),
            );
        }
        let lights = self.module_lights.get(&node.id);
        let recording = event_fork_lamp(lights, 7) > 0.0;
        let overflow = event_fork_lamp(lights, 8) > 0.0;
        painter.text(
            p(734.0, 274.0),
            Align2::CENTER_CENTER,
            if overflow {
                "OVERFLOW"
            } else if recording {
                "RECORDING"
            } else {
                "NOTE WEIGHTS"
            },
            font(8.0),
            Color32::from_rgb(191, 197, 174),
        );
        action
    }
}
