// OpenPatch original host harness (MIT); executes separately pinned originals.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <stdexcept>
#define private public
#include "marbles/random/t_generator.h"
#include "marbles/random/x_y_generator.h"
#include "marbles/scale_recorder.h"
#include "marbles/note_filter.h"
#undef private
#include "preset_scales.h"
using namespace marbles;
static uint32_t bits(float v) { uint32_t u; std::memcpy(&u,&v,4);return u; }
static void require(bool v,const char* s) { if(!v)throw std::runtime_error(s); }
static void group(GroupSettings& g) {
 int cm,vr,rm;std::cin>>cm>>vr>>rm>>g.register_value>>g.spread>>g.bias>>g.steps>>g.deja_vu>>g.scale_index>>g.length>>g.ratio.p>>g.ratio.q;
 g.control_mode=ControlMode(cm);g.voltage_range=VoltageRange(vr);g.register_mode=rm;
 require(cm>=0&&cm<=2&&vr>=0&&vr<=2&&g.scale_index>=0&&g.scale_index<6&&g.length>=1&&g.length<=16&&g.ratio.p>0&&g.ratio.q>0,"invalid group");
}
int main() { try {
 int mode;uint32_t seed;require(bool(std::cin>>mode>>seed),"header");
 static RandomGenerator random;static RandomStream stream;random.Init(seed);stream.Init(&random);
 if(mode==1) { // Real public calls reproduce float-to-word undefined endpoint.
  static RandomSequence sequence;sequence.Init(&stream);sequence.set_deja_vu(0);sequence.Record();
  std::cout<<"captured "<<sequence.NextValue(true,1.0f)<<std::endl;
  sequence.ReplayPseudoRandom(1);std::cout<<sequence.NextValue(false,0.0f)<<std::endl;return 0;
 }
 if(mode==2) { // Original X-only LoadScale leaves Y's quantizer empty.
  static XYGenerator xy;xy.Init(&stream,32000.0f);xy.LoadScale(0,preset_scales[0]);
  std::cout<<xy.output_channel_[3].Quantize(0.2f,1.0f)<<std::endl;return 0;
 }
 if(mode==5) { std::cout<<bits(stmlib::Interpolate(lut_raised_cosine,1.0f,256.0f))<<std::endl;return 0; }
 if(mode==3) { // Raw loop decisions, rewrite/replay and Clone use actual source.
  static RandomSequence a,b;a.Init(&stream);b.Init(&stream);
  int n;std::cin>>n;for(int i=0;i<n;++i){int op,length;float deja,v;uint32_t hash;std::cin>>op>>length>>deja>>v>>hash;
   a.set_length(length);a.set_deja_vu(deja);if(op==0)a.Record();if(op==1)a.Reset();if(op==2)a.ReplayShifted(hash);if(op==3)a.ReplayPseudoRandom(hash);if(op==4)b.Clone(a);if(op==5)a.Clone(b);
   float x=op==6?a.RewriteValue(v):a.NextValue(op==7,v);std::cout<<i<<' '<<bits(x)<<' '<<random.state_<<'\n';}
  return 0;
 }
 if(mode==4) { // Source NoteFilter and weighted ScaleRecorder, no native rewrite.
  static NoteFilter filter;static ScaleRecorder recorder;filter.Init();recorder.Init();
  int n;std::cin>>n;for(int i=0;i<n;++i){int op;float value;std::cin>>op>>value;
   if(op==0)recorder.NewNote(value);if(op==1)recorder.UpdateVoltage(value);if(op==2)recorder.AcceptNote();if(op==3)recorder.Clear();
   Scale scale={};bool valid=recorder.ExtractScale(&scale);float filtered=filter.Process(value);
   std::cout<<i<<' '<<bits(filtered)<<' '<<valid<<' '<<scale.num_degrees;
   for(int j=0;j<scale.num_degrees;++j)std::cout<<' '<<bits(scale.degree[j].voltage)<<' '<<int(scale.degree[j].weight);std::cout<<'\n';}
  return 0;
 }
 require(mode==0,"unknown mode");float sr;int blocks,load_y;std::cin>>sr>>blocks>>load_y;require(sr==32000&&blocks>0&&blocks<=20000,"dimensions");
 // All firmware objects are static zero-initialized before Init. Preserve80draws.
 static TGenerator t;static XYGenerator xy;t.Init(&stream,sr);xy.Init(&stream,sr);
 for(int slot=0;slot<6;++slot){xy.LoadScale(slot,preset_scales[slot]);if(load_y)xy.output_channel_[3].LoadScale(slot,preset_scales[slot]);}
 for(int block=0;block<blocks;++block){
  int model,range,length,ext,rt,rx,source;float rate,bias,jitter,pw,ps,deja;
  std::cin>>model>>range>>rate>>bias>>jitter>>pw>>ps>>deja>>length>>ext>>rt>>rx>>source;
  require(model>=0&&model<=6&&range>=0&&range<=2&&source>=0&&source<=4&&length>=1&&length<=16,"controls");
  GroupSettings x={},y={};group(x);group(y);stmlib::GateFlags tc[5],xc[5];int f;
  for(auto& flags:tc){std::cin>>f;flags=stmlib::GateFlags(f);}for(auto& flags:xc){std::cin>>f;flags=stmlib::GateFlags(f);}require(bool(std::cin),"block");
  float rb[4][5]={};Ramps ramps={rb[0],rb[1],{rb[2],rb[3]}};bool gates[10]={};float output[20]={};
  t.set_model(TGeneratorModel(model));t.set_range(TGeneratorRange(range));t.set_rate(rate);t.set_bias(bias);t.set_jitter(jitter);t.set_pulse_width_mean(pw);t.set_pulse_width_std(ps);t.set_deja_vu(deja);t.set_length(length);
  bool tr=rt,xr=rx;t.Process(ext,&tr,tc,ramps,gates,5);if(source!=CLOCK_SOURCE_EXTERNAL)xr|=tr;xy.Process(ClockSource(source),x,y,&xr,xc,ramps,output,5);
  for(int i=0;i<5;++i){std::cout<<block*5+i<<' '<<gates[i*2]<<' '<<(ramps.master[i]<0.5f)<<' '<<gates[i*2+1];for(int j=0;j<4;++j){require(std::isfinite(output[i*4+j]),"nonfinite output");std::cout<<' '<<bits(output[i*4+j]);}
   for(float* r:{ramps.master,ramps.slave[0],ramps.slave[1],ramps.external}){require(std::isfinite(r[i]),"nonfinite ramp");std::cout<<' '<<bits(r[i]);}std::cout<<' '<<random.state_<<' '<<tr<<' '<<xr<<'\n';}
 }
 std::string trailing;require(!(std::cin>>trailing),"trailing");return 0;
 }catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
