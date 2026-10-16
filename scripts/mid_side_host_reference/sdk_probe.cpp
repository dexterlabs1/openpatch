// Independently written host adapter. reference_module.hpp is the verbatim
// pinned MidSide class prefix, supplied by the runner (no widget/model code).
#include <rack.hpp>
#include "reference_module.hpp"
#include <array>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
using namespace rack;

static void require(bool ok, const char* message) { if (!ok) throw std::runtime_error(message); }
static json_t* field(json_t* value, const char* key) { auto* v=json_object_get(value,key); require(v,"missing JSON field"); return v; }
static std::string str(json_t* v) { require(json_is_string(v),"expected string"); return json_string_value(v); }
static int integer(json_t* v,int lo,int hi) { require(json_is_integer(v),"expected integer"); auto n=json_integer_value(v); require(n>=lo && n<=hi,"integer outside declared bounds"); return int(n); }
static float number(json_t* v) { require(json_is_number(v),"expected number"); float n=float(json_number_value(v)); require(std::isfinite(n),"nonfinite fixture number"); return n; }
static void put(json_t* obj,const char* key,json_t* value) { require(value,"JSON allocation failed"); require(json_object_set_new(obj,key,value)==0,"JSON object insertion failed"); }

struct HostSource : engine::Module {
    std::array<std::array<float,16>,6> held{};
    std::array<int,6> widths{};
    HostSource() {config(0,0,6,0);}
    void process(const ProcessArgs&) override {
        for(int p=0;p<6;++p) {
            outputs[p].setChannels(widths[p]); // SDK owns connection/zero-width bookkeeping.
            for(int c=0;c<16;++c) outputs[p].setVoltage(c<widths[p]?held[p][c]:0.f,c);
        }
    }
};
struct HostSink : engine::Module {
    HostSink() {config(0,8,0,0);}
    void process(const ProcessArgs&) override {} // The genuine Engine owns all input writes.
};
struct Harness {
    Context context;
    engine::Engine* engine=nullptr;
    HostSource* source=nullptr;
    MidSide* module=nullptr;
    HostSink* sink=nullptr;
    std::map<std::string,engine::Cable*> cables;
    int64_t nextCable=100;
    Harness() {
        require(contextGet()==nullptr,"unexpected existing Rack Context");
        contextSet(&context);
        try {engine=new engine::Engine;context.engine=engine;}
        catch(...) {contextSet(nullptr);throw;}
    }
    ~Harness() {
        // Engine::clear deletes registered cables and modules. Removed cables
        // are deleted immediately by disconnect(). Context never owns this Engine.
        if(engine) {engine->clear();context.engine=nullptr;delete engine;}
        contextSet(nullptr);
    }
    void setup(int rate) {
        settings::threadCount=1;settings::cpuMeter=false;
        engine->setSampleRate(rate);
        auto s=std::make_unique<HostSource>();s->id=1;engine->addModule(s.get());source=s.release();
        auto m=std::make_unique<MidSide>();m->id=2;engine->addModule(m.get());module=m.release();
        auto k=std::make_unique<HostSink>();k->id=3;engine->addModule(k.get());sink=k.release();
    }
    engine::Module* named(const std::string& name) {
        if(name=="source")return source;if(name=="module")return module;if(name=="sink")return sink;
        throw std::runtime_error("unknown module role");
    }
    void event(json_t* e) {
        const auto op=str(field(e,"op"));
        if(op=="step") {engine->stepBlock(1);return;}
        if(op=="source") {
            int p=integer(field(e,"port"),0,5);auto* a=field(e,"values");
            require(json_is_array(a)&&json_array_size(a)<=16,"invalid held source array");
            source->widths[p]=int(json_array_size(a));source->held[p].fill(0.f);
            for(int c=0;c<source->widths[p];++c)source->held[p][c]=number(json_array_get(a,c));
            return; // Configuration only; never write a physical port outside process().
        }
        if(op=="param") {engine->setParamValue(module,integer(field(e,"index"),0,1),number(field(e,"value")));return;}
        if(op=="reset") {engine->resetModule(module);return;}
        if(op=="bypass") {auto* v=field(e,"value");require(json_is_boolean(v),"expected bypass bool");engine->bypassModule(module,json_is_true(v));return;}
        if(op=="connect") {
            auto id=str(field(e,"id"));require(!cables.count(id),"duplicate cable name");
            auto c=std::make_unique<engine::Cable>();c->id=nextCable++;
            c->outputModule=named(str(field(e,"source")));c->inputModule=named(str(field(e,"destination")));
            c->outputId=integer(field(e,"port"),0,int(c->outputModule->outputs.size())-1);
            c->inputId=integer(field(e,"input"),0,int(c->inputModule->inputs.size())-1);
            engine->addCable(c.get());cables.emplace(id,c.release());return;
        }
        if(op=="disconnect") {
            auto it=cables.find(str(field(e,"id")));require(it!=cables.end(),"missing cable name");
            require(engine->hasCable(it->second),"cable absent from genuine Engine");
            engine->removeCable(it->second);delete it->second;cables.erase(it);return;
        }
        throw std::runtime_error("unknown event op");
    }
    template<class Ports> static json_t* ports(const Ports& ports) {
        auto* list=json_array();
        for(const auto& p:ports) {
            auto* item=json_object();put(item,"channels",json_integer(p.channels));auto* values=json_array();
            for(int lane=0;lane<16;++lane) {require(std::isfinite(p.voltages[lane]),"nonfinite observed port voltage");json_array_append_new(values,json_real(p.voltages[lane]));}
            put(item,"values",values);json_array_append_new(list,item);
        } return list;
    }
    json_t* snapshot(const std::string& id,int eventIndex,const std::string& op) {
        auto* out=json_object();put(out,"case",json_string(id.c_str()));put(out,"event",json_integer(eventIndex));put(out,"op",json_string(op.c_str()));
        put(out,"frame",json_integer(engine->getFrame()));put(out,"bypassed",json_boolean(module->isBypassed()));
        put(out,"inputs",ports(module->inputs));put(out,"outputs",ports(module->outputs));put(out,"sink_inputs",ports(sink->inputs));put(out,"source_outputs",ports(source->outputs));
        auto* params=json_array();for(auto& p:module->params)json_array_append_new(params,json_real(p.getValue()));put(out,"params",params);
        auto* config=json_array();for(int p=0;p<6;++p){auto* a=json_array();for(int c=0;c<source->widths[p];++c)json_array_append_new(a,json_real(source->held[p][c]));json_array_append_new(config,a);}put(out,"source_config",config);
        std::array<int,6> inputCounts{};std::array<int,4> outputCounts{};
        // Query Engine's actual cable registry, rather than deriving topology from fixture bookkeeping.
        for(auto cableId:engine->getCableIds()) {auto* c=engine->getCable(cableId);require(c&&engine->hasCable(c),"invalid Engine cable registry");if(c->inputModule==module)++inputCounts.at(c->inputId);if(c->outputModule==module)++outputCounts.at(c->outputId);}
        auto* in=json_array();for(int n:inputCounts)json_array_append_new(in,json_integer(n));put(out,"input_cables",in);
        auto* oc=json_array();for(int n:outputCounts)json_array_append_new(oc,json_integer(n));put(out,"output_cables",oc);
        return out;
    }
};
int main() {
    try {
        std::string line;
        while(std::getline(std::cin,line)) {
            require(!line.empty(),"blank input line");json_error_t error{};
            std::unique_ptr<json_t,decltype(&json_decref)> c(json_loads(line.c_str(),JSON_REJECT_DUPLICATES,&error),json_decref);
            require(bool(c),"invalid case JSON");auto id=str(field(c.get(),"id"));auto* events=field(c.get(),"events");require(json_is_array(events),"expected event list");
            Harness h;h.setup(integer(field(c.get(),"sample_rate"),8000,192000));
            for(size_t i=0;i<json_array_size(events);++i) {
                auto* e=json_array_get(events,i);h.event(e);
                std::unique_ptr<json_t,decltype(&json_decref)> row(h.snapshot(id,int(i),str(field(e,"op"))),json_decref);
                char* encoded=json_dumps(row.get(),JSON_COMPACT|JSON_SORT_KEYS);require(encoded,"JSON encode failed");std::cout<<encoded<<'\n';free(encoded);
            }
        }
        require(std::cin.eof(),"stdin read failure");return 0;
    } catch(const std::exception& e) {std::cerr<<"MidSide genuine Engine probe failed: "<<e.what()<<'\n';return 1;}
}
