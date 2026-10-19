fn original_plaits_edit(node:&Node,name:&str,spec:&Value,revision:u64)->ParameterEdit {
    let value=node.params.get(name).copied().unwrap_or(spec["default"].as_f64().unwrap_or(0.0) as f32);
    ParameterEdit {id:node.id.clone(),kind:node.kind.clone(),parameter:name.into(),revision,surface:ParameterEditSurface::Window,menu_popup:None,stale:false,text:value.to_string(),min:spec["min"].as_f64().unwrap_or(0.0) as f32,max:spec["max"].as_f64().unwrap_or(1.0) as f32,step:spec["step"].as_f64().map(|v|v as f32),display_scale:1.0,display_unit:spec["unit"].as_str().unwrap_or("").into(),slew_min:None,gain_power:None,raw_max:spec["max"].as_f64().unwrap_or(1.0) as f32,focus:true}
}
impl Rack {
    fn original_plaits_panel(&mut self,ui:&mut egui::Ui,node:&Node,module:Option<&Value>,rect:Rect,revision:u64,interactive:bool)->Option<RackAction> {
        let module=module?;let z=self.zoom;let painter=ui.painter().clone();let mut action=None;
        let engine=node.params.get("engine").copied().unwrap_or(4.0).clamp(0.0,15.0) as usize;
        for (index,x) in [23.32685,32.22764].into_iter().enumerate() {
            let button=crate::rack_vendor::piece_rect(rect,x,14.6539,5.2019);
            crate::rack_vendor::piece(&painter,button,"TL1105_0",0.0);
            if interactive {
                let response=ui.interact(button.expand(2.0*z),ui.id().with((&node.id,"original-model",index)),Sense::click());
                response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,if index==0{"Next pitched model"}else{"Next noise/percussion model"}));
                if response.clicked(){let next=if index==0{if engine>=8 {engine-8}else{(engine+1)%8}}else if engine<8{engine+8}else{(engine+1)%8+8};action=Some(RackAction::Command("set_parameter",json!({"id":node.id,"parameter":"engine","value":next,"expected_revision":revision})));}
                response.on_hover_text(module["params"]["engine"]["labels"][engine].as_str().unwrap_or("Plaits model"));
            }
        }
        for (name,x,y,size,base,fg) in [
            ("frequency",3.1577,20.21088,17.557,"Rogan3PSWhite","Rogan3PSWhite_fg"),
            ("harmonics",39.3327,20.21088,17.557,"Rogan3PSWhite","Rogan3PSWhite_fg"),
            ("timbre",4.04171,49.6562,13.44,"Rogan1PSWhite","Rogan1PSWhite_fg"),
            ("morph",42.71716,49.6562,13.44,"Rogan1PSWhite","Rogan1PSWhite_fg"),
            ("timbre_amount",7.88712,77.60705,6.047,"Trimpot_bg","Trimpot"),
            ("fm_amount",27.2245,77.60705,6.047,"Trimpot_bg","Trimpot"),
            ("morph_amount",46.56189,77.60705,6.047,"Trimpot_bg","Trimpot"),
        ] {
            let spec=&module["params"][name];let min=spec["min"].as_f64().unwrap_or(0.0) as f32;let max=spec["max"].as_f64().unwrap_or(1.0) as f32;
            let value=node.params.get(name).copied().unwrap_or(spec["default"].as_f64().unwrap_or(0.0) as f32);
            let normalized=((value-min)/(max-min)).clamp(0.0,1.0);
            let cap=crate::rack_vendor::piece_rect(rect,x,y,size);
            crate::rack_vendor::piece(&painter,cap,base,0.0);
            crate::rack_vendor::piece(&painter,cap,fg,(normalized-0.5)*std::f32::consts::PI*1.66);
            if interactive {
                let response=ui.interact(cap,ui.id().with(("knob",&node.id,name)),Sense::click_and_drag());
                response.widget_info(||egui::WidgetInfo::slider(ui.is_enabled(),value as f64,format!("{} {}",node.id,name)));
                let input=self.parameter_rotary_input(ui,&response,cap,normalized,self.shortcuts_enabled && self.editing.is_none() && !egui::Popup::is_any_open(ui.ctx()));
                let next=if input.reset {Some(spec["default"].as_f64().unwrap_or(0.0) as f32)}else if let Some(high)=input.boundary{Some(if high{max}else{min})}else if input.nudge!=0{Some((value+input.nudge as f32*(max-min)*0.01).clamp(min,max))}else{input.normalized.map(|n|min+n*(max-min))};
                if let Some(next)=next.filter(|v|*v!=value){let mut args=json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision});if let Some(group)=input.history_group{args["history_group"]=json!(group);}action=Some(RackAction::Command("set_parameter",args));}
                response.context_menu(|ui|{
                    if ui.button(format!("Edit {name}…")).clicked(){self.open_precision_editor(original_plaits_edit(node,name,spec,revision),cap);ui.close();}
                    if name=="morph" {for parameter in ["decay","level"] {if ui.button(format!("Edit {parameter}…")).clicked(){self.open_precision_editor(original_plaits_edit(node,parameter,&module["params"][parameter],revision),cap);ui.close();}}}
                });
                response.on_hover_text(format!("{}: {}\nDrag vertically. Double-click resets. Right-click for exact entry.",parameter_label(name),parameter_value(name,value,spec)));
            }
        }
        for index in 0..8 {
            let position=crate::rack_vendor::point(rect,30.29498,24.81649+index as f32*5.40025);
            let selected=index==engine%8;
            let color=if !selected {Color32::from_rgb(39,49,41)} else if engine<8 {Color32::from_rgb(107,230,94)}else{Color32::from_rgb(236,88,69)};
            self.lighting.circle(&painter,position,2.684*z,color,if selected{1.0}else{0.0},0.08);
        }
        action
    }
}
