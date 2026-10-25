// Notes and Blank share the actual rack/browser paint path. Drafts and resize
// previews stay local until one revision-checked Session command is accepted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BlankResizeProposal {
    pub id: String,
    pub before_hp: u16,
    pub width_hp: u16,
    pub movement: ModuleMove,
}

struct BlankResize {
    proposal: BlankResizeProposal,
    right: bool,
    delta: f32,
    blocked: bool,
}

struct AnnotationEdit {
    id: String,
    kind: String,
    revision: u64,
    text: String,
    focus: bool,
    stale: bool,
    error: Option<String>,
}

fn blank_resize_candidate(origin: Pos2, width_hp: u16, delta: f32, right: bool) -> (Pos2, u16) {
    let signed = if right { delta } else { -delta };
    let hp = (f32::from(width_hp) + signed / HP)
        .round()
        .clamp(
            f32::from(openpatch_core::MIN_BLANK_WIDTH_HP),
            f32::from(openpatch_core::MAX_BLANK_WIDTH_HP),
        ) as u16;
    let position = if right {
        origin
    } else {
        pos2(origin.x + f32::from(width_hp) * HP - f32::from(hp) * HP, origin.y)
    };
    (position, hp)
}

impl Rack {
    pub(crate) fn annotation_keyboard_active(&self) -> bool {
        self.annotation_edit.is_some() || self.annotation_input_guard
    }

    #[cfg(test)]
    pub(crate) fn annotation_edit_active(&self) -> bool {
        self.annotation_edit.is_some()
    }

    pub(crate) fn finish_annotation_edit(&mut self, accepted: bool) {
        if accepted { self.annotation_edit = None; }
        else if let Some(edit) = &mut self.annotation_edit {
            edit.error = Some("Could not apply the draft. Check the status bar, then correct or reload it.".into());
        }
    }

    pub(crate) fn open_annotation_editor(&mut self, node: &Node, revision: u64) {
        self.cancel_module_drag();
        self.annotation_input_guard = true;
        self.editing = None;
        self.annotation_edit = Some(AnnotationEdit {
            id: node.id.clone(),
            kind: node.kind.clone(),
            revision,
            text: if node.kind == "notes" {
                node.effective_notes_text().unwrap_or("").to_owned()
            } else {
                node.effective_blank_width_hp().unwrap_or(10).to_string()
            },
            focus: true,
            stale: false,
            error: None,
        });
    }

    fn annotation_geometry(&self, node: &Node) -> (Pos2, f32) {
        if let Some(drag) = self.blank_resize.as_ref().filter(|d| d.proposal.id == node.id) {
            return (drag.proposal.movement.after[&node.id], f32::from(drag.proposal.width_hp) * HP);
        }
        (self.positions[&node.id], module_width(node))
    }

    fn blank_fits(&self, patch: &Patch, id: &str, position: Pos2, width_hp: u16) -> bool {
        if !position.is_finite() || position.x.abs() > 100_000.0 || position.y.abs() > 100_000.0 {
            return false;
        }
        let width = f32::from(width_hp) * HP;
        patch.nodes.iter().filter(|n| n.id != id).all(|other| {
            self.positions.get(&other.id).is_none_or(|p| {
                position.x + width <= p.x || p.x + module_width(other) <= position.x
                    || position.y + PANEL_HEIGHT <= p.y || p.y + PANEL_HEIGHT <= position.y
            })
        })
    }

    pub(crate) fn validate_blank_resize(&self, patch: &Patch, proposal: &BlankResizeProposal) -> Result<(), String> {
        let node = patch.nodes.iter().find(|n| n.id == proposal.id && n.kind == "blank")
            .ok_or("Blank module was removed or replaced")?;
        if node.effective_blank_width_hp() != Some(proposal.before_hp) {
            return Err("Blank width changed; retry the resize".into());
        }
        if !(openpatch_core::MIN_BLANK_WIDTH_HP..=openpatch_core::MAX_BLANK_WIDTH_HP).contains(&proposal.width_hp)
            || proposal.movement.before.len() != 1 || proposal.movement.after.len() != 1
            || proposal.movement.before.keys().ne(proposal.movement.after.keys())
            || !proposal.movement.before.contains_key(&proposal.id)
            || self.positions.get(&proposal.id) != proposal.movement.before.get(&proposal.id)
        {
            return Err("Invalid or stale Blank resize geometry".into());
        }
        let before = proposal.movement.before[&proposal.id];
        let after = proposal.movement.after[&proposal.id];
        let right_before = before.x + f32::from(proposal.before_hp) * HP;
        let right_after = after.x + f32::from(proposal.width_hp) * HP;
        if before.y != after.y || (before.x != after.x && (right_before-right_after).abs() > 0.01) {
            return Err("Blank resize must retain one horizontal edge".into());
        }
        if !self.blank_fits(patch, &proposal.id, after, proposal.width_hp) {
            return Err("Blank resize overlaps another module or rack bounds".into());
        }
        Ok(())
    }

