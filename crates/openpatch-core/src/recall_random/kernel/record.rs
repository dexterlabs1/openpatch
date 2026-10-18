// Copyright 2015 Emilie Gillet. MIT licensed; see LICENSE-THIRD-PARTY.
// Rust adaptation of marbles/{note_filter,scale_recorder}.h, pinned in the
// independent reference manifest. Authored scale publication is host-owned.
use super::quantizer::{Degree, Scale};
#[derive(Clone, Debug)]
pub struct NoteFilter { values:[f32;7],lp1:f32,lp2:f32 }
impl NoteFilter {
 pub fn new()->Self {Self{values:[0.;7],lp1:0.,lp2:0.}}
 pub fn process(&mut self,value:f32)->f32 {
  // Native callers validate finite CV before invoking the source filter.
  self.values.rotate_left(1);self.values[6]=value;
  let mut sorted=self.values;sorted.sort_unstable_by(|a,b|a.total_cmp(b));
  let value=sorted[3];self.lp1+=0.65*(value-self.lp1);self.lp2+=0.65*(self.lp1-self.lp2);self.lp2
 }
}
#[derive(Clone, Copy, Debug, Default)]
struct RecordedDegree {average:f32,total:f32,count:f32}
#[derive(Clone, Debug)]
pub struct ScaleRecorder {count:usize,current:f32,total_count:f32,degrees:[RecordedDegree;16]}
impl ScaleRecorder {
 pub fn new()->Self {Self{count:0,current:0.,total_count:0.,degrees:[RecordedDegree::default();16]}}
 pub fn clear(&mut self){self.count=0;self.current=0.;self.total_count=0.;}
 pub fn new_note(&mut self,v:f32){self.current=v;}
 pub fn update_voltage(&mut self,v:f32){self.current+=0.01*(v-self.current);}
 pub fn accept_note(&mut self){
  let mut v=self.current;while v<0. {v+=1.;}let octave=(v as i32) as f32;v-=octave;
  let mut nearest=None;
  for i in 0..self.count {let av=self.degrees[i].average;let tolerance=1./36.;
   if (v-av).abs()<tolerance {nearest=Some(i);break;}
   if ((v-1.)-av).abs()<tolerance {v-=1.;nearest=Some(i);break;}
  }
  if nearest.is_none()&&self.count!=16 {nearest=Some(self.count);self.degrees[self.count]=RecordedDegree::default();self.count+=1;}
  if let Some(i)=nearest {let d=&mut self.degrees[i];d.total+=v;d.count+=1.;d.average=d.total/d.count;self.total_count+=1.;}
 }
 pub fn extract_scale(&mut self)->Option<Scale> {
  if self.count<2 {return None;}
  self.degrees[..self.count].sort_unstable_by(|a,b|a.average.total_cmp(&b.average));
  let mut max_count=0_f32;for d in &self.degrees[..self.count] {max_count=max_count.max(d.count);}
  let mut scale=Scale{base_interval:1.,num_degrees:self.count,degree:[Degree{voltage:0.,weight:0};16]};
  for (dst,src) in scale.degree.iter_mut().zip(&self.degrees).take(self.count) {dst.voltage=src.average;dst.weight=(255.*src.count/max_count) as u8;if dst.weight==0 {dst.weight+=1;}}
  Some(scale)
 }
}
