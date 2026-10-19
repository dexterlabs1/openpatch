// Import review stays in the workspace; it never opens an assistant or starts playback implicitly.
impl OpenPatch {
    fn import_dependencies_ui(&mut self, ctx: &egui::Context) {
        use openpatch::project_document::{
            AutomationOmissionReason, MappingFidelity, PlaybackMode, PortDirection,
        };
        let hosted: Vec<_> = self.project.graph.nodes.iter().filter_map(|node| {
            if let DocumentNode::Hosted { id, module, .. } = node { Some((id.clone(), module.clone())) } else { None }
        }).collect();
        if self.project.sources.is_empty() && hosted.is_empty() { return; }
        let hosted_status = self.shared.try_lock().ok()
            .map(|session| session.document_execution_status()["delayed_hosted_request"].clone())
            .unwrap_or(Value::Null);
        let dependencies: Vec<_> = self
            .project
            .graph
            .nodes
            .iter()
            .filter_map(|node| {
                let DocumentNode::Unresolved { id, source, ports } = node else {
                    return None;
                };
                let record = self.project.source_module(source).ok()?;
                Some((
                    id.clone(),
                    format!(
                        "{} / {}",
                        record["plugin"].as_str().unwrap_or("Unknown"),
                        record["model"].as_str().unwrap_or("Unknown")
                    ),
                    ports.clone(),
                    source.clone(),
                ))
            })
            .collect();
        let mode = self
            .shared
            .try_lock()
            .ok()
            .map(|session| session.playback_mode());
        let automation = match self.project.automation_omissions() {
            Ok(automation) => automation,
            Err(error) => {
                self.status = format!("Invalid imported automation: {error}");
                return;
            }
        };
        let suspended = mode == Some(PlaybackMode::Exact) && hosted_status["active"] != true
            && (!dependencies.is_empty() || !automation.is_empty() || !hosted.is_empty());
        let automation_count = format!(
            "{} omitted automation {}",
            automation.len(),
            if automation.len() == 1 {
                "lane"
            } else {
                "lanes"
            }
        );
        let mut command: Option<(&str, Value)> = None;
        egui::TopBottomPanel::bottom("import-status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(if !hosted.is_empty() {
                    format!("Experimental host · {} hosted · {}", hosted.len(), if hosted_status["active"] == true { "running" } else if hosted_status["status"] == "requested" { "requested" } else { "stopped" })
                } else if suspended {
                    format!(
                        "Playback suspended · {} unresolved modules · {automation_count}",
                        dependencies.len()
                    )
                } else {
                    format!(
                        "Imported project · {} unresolved modules · {automation_count}",
                        dependencies.len()
                    )
                });
                if ui
                    .small_button(if self.import_review_open {
                        "Hide review"
                    } else {
                        if hosted.is_empty() { "Review dependencies" } else { "Review playback" }
                    })
                    .clicked()
                {
                    self.import_review_open = !self.import_review_open;
                }
            });
        });
        if self.import_review_open {
            egui::SidePanel::right("import-dependencies").default_width(290.).min_width(240.).resizable(true).show(ctx,|ui| {
                ui.heading(if hosted.is_empty() { "Import review" } else { "Project playback" });
                ui.label(format!("{} native · {} unresolved",self.patch.nodes.len(),dependencies.len()));
                if suspended {ui.label("Exact playback waits for every module and automation target to be resolved.");}
                if !dependencies.is_empty() || !automation.is_empty() {ui.label("Partial playback omits unresolved modules, their routes and unbound automation. Sound can change.");}
                ui.horizontal(|ui| {
                    if ui.selectable_label(mode==Some(PlaybackMode::Exact),"Exact").clicked() {command=Some(("set_document_playback",json!({"mode":"exact"})));}
                    if ui.selectable_label(mode==Some(PlaybackMode::Partial),"Enable partial").clicked() {command=Some(("set_document_playback",json!({"mode":"partial"})));}
                });
                if !hosted.is_empty() {
                    ui.separator();
                    ui.strong("Experimental binary host");
                    ui.label("Pinned Bogaudio OFFSET only · 48 kHz · 128-frame buffers · 384 transport frames. Permission lasts for this session and graph.");
                    ui.label(if hosted_status["active"] == true { "Hosted audio running" }
                        else if hosted_status["status"] == "requested" { "Hosted execution requested; inspect performance for preparation or failure" }
                        else { "Hosted execution stopped" });
                    if ui.button(if hosted_status["status"] == "requested" { "Stop hosted playback" } else { "Start experimental hosted playback" }).clicked() {
                        command=Some(("set_hosted_execution",json!({"enabled":hosted_status["status"] != "requested"})));
                    }
                    if let Some(reason) = hosted_status["runtime"]["runtime"]["reason"].as_str() { ui.colored_label(Color32::from_rgb(201,159,92), reason); }
                    ui.small("Save briefly suspends execution while the companion state is captured. A failed capture never marks the project saved.");
                    for (id, module) in &hosted {
                        ui.label(format!("{id} · {} {}", module.identity.plugin, module.identity.version));
                    }
                }
                if self.patch.output.is_none() {ui.colored_label(Color32::from_rgb(201,159,92),"No native audio output selected.");}
                egui::ComboBox::from_id_salt("import-monitor").selected_text(self.patch.output.as_deref().unwrap_or("Choose monitor output")).show_ui(ui,|ui| {
                    for node in &self.patch.nodes {
                        let module=self.catalog["modules"].as_array().and_then(|modules|modules.iter().find(|module|module["kind"]==node.kind));
                        if let Some(ports)=module.and_then(|module|module["outputs"].as_array()) {
                            for port in ports.iter().filter_map(Value::as_str) {
                                if ui.selectable_label(self.patch.output.as_deref()==Some(node.id.as_str())&&self.patch.output_port==port,format!("{} · {} / {}",node.id,module_title(&node.kind),port)).clicked() {command=Some(("set_output",json!({"id":node.id,"port":port})));}
                            }
                        }
                    }
                });
                let substitutions=self.project.graph.nodes.iter().filter(|node|matches!(node,DocumentNode::Native{source:Some(source),..} if source.mapping.as_ref().is_some_and(|mapping|matches!(mapping.fidelity,MappingFidelity::UserSubstitution|MappingFidelity::MusicalRole)))).count();
                if substitutions>0 {ui.colored_label(Color32::from_rgb(201,159,92),format!("{substitutions} musical substitutions — source equivalence is not claimed."));}
                if !automation.is_empty() {
                    ui.separator();
                    ui.label(&automation_count);
                    egui::ScrollArea::vertical().id_salt("omitted-import-automation").max_height(140.).show(ui, |ui| {
                        for lane in &automation {
                            let reason = match lane.reason {
                                AutomationOmissionReason::UnboundLane => "no qualified control binding",
                                AutomationOmissionReason::MissingTarget => "target removed",
                                AutomationOmissionReason::UnresolvedTarget => "target unresolved",
                            };
                            ui.label(format!("{} → source {} / control {}: {reason}", lane.sequencer_id, lane.source_module_id, lane.source_param_id));
                        }
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical().id_salt("dependency-list").max_height(150.).show(ui,|ui| {
                    for (id,label,_,_) in &dependencies {
                        if ui.selectable_label(self.import_review.as_deref()==Some(id.as_str()),label).clicked() {
                            self.import_review=Some(id.clone());self.import_substitute_kind.clear();self.import_port_mappings.clear();
                        }
                    }
                });
                let selected=dependencies.iter().find(|(id,_,_,_)|self.import_review.as_deref()==Some(id.as_str()));
                if let Some((id,label,ports,source))=selected {
                    ui.separator();ui.strong(label);ui.small(id);
                    ui.label("Original source and assets remain in the project. Only observed source ports are shown.");
                    egui::ComboBox::from_id_salt("import-substitute-kind").selected_text(if self.import_substitute_kind.is_empty() {"Choose native substitute"} else {&self.import_substitute_kind}).show_ui(ui,|ui| {
                        if let Some(modules)=self.catalog["modules"].as_array() {for module in modules {
                            if let Some(kind)=module["kind"].as_str() {
                                if ui.selectable_label(self.import_substitute_kind==kind,module_title(kind)).clicked() {self.import_substitute_kind=kind.into();self.import_port_mappings.clear();}
                            }
                        }}
                    });
                    let target=self.catalog["modules"].as_array().and_then(|modules|modules.iter().find(|module|module["kind"]==self.import_substitute_kind));
                    if let Some(target)=target {
                        egui::ScrollArea::vertical().id_salt("dependency-port-map").max_height(180.).show(ui,|ui| {
                            for port in ports {
                                let key=port.key();
                                let available=&target[if port.direction==PortDirection::Input {"inputs"} else {"outputs"}];
                                ui.horizontal(|ui| {
                                    ui.label(&key);
                                    egui::ComboBox::from_id_salt(("import-port",&key)).selected_text(self.import_port_mappings.get(&key).map_or("Choose port",String::as_str)).show_ui(ui,|ui| {
                                        if let Some(available)=available.as_array() {for name in available.iter().filter_map(Value::as_str) {
                                            if ui.selectable_label(self.import_port_mappings.get(&key).is_some_and(|value|value==name),name).clicked() {self.import_port_mappings.insert(key.clone(),name.into());}
                                        }}
                                    });
                                });
                            }
                        });
                    }
                    ui.small("This uses native default parameters. It is an explicit musical substitution, not a verified model of the original.");
                    if ui.add_enabled(target.is_some()&&ports.iter().all(|port|self.import_port_mappings.contains_key(&port.key())),egui::Button::new("Use substitute (undoable)")).clicked() {
                        let inputs:BTreeMap<_,_>=self.import_port_mappings.iter().filter(|(key,_)|key.starts_with("vcv_in_")).map(|(k,v)|(k.clone(),v.clone())).collect();
                        let outputs:BTreeMap<_,_>=self.import_port_mappings.iter().filter(|(key,_)|key.starts_with("vcv_out_")).map(|(k,v)|(k.clone(),v.clone())).collect();
                        command=Some(("substitute_unresolved_module",json!({"id":id,"kind":self.import_substitute_kind,"inputs":inputs,"outputs":outputs})));
                    }
                    if ui.button("Remove placeholder + routes (undoable)").clicked() {command=Some(("remove_unresolved_module",json!({"id":id})));}
                    egui::CollapsingHeader::new("Retained original state").show(ui,|ui| {
                        if let Ok(record)=self.project.source_module(source) {
                            let text=serde_json::to_string_pretty(record).unwrap_or_default();
                            let preview:String=text.chars().take(4096).collect();
                            egui::ScrollArea::vertical().id_salt("dependency-source-preview").max_height(170.).show(ui,|ui| {ui.monospace(&preview);});
                            if text.chars().count()>4096 {ui.small("Preview truncated; complete source remains in the project.");}
                        }
                    });
                }
            });
        }
        if let Some((tool, mut args)) = command {
            args["expected_revision"] = json!(self.revision);
            self.status = match execute_shared(&self.shared, tool, args) {
                Ok(_) => if tool == "set_hosted_execution" { "Hosted playback request committed".into() } else { "Import change committed; original source retained.".into() },
                Err(error) => format!("Import change rejected: {error}"),
            };
            self.sync();
            self.observe_document();
        }
    }
}

impl OpenPatch {
    /// Publish committed geometry before a save or a local graph edit. The
    /// graph/state/generation guards prevent a stale view from decorating a
    /// newer MCP document with an older arrangement.
    fn publish_current_layout(&mut self) -> Result<(), String> {
        let mut live = self
            .shared
            .try_lock()
            .map_err(|_| "Project is busy; try again".to_string())?;
        if live.revision() != self.revision || live.state_id() != self.state_id {
            return Err("Project changed; synchronize before publishing its layout".into());
        }
        if live.layout_generation() != self.layout_generation {
            return Err("Project layout changed; synchronize before saving".into());
        }
        live.attach_layout_host()?;
        if live.layout_origin() == Some(openpatch::control::LayoutOrigin::SourceImport) {
            self.rack.restore_layout(
                &serde_json::to_value(&live.project_document().layout)
                    .map_err(|e| e.to_string())?,
            )?;
            self.rack
                .reconcile_document(&self.patch, live.project_document(), &self.catalog);
            self.rack
                .reflow_imported_source_layout(live.project_document());
        }
        let mut layout: ProjectLayout =
            serde_json::from_value(self.rack.layout()).map_err(|e| e.to_string())?;
        if !self.rack.navigation_idle() {
            layout.pan = live.project_document().layout.pan;
            layout.zoom = live.project_document().layout.zoom;
        }
        let intent = if live.layout_pending()
            || live.layout_origin().is_some()
            || layout.positions != live.project_document().layout.positions
        {
            Some(openpatch::control::LayoutIntent::Placement)
        } else if layout != live.project_document().layout {
            Some(openpatch::control::LayoutIntent::Camera)
        } else {
            None
        };
        if let Some(intent) = intent {
            let generation = live.layout_generation();
            live.publish_host_layout(self.revision, self.state_id, generation, layout, intent)?;
        }
        self.layout_generation = live.layout_generation();
        self.project.layout = live.project_document().layout.clone();
        Ok(())
    }
}
