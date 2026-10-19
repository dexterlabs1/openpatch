// Studio instruments: lit glass and meaningful configuration or signal views.
impl Rack {
    pub(crate) fn set_panel_screens(&mut self, frame: Option<Arc<crate::panel_screens::Frame>>) {
        self.panel_screen_frame=frame;
    }
    fn studio_screen(&self, painter:&egui::Painter,node:&Node,rect:Rect) {
        let Some(local)=crate::panel_screens::geometry(&node.kind,rect.width()/self.zoom) else {return};
        let z=self.zoom;
        let screen=Rect::from_min_max(rect.min+local.min.to_vec2()*z,rect.min+local.max.to_vec2()*z);
        painter.rect_filled(screen.expand(z),3.0*z,Color32::from_rgb(99,103,100));
        painter.rect_filled(screen,2.0*z,Color32::from_rgb(10,17,20));
        painter.line_segment([screen.left_top()+vec2(2.0,2.0)*z,screen.right_top()+vec2(-2.0,2.0)*z],Stroke::new(z,Color32::from_rgb(38,47,48)));
        let emission=crate::lighting::next(painter);
        let color=crate::panel_skin::luminous_accent(&node.kind);
        let plot=Rect::from_min_max(screen.min+vec2(6.0,if node.kind=="bass_glitch"{5.0}else{16.0})*z,screen.max-vec2(6.0,5.0)*z);
        for fraction in [0.25,0.5,0.75] {
            let x=egui::lerp(plot.left()..=plot.right(),fraction);
            painter.line_segment([pos2(x,plot.top()),pos2(x,plot.bottom())],Stroke::new(0.45*z,Color32::from_rgb(30,45,49)));
        }
        painter.line_segment([pos2(plot.left(),plot.center().y),pos2(plot.right(),plot.center().y)],Stroke::new(0.6*z,Color32::from_rgb(37,51,53)));
        let param=|name:&str,default:f32|node.params.get(name).copied().filter(|v|v.is_finite()).unwrap_or(default);
        let mut label="OUTPUT · ±10 V";
        let mut lines=Vec::<(Vec<Pos2>,Color32)>::new();
        if matches!(node.kind.as_str(),"ladder_vcf"|"lowpass"|"stock_vcf") {
            label="CUTOFF GUIDE";
            let cutoff=if node.kind=="stock_vcf"{stock_vcf_frequency_hz(param("frequency",0.5))}else{param("cutoff",1000.0).max(1.0)};
            let order=if node.kind=="ladder_vcf" {4.0} else {1.0};
            let points=(0..80).map(|i|{let x=i as f32/79.0;let hz=20.0*1000.0_f32.powf(x);let gain=(1.0+(hz/cutoff).powf(2.0*order)).sqrt().recip();pos2(egui::lerp(plot.left()..=plot.right(),x),plot.bottom()-gain*plot.height()*0.9)}).collect();
            lines.push((points,color));
        } else if node.kind=="adsr" {
            label="ENVELOPE SHAPE";
            let a=param("attack",0.01).max(0.001).sqrt();let d=param("decay",0.2).max(0.001).sqrt();let r=param("release",0.5).max(0.001).sqrt();let hold=(a+d+r)*0.3;let total=a+d+hold+r;
            let sustain=param("sustain",0.5).clamp(0.0,1.0);
            lines.push(([(0.0,0.0),(a/total,1.0),((a+d)/total,sustain),((a+d+hold)/total,sustain),(1.0,0.0)].map(|(x,y)|pos2(egui::lerp(plot.left()..=plot.right(),x),plot.bottom()-y*plot.height()*0.9)).to_vec(),color));
        } else if let Some(trace)=self.panel_screen_frame.as_ref().filter(|f|f.revision==self.observed_revision).and_then(|f|f.traces.get(&node.id)).filter(|t|t.count>1) {
            for lane in 0..usize::from(trace.channels) {
                if !trace.active[lane] {continue;}
                let count=trace.count;let first=(trace.cursor+crate::panel_screens::POINTS-count)%crate::panel_screens::POINTS;
                let points=(0..count).map(|i|{let value=trace.values[lane][(first+i)%crate::panel_screens::POINTS];pos2(egui::lerp(plot.left()..=plot.right(),i as f32/(count-1) as f32),plot.center().y-(value/10.0).clamp(-1.0,1.0)*plot.height()*0.47)}).collect();
                lines.push((points,if lane==0 {color} else {Color32::from_rgb(206,223,224)}));
            }
        } else {
            label=if node.bypassed {"BYPASSED"} else {"OUTPUT · WAITING"};
        }
        for (points,color) in lines {
            painter.add(egui::Shape::line(points.clone(),Stroke::new(4.0*z,color.gamma_multiply(0.12))));
            painter.add(egui::Shape::line(points,Stroke::new(1.15*z,color)));
        }
        if node.kind!="bass_glitch" {painter.text(screen.min+vec2(6.0,7.5)*z,Align2::LEFT_CENTER,label,FontId::monospace(7.3*z),color);}
        self.lighting.preserve(painter,emission);
    }
}
