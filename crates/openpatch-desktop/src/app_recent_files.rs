const RECENT_FILE_LIMIT: usize = 10;

fn recent_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        && (name.ends_with(".openpatch")
            || name.ends_with(".vcv")
            || (name.ends_with(".json") && !name.ends_with(".layout.json")))
}

fn recent_files_from_settings(value: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for name in value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if recent_file_name(name) && !names.iter().any(|existing| existing == name) {
            names.push(name.to_owned());
            if names.len() == RECENT_FILE_LIMIT {
                break;
            }
        }
    }
    names
}

impl OpenPatch {
    fn remember_recent_file(&mut self, name: &str) {
        if !recent_file_name(name) || self.recent_files.first().is_some_and(|first| first == name) {
            return;
        }
        self.recent_files.retain(|existing| existing != name);
        self.recent_files.insert(0, name.into());
        self.recent_files.truncate(RECENT_FILE_LIMIT);
        self.save_host_settings();
    }

    fn recent_files_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Open recent", |ui| {
            if self.recent_files.is_empty() {
                ui.label("No recently opened patches yet");
                return;
            }
            for name in self.recent_files.clone() {
                let available = std::fs::symlink_metadata(Path::new(&self.workspace).join(&name))
                    .is_ok_and(|metadata| metadata.file_type().is_file());
                let label = if available {
                    name.clone()
                } else {
                    format!("{name} · unavailable")
                };
                let response = ui
                    .add_enabled(available, egui::Button::new(label))
                    .on_disabled_hover_text(
                        "This patch is no longer available in the project folder",
                    );
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(("recent-file-rect", &name)), response.rect)
                });
                if response.clicked() {
                    self.request_action(PendingAction::Load(name));
                    ui.close();
                }
            }
        });
    }
}
