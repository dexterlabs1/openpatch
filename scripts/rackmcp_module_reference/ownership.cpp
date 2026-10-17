// SPDX-License-Identifier: MIT
// Original parameter-ownership audit. Compile unchanged SequencerModule.cpp as
// a separate TU and link the genuine Rack SDK; no replacement registry or DSP.
#include <rack.hpp>
#include <context.hpp>
#include <engine/Engine.hpp>
#include <random.hpp>
#include "rackside/SequencerModule.hpp"
#include <algorithm>
#include <cstring>
#include <cstdlib>
#include <iostream>
#include <memory>
#include <stdexcept>

rack::plugin::Plugin* pluginInstance=nullptr; // Unused widget asset global only.
extern rack::plugin::Model* modelSequencer;
static uint32_t bits(float value){uint32_t out;std::memcpy(&out,&value,4);return out;}
struct RestoreContext{rack::Context* previous=rack::contextGet();~RestoreContext(){rack::contextSet(previous);}};
struct RegisterPlugin{
    rack::plugin::Plugin* value;
    explicit RegisterPlugin(rack::plugin::Plugin* p):value(p){rack::plugin::plugins.push_back(value);}
    ~RegisterPlugin(){auto& r=rack::plugin::plugins;r.erase(std::remove(r.begin(),r.end(),value),r.end());}
};
struct Target:rack::engine::Module{Target(){config(1,0,0,0);configParam(0,0.f,1.f,1.f,"Ownership target");id=42;}};
struct JsonDeleter{void operator()(json_t* value)const{json_decref(value);}};
using Json=std::unique_ptr<json_t,JsonDeleter>;
static Json data(float value){
    // Fixed original fixture: silent pattern, constant raw lane, source ID42:0.
    Json result(json_object());json_object_set_new(result.get(),"version",json_integer(1));
    Json patterns(json_array());Json pattern(json_object());
    json_object_set_new(pattern.get(),"name",json_string("A"));json_object_set_new(pattern.get(),"length",json_integer(16));
    json_array_append(patterns.get(),pattern.get());json_object_set(result.get(),"patterns",patterns.get());
    Json lanes(json_array());Json lane(json_object());Json points(json_array());Json point(json_object());
    json_object_set_new(point.get(),"bar",json_integer(0));json_object_set_new(point.get(),"value",json_real(value));json_array_append(points.get(),point.get());
    json_object_set_new(lane.get(),"moduleId",json_string("42"));json_object_set_new(lane.get(),"paramId",json_integer(0));
    json_object_set_new(lane.get(),"mode",json_string("raw"));json_object_set(lane.get(),"points",points.get());
    json_array_append(lanes.get(),lane.get());json_object_set(result.get(),"lanes",lanes.get());return result;
}
static void load(rack::engine::Engine& engine,rackmcp::SequencerModule& module,json_t* value){
    Json envelope(module.toJson());if(!json_is_object(envelope.get()))throw std::runtime_error("invalid SDK module envelope");
    json_object_set(envelope.get(),"data",value);engine.moduleFromJson(&module,envelope.get());
}
static json_t* handle(rack::engine::ParamHandle* value,bool alive){
    json_t* out=json_object();json_object_set_new(out,"alive",json_boolean(alive));json_object_set_new(out,"known",json_boolean(value!=nullptr));
    // Deleted B's saved pointer is cleared before any snapshot is taken.
    if(alive&&value){json_object_set_new(out,"module_id",json_integer(value->moduleId));json_object_set_new(out,"param_id",json_integer(value->paramId));json_object_set_new(out,"resolved_module_id",json_integer(value->module?value->module->id:-1));}
    return out;
}
int main(){
    if(rack::APP_VERSION!="2.6.6")throw std::runtime_error("requires genuine Rack SDK2.6.6");rack::random::init();
    RestoreContext restore;rack::Context context;rack::contextSet(&context);context.engine=new rack::engine::Engine;auto& engine=*context.engine;
    rack::plugin::Plugin plugin;plugin.slug="RackMCP";plugin.version="2.0.0";plugin.addModel(modelSequencer);RegisterPlugin registered(&plugin);
    auto a=std::make_unique<rackmcp::SequencerModule>();auto b=std::make_unique<rackmcp::SequencerModule>();
    a->id=100;b->id=200;a->model=modelSequencer;b->model=modelSequencer;
    a->params[rackmcp::SequencerModule::RUN_PARAM].setValue(1.f);b->params[rackmcp::SequencerModule::RUN_PARAM].setValue(1.f);
    Target target;engine.addModule(&target);engine.addModule(a.get());engine.addModule(b.get());
    Json dataA=data(.25f),dataB=data(.75f);rack::engine::ParamHandle* knownA=nullptr; rack::engine::ParamHandle* knownB=nullptr;
    rack::engine::Module::ProcessArgs args{};args.sampleRate=48000;args.sampleTime=1.f/48000;
    auto owner=[&](){auto* current=engine.getParamHandle(42,0);return !current?0:current==knownA?100:current==knownB?200:-1;};
    auto snapshot=[&](){json_t* out=json_object();json_object_set_new(out,"owner",json_integer(owner()));json_object_set_new(out,"target_bits",json_integer(bits(target.params[0].getValue())));json_object_set_new(out,"A",handle(knownA,bool(a)));json_object_set_new(out,"B",handle(knownB,bool(b)));return out;};
    Json report(json_object());json_object_set_new(report.get(),"sdk_version",json_string(rack::APP_VERSION.c_str()));json_object_set_new(report.get(),"sample_rate",json_integer(48000));json_t* stages=json_array();json_object_set_new(report.get(),"stages",stages);bool passed=true;
    auto observe=[&](const char* name,int expectedOwner,float expectedValue,int expectedAId,int expectedBId){
        const int immediateOwner=owner();
        json_t* stage=json_object();json_object_set_new(stage,"name",json_string(name));json_object_set_new(stage,"immediate",snapshot());
        for(int frame=0;frame<32;frame++){if(a)a->process(args);if(b)b->process(args);args.frame++;}
        json_object_set_new(stage,"after_32_frames",snapshot());json_t* expected=json_object();json_object_set_new(expected,"owner",json_integer(expectedOwner));json_object_set_new(expected,"target_bits",json_integer(bits(expectedValue)));json_object_set_new(expected,"A_handle_module_id",json_integer(expectedAId));if(b)json_object_set_new(expected,"B_handle_module_id",json_integer(expectedBId));json_object_set_new(stage,"expected",expected);
        bool ok=immediateOwner==expectedOwner&&owner()==expectedOwner&&bits(target.params[0].getValue())==bits(expectedValue)&&knownA&&knownA->moduleId==expectedAId;
        if(b&&knownB)ok&=knownB->moduleId==expectedBId;
        json_object_set_new(stage,"passed",json_boolean(ok));json_array_append_new(stages,stage);passed&=ok;
    };
    load(engine,*a,dataA.get());knownA=engine.getParamHandle(42,0);observe("A_acquires",100,.25f,42,-1);
    load(engine,*b,dataB.get());knownB=engine.getParamHandle(42,0);observe("B_takes_over",200,.75f,-1,42);
    load(engine,*b,dataB.get());observe("equal_incumbent_B_reload_self_unbinds",0,.75f,-1,-1);
    load(engine,*a,dataA.get());observe("displaced_A_reloads_and_acquires",100,.25f,42,-1);
    load(engine,*b,dataB.get());observe("B_takes_over_again",200,.75f,-1,42);
    engine.removeModule(b.get());b.reset();knownB=nullptr;observe("delete_winner_B_does_not_restore_A",0,.75f,-1,-1);
    json_object_set_new(report.get(),"frames",json_integer(args.frame));json_object_set_new(report.get(),"passed",json_boolean(passed));
    engine.removeModule(a.get());a.reset();knownA=nullptr;engine.removeModule(&target);
    char* serialized=json_dumps(report.get(),JSON_INDENT(2)|JSON_SORT_KEYS);if(!serialized)return 2;std::cout<<serialized<<'\n';std::free(serialized);return passed?0:1;
}
