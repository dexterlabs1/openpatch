// Real callback tests, included inside audio::tests. No device is opened.
fn stock_vcf_callback_patch(voices: usize) -> Patch {
    let mut p:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
      "nodes":[{"id":"chord","kind":"poly_constant","params":{"channels":voices}},
               {"id":"source","kind":"multi_vco","params":{"frequency":130.8128}},
               {"id":"filter","kind":"stock_vcf","params":{"frequency":0.65,"resonance":0.4}},
               {"id":"sum","kind":"poly_sum","params":{"level":0.02}}],
      "cables":[{"from":"chord","to":"source","input":"pitch"},
                {"from":"source","output":"saw","to":"filter","input":"signal"},
                {"from":"filter","output":"lowpass","to":"sum","input":"signal"}],
      "output":"sum"})).unwrap();p.assign_missing_cable_ids().unwrap();p
}
#[test]
fn stock_vcf_callback_first_buffer_and_graph_crossfade_have_no_heap_activity() {
    for voices in [1,16] {
        let mut patch=stock_vcf_callback_patch(voices);
        let (mut callback,mut updates,mut retired)=sequence_callback(&patch);
        let mut samples=[0.0_f32;512];
        without_heap_activity(||callback.render(&mut samples,2));
        assert!(samples.iter().all(|v|v.is_finite()));
        for (index,frequency) in [0.45,0.72,0.31].into_iter().enumerate() {
            patch.nodes[2].params.insert("frequency".into(),frequency);
            let revision=index as u64+2;
            callback.performance.sync_patch(&patch,revision);
            updates.push(sequence_graph(&callback,&patch,revision)).ok().unwrap();
            without_heap_activity(||callback.render(&mut samples,2));
            let timing=callback.performance.diagnostics.snapshot();
            assert!(timing.adopted_graph && timing.crossfade);
            assert!(samples.iter().all(|v|v.is_finite()));
            while retired.pop().is_ok() {}
        }
    }
}
#[test]
fn stock_vcf_callback_audio_is_independent_of_callback_partition() {
    let patch=stock_vcf_callback_patch(16);
    let (mut full,_,_)=sequence_callback(&patch);let (mut split,_,_)=sequence_callback(&patch);
    let mut a=[0.0_f32;2050];let mut b=a;
    without_heap_activity(||full.render(&mut a,2));
    let mut cursor=0;
    for frames in [0,1,3,64,127,128,129,573] {
        let end=cursor+frames*2;
        without_heap_activity(||split.render(&mut b[cursor..end],2));cursor=end;
    }
    assert_eq!(cursor,a.len());assert_eq!(a,b);
    assert!(a.iter().any(|v|v.abs()>0.0001));
}