    fn annotation_panel(&mut self, ui: &mut egui::Ui, node: &Node, patch: &Patch, rect: Rect, revision: u64, drag_interactive: bool) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let width = rect.width() / z;
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        if node.kind == "notes" {
            let area = Rect::from_min_max(p(14.0, 81.0), p(width-14.0, 340.0));
            let emission = crate::lighting::next(&painter);
            AtlasVoicePaint { painter:&painter,origin:rect.min,layout:AtlasVoiceLayout {width},zoom:z }.screen(area,&self.lighting);
            let text = node.effective_notes_text().unwrap_or("");
            let (text, color) = if text.is_empty() {
                ("Patch notes\n\nDescribe the signal flow,\nperformance controls or\nhow this instrument works.", Color32::from_rgb(102, 136, 135))
            } else { (text, Color32::from_rgb(175, 214, 202)) };
            let clipped = painter.with_clip_rect(area.shrink(10.0*z));
            let job = egui::text::LayoutJob::simple(text.to_owned(), crate::panel_skin::print_font(painter.ctx(),14.0*z), color, area.width()-20.0*z);
            let galley = painter.layout_job(job);
            clipped.galley(area.min+vec2(10.0, 10.0)*z, galley, color);
            self.lighting.preserve(&painter, emission);
            let button = Rect::from_min_max(p(14.0, 351.0), p(width-14.0, 375.0));
            let response=ui.interact(button,ui.id().with(("notes-edit",&node.id)),Sense::click());
            paint_atlas_manual_button(&painter,button,z,false,response.hovered(),true,("EDIT NOTES",11.0));
            if response.clicked() {
                self.open_annotation_editor(node, revision);
            }
        } else {
            // A spacer stays visually quiet; narrow widths retain useful handles
            // without squeezing a full-size control or heading into three HP.
            if width >= 120.0 {
                let button = Rect::from_center_size(p(width*0.5, 210.0), vec2((width-30.0).min(170.0), 25.0)*z);
                let hp = self.blank_resize.as_ref().filter(|d| d.proposal.id==node.id)
                    .map_or_else(|| node.effective_blank_width_hp().unwrap_or(10), |d|d.proposal.width_hp);
                let response=ui.interact(button,ui.id().with(("blank-width",&node.id)),Sense::click());
                paint_atlas_manual_button(&painter,button,z,false,response.hovered(),true,(&format!("{hp} HP · SET WIDTH"),10.0));
                if response.clicked() {
                    self.open_annotation_editor(node, revision);
                }
                painter.text(p(width*0.5, 245.0), Align2::CENTER_CENTER, "Drag either edge", crate::panel_skin::print_font(painter.ctx(),10.0*z), MUTED);
            }
            for right in [false, true] {
                let x = if right { rect.right()-8.0*z } else { rect.left() };
                let handle = Rect::from_min_size(pos2(x, rect.top()+18.0*z), vec2(8.0*z, rect.height()-36.0*z));
                for offset in [3.0, 5.0] {
                    let xx = handle.left()+offset*z;
                    painter.line_segment([pos2(xx,handle.top()),pos2(xx,handle.bottom())], Stroke::new(0.6*z, Color32::from_gray(143)));
                }
                let response = ui.interact(handle, ui.id().with(("blank-resize", &node.id, right)), Sense::drag());
                if drag_interactive && !self.positions_locked && response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if drag_interactive && !self.positions_locked && response.drag_started_by(PointerButton::Primary) {
                    self.select_primary(node.id.clone());
                    let before = self.positions[&node.id];
                    let hp = node.effective_blank_width_hp().unwrap_or(10);
                    self.blank_resize = Some(BlankResize {
                        proposal: BlankResizeProposal { id:node.id.clone(), before_hp:hp, width_hp:hp,
                            movement:ModuleMove { revision, before:BTreeMap::from([(node.id.clone(),before)]), after:BTreeMap::from([(node.id.clone(),before)]) } },
                        right, delta:0.0, blocked:false,
                    });
                }
                if drag_interactive && response.dragged_by(PointerButton::Primary) {
                    if let Some(mut drag) = self.blank_resize.take().filter(|d| d.proposal.id==node.id && d.right==right) {
                        drag.delta += response.drag_delta().x / z;
                        let origin = drag.proposal.movement.before[&node.id];
                        let (position,hp) = blank_resize_candidate(origin,drag.proposal.before_hp,drag.delta,right);
                        drag.blocked = !self.blank_fits(patch,&node.id,position,hp);
                        if !drag.blocked {
                            drag.proposal.width_hp=hp;
                            drag.proposal.movement.after.insert(node.id.clone(),position);
                        }
                        self.blank_resize=Some(drag);
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if drag_interactive && response.drag_stopped_by(PointerButton::Primary) {
                    if let Some(drag) = self.blank_resize.take().filter(|d|d.proposal.id==node.id && d.right==right) {
                        if (drag.proposal.width_hp!=drag.proposal.before_hp || drag.proposal.movement.before!=drag.proposal.movement.after)
                            && self.validate_blank_resize(patch,&drag.proposal).is_ok() {
                            return Some(RackAction::ResizeBlank(drag.proposal));
                        }
                    }
                }
            }
            if let Some(drag)=self.blank_resize.as_ref().filter(|d|d.proposal.id==node.id) {
                painter.rect_stroke(rect.shrink(1.0*z),0,Stroke::new(2.0*z,if drag.blocked {Color32::from_rgb(203,95,79)} else {Color32::from_rgb(66,150,157)}),egui::StrokeKind::Inside);
            }
        }
        None
    }

    fn annotation_editor(&mut self, ctx: &egui::Context, patch: &Patch, revision:u64, action:&mut Option<RackAction>) {
        let Some(mut edit)=self.annotation_edit.take() else {return;};
        edit.stale |= edit.revision!=revision || !patch.nodes.iter().any(|n|n.id==edit.id && n.kind==edit.kind);
        let mut open=true;
        let mut apply=false;
        let mut cancel=ctx.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Escape));
        let mut reload=false;
        let notes=edit.kind=="notes";
        let mut validation: Option<String> = None;
        egui::Window::new(if notes {"Patch notes"} else {"Blank width"})
            .id(egui::Id::new(("annotation-editor",&edit.id,&edit.kind)))
            .open(&mut open).collapsible(false).default_width(if notes {470.0}else{330.0}).show(ctx,|ui|{
                ui.label(format!("{} · {}",edit.id,if notes {"Saved with this module"}else{"Left edge stays fixed"}));
                let response=if notes {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui,|ui|{
                        ui.add(egui::TextEdit::multiline(&mut edit.text).id(egui::Id::new(("annotation-text",&edit.id,&edit.kind))).desired_width(f32::INFINITY).desired_rows(14))
                    }).inner
                }else{
                    ui.add(egui::TextEdit::singleline(&mut edit.text).id(egui::Id::new(("annotation-text",&edit.id,&edit.kind))).desired_width(100.0))
                };
                if edit.focus {response.request_focus();edit.focus=false;}
                validation = if notes {
                    (edit.text.len()>openpatch_core::MAX_NOTES_BYTES).then(||format!("Text exceeds {} UTF-8 bytes",openpatch_core::MAX_NOTES_BYTES))
                } else {
                    edit.text.trim().parse::<u16>().ok().filter(|hp|(openpatch_core::MIN_BLANK_WIDTH_HP..=openpatch_core::MAX_BLANK_WIDTH_HP).contains(hp)).is_none().then(||"Enter a whole width from 3 to 96 HP".into())
                };
                if let Some(error) = &edit.error { ui.colored_label(Color32::from_rgb(235,174,102),error); }
                if notes {ui.small(format!("{} / {} UTF-8 bytes",edit.text.len(),openpatch_core::MAX_NOTES_BYTES));}
                if edit.stale {
                    ui.colored_label(Color32::from_rgb(235,174,102),"The patch changed. Your draft is retained; reload before applying.");
                    reload=ui.add_enabled(patch.nodes.iter().any(|n|n.id==edit.id && n.kind==edit.kind),egui::Button::new("Reload current content")).clicked();
                }
                if let Some(error)=&validation {ui.colored_label(Color32::from_rgb(235,140,120),error);}
                ui.horizontal(|ui|{
                    apply=ui.add_enabled(!edit.stale && validation.is_none(),egui::Button::new("Apply")).clicked();
                    cancel |=ui.button("Cancel").clicked();
                });
            });
        if cancel || !open {return;}
        if reload {
            if let Some(node)=patch.nodes.iter().find(|n|n.id==edit.id && n.kind==edit.kind) {
                self.open_annotation_editor(node,revision);
            }
            return;
        }
        if apply && !edit.stale && validation.is_none() {
            if notes {
                let unchanged = patch.nodes.iter().find(|n|n.id==edit.id && n.kind==edit.kind)
                    .is_some_and(|n|n.effective_notes_text()==Some(edit.text.as_str()));
                if unchanged {return;}
                *action=Some(RackAction::ApplyAnnotation(json!({"id":edit.id,"content":{"type":"notes","text":edit.text},"expected_revision":edit.revision})));
            } else if let Some(node) = patch.nodes.iter().find(|n|n.id==edit.id && n.kind==edit.kind) {
                let before=self.positions[&node.id];
                *action=Some(RackAction::ResizeBlank(BlankResizeProposal {
                    id:node.id.clone(), before_hp:node.effective_blank_width_hp().unwrap_or(10),
                    width_hp:edit.text.trim().parse::<u16>().unwrap(),
                    movement:ModuleMove {revision:edit.revision,before:BTreeMap::from([(node.id.clone(),before)]),after:BTreeMap::from([(node.id.clone(),before)])}
                }));
            }
        }
        self.annotation_edit=Some(edit);
    }
}
