// Original host adapter. The runner supplies the verbatim, GPL-licensed pinned
// Mixer class prefix as reference_module.hpp; no Mixer arithmetic is mirrored.
#include <rack.hpp>
#include "reference_module.hpp"
#include <array>
#include <cmath>
#include <cstdlib>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
using namespace rack;

static void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
static json_t* field(json_t* root,const char* key){auto* p=json_object_get(root,key);require(p,"missing field");return p;}
static int integer(json_t* p,int low,int high){require(json_is_integer(p),"integer required");auto n=json_integer_value(p);require(n>=low&&n<=high,"integer range");return int(n);}
static std::string string_value(json_t* p){require(json_is_string(p),"string required");return json_string_value(p);}
static bool boolean(json_t* p){require(json_is_boolean(p),"boolean required");return json_is_true(p);}
static uint32_t raw_bits(float value){uint32_t result;std::memcpy(&result,&value,sizeof(result));return result;}
static float from_bits(json_t* p){require(json_is_integer(p),"u32 float bits required");auto n=json_integer_value(p);require(n>=0&&uint64_t(n)<=UINT32_MAX,"u32 range");uint32_t b=uint32_t(n);float value;std::memcpy(&value,&b,sizeof(value));require(std::isfinite(value),"ordinary fixtures must be finite");return value;}
static void put(json_t* root,const char* key,json_t* p){require(p,"allocation failed");require(json_object_set_new(root,key,p)==0,"JSON insertion failed");}

