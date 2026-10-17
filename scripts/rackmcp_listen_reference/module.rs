// SPDX-License-Identifier: MIT
// Original adapter, invoking the actual compiled native Engine model.
use openpatch_core::{Cable, Engine, ListenFrame, ListenSink, ModuleHandle, Node, Patch, PortFrame, HostPortLifecycle, OutputLifecycleTransform};
use std::{collections::BTreeMap, io::{self, Read}, str::SplitWhitespace};
const PORTS: [&str; 2] = ["left", "right"];
fn number<T: std::str::FromStr>(words: &mut SplitWhitespace<'_>) -> T where T::Err: std::fmt::Debug { words.next().unwrap().parse().unwrap() }
struct Capture { armed: bool, data: Vec<ListenFrame>, head: usize, tail: usize }
impl Capture { fn new() -> Self { Self { armed: false, data: vec![ListenFrame::default(); 1<<18], head: 0, tail: 0 } } }
impl ListenSink for Capture {
    fn armed(&self)->bool { self.armed }
    fn push(&mut self, frame:ListenFrame)->bool { if self.head-self.tail == self.data.len() {return false} let index=self.head & (self.data.len()-1); self.data[index] = frame; self.head+=1; true }
}
struct Host { patch:Patch, engine:Engine, handle:ModuleHandle, capture:Capture, sources:[PortFrame;2] }
impl Host {
    fn new(rate:u32,input_mask:usize,output_mask:usize)->Self {
        let mut patch=Patch::new_v2();patch.sample_rate=rate;
        for (id,kind) in [("source0","poly_constant"),("source1","poly_constant"),("module","rackmcp_listen"),("sink0","mult"),("sink1","mult")] {
            let mut params=BTreeMap::new();if kind=="poly_constant" {params.insert("channels".into(),0.0);}
            patch.nodes.push(Node{content:None,score:None,rackmcp_sequence:None,id:id.into(),kind:kind.into(),bypassed:false,params});
        }
        for (p,port) in PORTS.iter().enumerate() {
            for (mask,id,from,to,output,input) in [(input_mask,format!("in{p}"),format!("source{p}"),"module".into(),"out",*port),
                (output_mask,format!("out{p}"),"module".into(),format!("sink{p}"),*port,"signal")] {
                if mask & (1<<p) != 0 {patch.cables.push(Cable{input_order:None,output_order:None,color:None,id:Some(id),from,to,output:output.into(),input:input.into()});}
            }
        }
        let engine=patch.compile().unwrap();let handle=engine.resolve_module_state("module").unwrap();
        Self{patch,engine,handle,capture:Capture::new(),sources:[PortFrame::default();2]}
    }
    fn source(&mut self,p:usize,frame:PortFrame){self.sources[p]=frame;let h=self.engine.resolve_module_state(&format!("source{p}")).unwrap();assert!(self.engine.set_host_reference_source(h,frame));}
    fn bypass(&mut self,value:bool){
        let changed=self.patch.nodes[2].bypassed!=value;
        self.patch.nodes[2].bypassed=value;let mut next=self.patch.compile().unwrap();next.adopt_state_from(&self.engine);self.engine=next;self.handle=self.engine.resolve_module_state("module").unwrap();
        // Match Engine::bypassModule's synchronous host event using the actual
        // production ordered-lifecycle API, as the desktop does after adoption.
        // Direct adoption alone intentionally defers net context to next sample.
        let raw=self.engine.host_module_snapshot(self.handle).unwrap();
        let mut delta=HostPortLifecycle{input_presence:raw.input_presence,output_consumers:raw.output_consumers,bypassed:value,..HostPortLifecycle::default()};
        if changed {for port in 0..2 {delta.outputs[port]=OutputLifecycleTransform::bypass_edge();}}
        assert!(self.engine.apply_host_port_lifecycle(self.handle,&delta));
        for p in 0..2 {self.source(p,self.sources[p]);}
    }
    fn snapshot(&self){let s=self.engine.listen_status(self.handle).unwrap();let raw=self.engine.host_module_snapshot(self.handle).unwrap();let lights=self.engine.module_lights(self.engine.resolve_module_lights("module").unwrap()).unwrap();
        print!("{} {} {} {} {} {}",s.frames_seen,s.dropped,usize::from(s.left_connected),usize::from(s.right_connected),usize::from(s.capturing),lights.brightness[0][0].to_bits());
        for port in raw.inputs[..2].iter().chain(raw.outputs[..2].iter()){print!(" {}",port.channels);for v in port.values{print!(" {}",v.to_bits());}}
        println!(" {}",self.capture.head-self.capture.tail);
    }
}
fn main(){let mut text=String::new();io::stdin().read_to_string(&mut text).unwrap();let mut words=text.split_whitespace();let mut host:Option<Host>=None;
    while let Some(op)=words.next(){
        if op=="new" {host=Some(Host::new(number(&mut words),number(&mut words),number(&mut words)));}
        else {let h=host.as_mut().unwrap();match op {
            "source"=>{let p=number(&mut words);let n=number::<usize>(&mut words);let mut frame=PortFrame{channels:n as u8,..PortFrame::default()};for v in &mut frame.values[..n]{*v=f32::from_bits(number(&mut words));}h.source(p,frame);},
            "arm"=>h.capture.armed=number::<u8>(&mut words)!=0,
            "bypass"=>h.bypass(number::<u8>(&mut words)!=0),
            "step"=>{let n=number::<usize>(&mut words);for _ in 0..n {h.engine.process_poly_block_with_listen_capture(&mut [PortFrame::default();1],h.handle,&mut h.capture);}},
            "drain"=>{let n=number::<usize>(&mut words).min(h.capture.head-h.capture.tail);print!("drain {n}");for _ in 0..n {let f=h.capture.data[h.capture.tail&(h.capture.data.len()-1)];h.capture.tail+=1;print!(" {} {}",f.left.to_bits(),f.right.to_bits());}println!();},
            _=>panic!("unknown operation")}}
        host.as_ref().unwrap().snapshot();
    }
}
