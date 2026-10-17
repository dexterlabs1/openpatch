// SPDX-License-Identifier: MIT
// Original headless driver. SequencerModule.cpp is a separate untouched TU.
#include <rack.hpp>
#include <context.hpp>
#include <engine/Engine.hpp>
#include <random.hpp>
#include "rackside/SequencerModule.hpp"
#include <algorithm>
#include <cstring>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <vector>

rack::plugin::Plugin* pluginInstance = nullptr; // Link-only UI asset global; never used.
extern rack::plugin::Model* modelSequencer;
static uint32_t bits(float value) { uint32_t out; std::memcpy(&out,&value,4); return out; }
static float unbits(json_t* value) { uint32_t raw=json_integer_value(value); float out; std::memcpy(&out,&raw,4); return out; }
struct RestoreContext { rack::Context* previous=rack::contextGet(); ~RestoreContext(){rack::contextSet(previous);} };
struct RegisterPlugin {
    rack::plugin::Plugin* plugin;
    explicit RegisterPlugin(rack::plugin::Plugin* p):plugin(p){rack::plugin::plugins.push_back(plugin);}
    ~RegisterPlugin(){auto& registry=rack::plugin::plugins;registry.erase(std::remove(registry.begin(),registry.end(),plugin),registry.end());}
};
struct Target : rack::engine::Module {
    Target(json_t* descriptors) {
        config(2,0,0,0);
        for(int i=0;i<2;i++) {
            json_t* d=json_array_get(descriptors,i);
            auto* q=configParam(i,unbits(json_object_get(d,"min_bits")),unbits(json_object_get(d,"max_bits")),unbits(json_object_get(d,"default_bits")),"Reference target");
            q->snapEnabled=json_is_true(json_object_get(d,"snap"));
        }
        id=42;
    }
};
static void load(rack::engine::Engine& host,rackmcp::SequencerModule& seq,json_t* data) {
    // Off-trace audit only: repeated same-target updates can self-unbind in the
    // genuine SDK's overwrite path. Do not repair or replace source behavior.
    auto describe=[](rack::engine::ParamHandle* handle) {
        json_t* value=json_object();
        json_object_set_new(value,"present",json_boolean(handle!=nullptr));
        if(handle) {
            json_object_set_new(value,"module_id",json_integer(handle->moduleId));
            json_object_set_new(value,"param_id",json_integer(handle->paramId));
            json_object_set_new(value,"resolved_module_id",json_integer(handle->module?handle->module->id:-1));
        }
        return value;
    };
    rack::engine::ParamHandle* previous[2]={host.getParamHandle(42,0),host.getParamHandle(42,1)};
    json_t* audit=json_object();json_t* before=json_array();
    for(auto* handle:previous)json_array_append_new(before,describe(handle));
    json_object_set_new(audit,"handles_before",before);
    json_object_set_new(audit,"module_id_before",json_integer(seq.id));
    // Preserve the genuine SDK module envelope, including plugin/model/version,
    // ID and current controls; replace only the requested source data.
    json_t* root=seq.toJson();
    if(!json_is_object(root)) throw std::runtime_error("SDK Module::toJson did not produce an object");
    json_object_set(root,"data",data);
    host.moduleFromJson(&seq,root); json_decref(root);
    json_t* after=json_array();json_t* priorAfter=json_array();
    for(int i=0;i<2;i++) {
        json_array_append_new(after,describe(host.getParamHandle(42,i)));
        json_array_append_new(priorAfter,describe(previous[i]));
    }
    json_object_set_new(audit,"handles_after",after);
    json_object_set_new(audit,"previous_handles_after",priorAfter);
    json_object_set_new(audit,"module_id_after",json_integer(seq.id));
    json_object_set_new(audit,"target_registered",json_boolean(host.getModule(42)!=nullptr));
    json_object_set_new(audit,"data_after",seq.dataToJson());
    char* serialized=json_dumps(audit,JSON_COMPACT|JSON_SORT_KEYS);
    if(serialized){std::cerr<<serialized<<'\n';std::free(serialized);}json_decref(audit);
}
int main(int argc,char** argv) {
    if(argc!=3) return 2;
    if(rack::APP_VERSION!="2.6.6") throw std::runtime_error("requires genuine Rack SDK 2.6.6");
    rack::random::init(); // Real SDK process-global RNG initialization.
    json_error_t error; json_t* fixture=json_load_file(argv[1],0,&error);
    if(!fixture) throw std::runtime_error(error.text);
    RestoreContext restore; rack::Context context; rack::contextSet(&context);
    context.engine=new rack::engine::Engine; auto& host=*context.engine;
    // Actual registry and actual SDK ParamQuantity, with source IDs 42:0/42:1.
    rack::plugin::Plugin plugin; plugin.slug="RackMCP"; plugin.version="2.0.0";
    plugin.addModel(modelSequencer);
    RegisterPlugin registered(&plugin);
    auto seq=std::make_unique<rackmcp::SequencerModule>(); seq->id=100; seq->model=modelSequencer;
    rack::engine::Module producers,sink; producers.config(0,0,2,0); producers.id=101; sink.config(0,18,0,0); sink.id=102;
    auto target=std::make_unique<Target>(json_object_get(fixture,"descriptors"));
    host.addModule(seq.get()); host.addModule(&producers); host.addModule(&sink);
    bool live=json_is_true(json_object_get(fixture,"initial_live")); if(live) host.addModule(target.get());
    std::vector<std::unique_ptr<rack::engine::Cable>> outputCables;
    for(int i=0;i<18;i++) { auto c=std::make_unique<rack::engine::Cable>(); c->id=1000+i;c->outputModule=seq.get();c->outputId=i;c->inputModule=&sink;c->inputId=i;host.addCable(c.get());outputCables.push_back(std::move(c)); }
    rack::engine::Cable clockCable,resetCable;
    clockCable.id=2000;resetCable.id=2001;
    clockCable.outputModule=&producers;clockCable.outputId=0;clockCable.inputModule=seq.get();clockCable.inputId=0;
    resetCable.outputModule=&producers;resetCable.outputId=1;resetCable.inputModule=seq.get();resetCable.inputId=1;host.addCable(&resetCable);
    bool connected=false; float previousClock=0,previousReset=0;
    int version=json_integer_value(json_object_get(fixture,"initial_version"));
    json_t* versions=json_object_get(fixture,"versions"); load(host,*seq,json_array_get(versions,version));
    const float rate=json_number_value(json_object_get(fixture,"rate"));
    rack::engine::Module::ProcessArgs args{};args.sampleRate=rate;args.sampleTime=1.f/rate;
    std::ifstream input(argv[2]); std::string line;
    while(std::getline(input,line)) {
        int action,index,running,button,ext,present,manual;float clock,reset,manualValue;
        std::istringstream row(line);row>>action>>index>>running>>button>>ext>>clock>>reset>>present>>manual>>manualValue;
        if(!row) throw std::runtime_error("malformed packet");
        const bool newlyConnected=ext&&!connected;
        if(bool(ext)!=connected) { if(ext) host.addCable(&clockCable);else host.removeCable(&clockCable);connected=ext; }
        if(bool(present)!=live) { if(present) {target=std::make_unique<Target>(json_object_get(fixture,"descriptors"));host.addModule(target.get());} else host.removeModule(target.get());live=present; }
        if(action==1) {version=index;load(host,*seq,json_array_get(versions,version));}
        if(manual>=0&&live) target->params.at(manual).setValue(manualValue);
        seq->params[0].setValue(300.f);seq->params[1].setValue(63.25f);seq->params[2].setValue(running);seq->params[3].setValue(button);
        // Explicit declared native cable boundary: desired producer n reaches
        // the receiver on n+1. A new route starts at positive zero.
        seq->inputs[0].setVoltage(ext&&!newlyConnected?previousClock:0.f);
        seq->inputs[1].setVoltage(previousReset);
        seq->process(args);
        bool first=true;auto emit=[&](uint32_t v){if(!first) std::cout<<' ';first=false;std::cout<<v;};
        for(auto& port:seq->outputs) {int ch=port.getChannels();emit(ch);for(int c=0;c<16;c++)emit(bits(c<ch?port.getVoltage(c):0.f));}
        for(int p=0;p<2;p++)emit(live?bits(target->params[p].getValue()):0);emit(live);
        std::cout<<'\n';previousClock=clock;previousReset=reset;args.frame++;
    }
    if(connected) host.removeCable(&clockCable);host.removeCable(&resetCable);
    for(auto& c:outputCables)host.removeCable(c.get());
    if(live)host.removeModule(target.get());host.removeModule(&sink);host.removeModule(&producers);host.removeModule(seq.get());seq.reset();json_decref(fixture);
}