struct HostSource : engine::Module {
    std::array<std::array<float,16>,6> held{};
    std::array<int,6> widths{};
    HostSource(){config(0,0,6,0);}
    void process(const ProcessArgs&)override{
        for(int p=0;p<6;++p){
            outputs[p].setChannels(widths[p]);
            for(int c=0;c<16;++c)outputs[p].setVoltage(c<widths[p]?held[p][c]:0.f,c);
        }
    }
};
struct HostSink : engine::Module {
    HostSink(){config(0,2,0,0);}
    void process(const ProcessArgs&)override{}
};
struct Harness {
    Context context;
    engine::Engine* engine=nullptr;
    HostSource* source=nullptr;
    Mixer* module=nullptr;
    HostSink* sink=nullptr;
    std::map<std::string,engine::Cable*> cables;
    int64_t nextCable=100;
    Harness(){
        require(contextGet()==nullptr,"unexpected Rack Context");contextSet(&context);
        try{engine=new engine::Engine;context.engine=engine;}catch(...){contextSet(nullptr);throw;}
    }
    ~Harness(){if(engine){engine->clear();context.engine=nullptr;delete engine;}contextSet(nullptr);}
    void setup(int rate){
        settings::threadCount=1;settings::cpuMeter=false;engine->setSampleRate(rate);
        auto s=std::make_unique<HostSource>();s->id=1;engine->addModule(s.get());source=s.release();
        auto m=std::make_unique<Mixer>();m->id=2;engine->addModule(m.get());module=m.release();
        auto k=std::make_unique<HostSink>();k->id=3;engine->addModule(k.get());sink=k.release();
    }
    engine::Module* named(const std::string& name){
        if(name=="source")return source;if(name=="module")return module;if(name=="sink")return sink;
        throw std::runtime_error("unknown role");
    }
    void configure_source(int port,json_t* array){
        require(json_is_array(array)&&json_array_size(array)<=16,"source array size");
        source->widths[port]=int(json_array_size(array));source->held[port].fill(0.f);
        for(int i=0;i<source->widths[port];++i){float v=from_bits(json_array_get(array,i));require(v>=-16.f&&v<=16.f,"source outside real poly_constant range");source->held[port][i]=v;}
    }
    void level(json_t* value){float v=from_bits(value);require(v>=0.f&&v<=1.f,"level range");engine->setParamValue(module,Mixer::LEVEL_PARAM,v);}
    void connect(const std::string& id,const std::string& src,int port,const std::string& dst,int input){
        require(!cables.count(id),"duplicate cable name");auto c=std::make_unique<engine::Cable>();
        c->id=nextCable++;c->outputModule=named(src);c->inputModule=named(dst);
        require(port>=0&&size_t(port)<c->outputModule->outputs.size(),"source port range");
        require(input>=0&&size_t(input)<c->inputModule->inputs.size(),"input port range");
        c->outputId=port;c->inputId=input;engine->addCable(c.get());cables.emplace(id,c.release());
    }
    void disconnect(const std::string& id){
        auto it=cables.find(id);require(it!=cables.end(),"unknown cable");require(engine->hasCable(it->second),"actual cable absent");
        engine->removeCable(it->second);delete it->second;cables.erase(it);
    }
    void initialize(json_t* init){
        auto* sources=field(init,"sources");require(json_is_array(sources)&&json_array_size(sources)==6,"six source controls required");
        for(int p=0;p<6;++p)configure_source(p,json_array_get(sources,p));
        level(field(init,"level_bits"));module->dataFromJson(field(init,"data"));
        int in=integer(field(init,"input_mask"),0,63),out=integer(field(init,"output_mask"),0,3);
        for(int p=0;p<6;++p)if(in&(1<<p))connect("in"+std::to_string(p),"source",p,"module",p);
        for(int p=0;p<2;++p)if(out&(1<<p))connect("out"+std::to_string(p),"module",0,"sink",p);
    }
    void event(json_t* e){
        const auto op=string_value(field(e,"op"));
        if(op=="step"){engine->stepBlock(1);return;}
        if(op=="source"){configure_source(integer(field(e,"port"),0,5),field(e,"values_bits"));return;}
        if(op=="level"){level(field(e,"value_bits"));return;}
        if(op=="data_load"){module->dataFromJson(field(e,"value"));return;}
        if(op=="save"){auto* data=module->dataToJson();require(data,"save failure");json_decref(data);return;}
        if(op=="reset"){engine->resetModule(module);return;}
        if(op=="bypass"){engine->bypassModule(module,boolean(field(e,"value")));return;}
        if(op=="connect"){connect(string_value(field(e,"id")),string_value(field(e,"source")),integer(field(e,"port"),0,5),string_value(field(e,"destination")),integer(field(e,"input"),0,5));return;}
        if(op=="disconnect"){disconnect(string_value(field(e,"id")));return;}
        if(op=="adopt"){
            // Rack keeps this actual module alive; the native counterpart rebuilds
            // and adopts the same incarnation with receiving controls winning.
            level(field(e,"level_bits"));auto* data=json_object();
            put(data,"invert",json_boolean(boolean(field(e,"invert"))));
            put(data,"average",json_boolean(boolean(field(e,"average"))));
            module->dataFromJson(data);json_decref(data);return;
        }
        if(op=="recreate"){
            std::vector<std::string> remove;
            for(auto& entry:cables)if(entry.second->inputModule==module||entry.second->outputModule==module)remove.push_back(entry.first);
            for(auto& id:remove)disconnect(id);
            engine->removeModule(module);delete module;auto m=std::make_unique<Mixer>();m->id=2;
            engine->addModule(m.get());module=m.release();return;
        }
        throw std::runtime_error("unknown event");
    }
    template<class Ports>static json_t* ports(const Ports& values){
        auto* list=json_array();for(const auto& p:values){auto* row=json_object();put(row,"channels",json_integer(p.channels));
            auto* lanes=json_array();for(int i=0;i<16;++i)json_array_append_new(lanes,json_integer(raw_bits(p.voltages[i])));
            put(row,"values_bits",lanes);json_array_append_new(list,row);}return list;
    }
    template<size_t N>static json_t* counts(const std::array<int,N>& values){auto* a=json_array();for(int v:values)json_array_append_new(a,json_integer(v));return a;}
    json_t* snapshot(const std::string& id,int index,const std::string& op){
        auto* row=json_object();put(row,"case",json_string(id.c_str()));put(row,"event",json_integer(index));put(row,"op",json_string(op.c_str()));
        put(row,"frame",json_integer(engine->getFrame()));put(row,"bypassed",json_boolean(module->isBypassed()));
        put(row,"level_bits",json_integer(raw_bits(module->params[Mixer::LEVEL_PARAM].getValue())));
        put(row,"average",json_boolean(module->average));put(row,"invert",json_boolean(module->invert));put(row,"saved_data",module->dataToJson());
        put(row,"source_outputs",ports(source->outputs));put(row,"inputs",ports(module->inputs));put(row,"outputs",ports(module->outputs));put(row,"sink_inputs",ports(sink->inputs));
        auto* config=json_array();for(int p=0;p<6;++p){auto* a=json_array();for(int i=0;i<source->widths[p];++i)json_array_append_new(a,json_integer(raw_bits(source->held[p][i])));json_array_append_new(config,a);}put(row,"source_config",config);
        std::array<int,6> in{},src{};std::array<int,1> out{};std::array<int,2> dst{};
        for(auto id:engine->getCableIds()){auto* c=engine->getCable(id);require(c&&engine->hasCable(c),"invalid actual registry");
            if(c->inputModule==module)++in.at(c->inputId);if(c->outputModule==module)++out.at(c->outputId);
            if(c->outputModule==source)++src.at(c->outputId);if(c->inputModule==sink)++dst.at(c->inputId);}
        put(row,"input_cables",counts(in));put(row,"output_cables",counts(out));put(row,"source_cables",counts(src));put(row,"sink_cables",counts(dst));return row;
    }
};
static void emit(Harness& h,const std::string& id,int index,const std::string& op){
    std::unique_ptr<json_t,decltype(&json_decref)> row(h.snapshot(id,index,op),json_decref);
    char* text=json_dumps(row.get(),JSON_SORT_KEYS|JSON_COMPACT);require(text,"encode failed");std::cout<<text<<'\n';free(text);
}
int main(){
    try{std::string line;while(std::getline(std::cin,line)){
        require(!line.empty(),"blank case");json_error_t error{};
        std::unique_ptr<json_t,decltype(&json_decref)> c(json_loads(line.c_str(),JSON_REJECT_DUPLICATES,&error),json_decref);
        require(bool(c),"invalid JSON case");auto id=string_value(field(c.get(),"id"));auto* events=field(c.get(),"events");require(json_is_array(events),"events array required");
        Harness h;h.setup(integer(field(c.get(),"sample_rate"),8000,192000));h.initialize(field(c.get(),"initial"));emit(h,id,-1,"initial");
        for(size_t i=0;i<json_array_size(events);++i){auto* e=json_array_get(events,i);h.event(e);emit(h,id,int(i),string_value(field(e,"op")));}
    }require(std::cin.eof(),"stdin read failure");return 0;}
    catch(const std::exception& error){std::cerr<<"Stock Mix genuine Engine probe failed: "<<error.what()<<'\n';return 1;}
}
