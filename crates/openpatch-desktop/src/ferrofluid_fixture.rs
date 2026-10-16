// Native CI uses actual offline output, without opening an audio device.
impl OpenPatch {
    fn prepare_ferrofluid_fixture(&mut self, ctx: &egui::Context) {
        assert!(self.visualizer.is_some(), "Bundled Ferrofluid GLSL must compile on the native driver");
        let mut engine = self.patch.compile().expect("Studio Mixer fixture compiles");
        let mut audio = vec![[0.0; 2]; 4096];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|v| v.is_finite()));
        let wave: [f32; 256] = std::array::from_fn(|i| {
            let frame = audio[audio.len() - 1024 + i * 4];
            (frame[0] + frame[1]) * 0.5
        });
        let peak = audio.iter().flatten().fold(0.0_f32, |p, v| p.max(v.abs()));
        assert!(peak > 0.00001 && wave.iter().any(|v| v.abs() > 0.00001));
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("ferrofluid-smoke-audio"), (wave, peak)));
        egui::Popup::close_all(ctx);
        self.library = ModuleLibrary::default();
        self.browser_open = false;
        self.select_analysis(Some(AnalysisTab::Visuals));
        self.visual_mode = 3;
        self.title = "Ferrofluid · restored native visualizer".into();
        self.status = "Ferrofluid · actual offline Studio Mixer output · native GLSL · no live audio device".into();
    }
}
