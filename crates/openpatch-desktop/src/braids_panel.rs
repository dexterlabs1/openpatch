impl Rack {
    fn braids_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let module = module?;
        let painter = ui.painter().clone();
        let mut action = None;
        let z = self.zoom;
        for (name, x, y, base, fg) in [
            ("model", 176., 59., "Rogan2SGray", "Rogan2SGray_fg"),
            ("fine", 19., 138., "Rogan2PSWhite", "Rogan2PSWhite_fg"),
            ("frequency", 97., 138., "Rogan2PSWhite", "Rogan2PSWhite_fg"),
            ("fm_amount", 176., 138., "Rogan2PSWhite", "Rogan2PSWhite_fg"),
            ("timbre", 19., 217., "Rogan2PSGreen", "Rogan2PSGreen_fg"),
            (
                "timbre_amount",
                97.,
                217.,
                "Rogan2PSGreen",
                "Rogan2PSGreen_fg",
            ),
            ("color", 176., 217., "Rogan2PSRed", "Rogan2PSRed_fg"),
        ] {
            let cap = crate::rack_vendor::braids_piece_rect(rect, x, y, 38.5);
            let spec = &module["params"][name];
            let min = spec["min"].as_f64().unwrap() as f32;
            let max = spec["max"].as_f64().unwrap() as f32;
            let value = node
                .params
                .get(name)
                .copied()
                .unwrap_or(spec["default"].as_f64().unwrap() as f32);
            let n = normalize(value, min, max, name == "frequency");
            crate::rack_vendor::piece(&painter, cap, base, 0.);
            crate::rack_vendor::piece(&painter, cap, fg, (n - 0.5) * std::f32::consts::PI * 1.66);
            if let Some(a) = self.console_parameter(
                ui,
                node,
                module,
                name,
                cap.center(),
                19.25,
                false,
                revision,
                interactive,
                false,
            ) {
                action = Some(a);
            }
        }
        let display = crate::rack_vendor::braids_piece_rect(rect, 14., 53., 148.);
        let display = Rect::from_min_size(display.min, vec2(148., 56.) * z);
        painter.rect_filled(display, 3. * z, Color32::from_rgb(20, 25, 20));
        let model = node.params.get("model").copied().unwrap_or(37.) as usize;
        let label = module["params"]["model"]["labels"][model.min(46)]
            .as_str()
            .unwrap_or("BRAIDS");
        let codes = [
            "CSAW", "MORP", "SWSQ", "SITR", "BUZZ", "SBSQ", "SBSA", "SYSQ", "SYSA", "3SAW", "3SQR",
            "3TRI", "3SIN", "RING", "SWRM", "COMB", "TOY!", "ZLPF", "ZPKF", "ZBPF", "ZHPF", "VOSM",
            "VOWL", "VFOF", "HARM", "FM  ", "FBFM", "CFM!", "PLUK", "BOWD", "BLOW", "FLUT", "BELL",
            "DRUM", "KICK", "CYMB", "SNAR", "WTBL", "WMAP", "WLIN", "WPAR", "NOIS", "TWIN", "CLKN",
            "GRAN", "PART", "DGM ",
        ];
        painter.text(
            display.center(),
            Align2::CENTER_CENTER,
            codes[model.min(46)],
            egui::FontId::monospace(32. * z),
            Color32::from_rgb(173, 209, 48),
        );
        let response = ui.interact(
            display,
            ui.id().with((&node.id, "braids-model-list")),
            Sense::click(),
        );
        response.on_hover_text(format!("{label}\nRight-click to choose an original Braids model or set output level.")).context_menu(|ui|{
            if ui.button("Output level…").clicked(){self.open_precision_editor(original_plaits_edit(node,"level",&module["params"]["level"],revision),display);ui.close();}
            egui::ScrollArea::vertical().max_height(360.).show(ui,|ui|{for (i,name) in module["params"]["model"]["labels"].as_array().unwrap().iter().enumerate(){if ui.selectable_label(model==i,format!("{i:02} {}",name.as_str().unwrap())).clicked(){action=Some(RackAction::Command("set_parameter",json!({"id":node.id,"parameter":"model","value":i,"expected_revision":revision})));ui.close();}}});
        });
        action
    }
}
