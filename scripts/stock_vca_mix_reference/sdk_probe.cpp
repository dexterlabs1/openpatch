// Original host adapter invoking the verbatim GPL-licensed VCMixer class prefix.
// The genuine Engine owns scheduling, cable arithmetic and processBypass.
#include <rack.hpp>
#include "reference_module.hpp"
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdlib>
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
static float from_bits(json_t* p){require(json_is_integer(p),"u32 bits required");auto n=json_integer_value(p);require(n>=0&&uint64_t(n)<=UINT32_MAX,"u32 range");uint32_t b=uint32_t(n);float value;std::memcpy(&value,&b,sizeof(value));require(std::isfinite(value),"finite control required");return value;}
static void put(json_t* root,const char* key,json_t* p){require(p,"allocation failed");require(json_object_set_new(root,key,p)==0,"JSON insertion failed");}

// Passive observation inside the actual Engine-owned source process. This
// adapter never writes FPCR or changes global floating point flags.
static void verify_engine_fpcr(){
#if defined(__aarch64__) || defined(__arm64__)
    uint64_t fpcr=0;__asm__ __volatile__("mrs %0, fpcr" : "=r" (fpcr));
    require((fpcr&(uint64_t(1)<<24))!=0,"Engine FPCR must enable FZ");
    require((fpcr&(uint64_t(3)<<22))==0,"Engine FPCR must use nearest rounding");
    static bool reported=false;if(!reported){std::cerr<<"{\"kind\":\"engine_fpcr\",\"fpcr\":"<<fpcr<<",\"fz\":true,\"rounding\":\"nearest\"}\n";reported=true;}
#else
    require(false,"strict genuine Engine oracle requires ARM64 FPCR observation");
#endif
}

struct HostSource : engine::Module {
    std::array<std::array<float,16>,9> held{};
    std::array<int,9> widths{};
    HostSource(){config(0,0,9,0);}
    void process(const ProcessArgs&)override{
        verify_engine_fpcr();
        for(int p=0;p<9;++p){
            outputs[p].setChannels(widths[p]);
            for(int c=0;c<16;++c)outputs[p].setVoltage(c<widths[p]?held[p][c]:0.f,c);
        }
    }
};
struct HostSink : engine::Module {
    HostSink(){config(0,5,0,0);}
    void process(const ProcessArgs&)override{}
};
struct Harness {
    Context context;
    engine::Engine* engine=nullptr;
    HostSource* source=nullptr;
    VCMixer* module=nullptr;
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
        auto m=std::make_unique<VCMixer>();m->id=2;engine->addModule(m.get());module=m.release();
        auto k=std::make_unique<HostSink>();k->id=3;engine->addModule(k.get());sink=k.release();
        require(module->params.size()==5&&module->inputs.size()==9&&module->outputs.size()==5&&module->lights.size()==4,"actual VCMixer dimensions");
    }
    engine::Module* named(const std::string& name){
        if(name=="source")return source;if(name=="module")return module;if(name=="sink")return sink;
        throw std::runtime_error("unknown role");
    }
    const char* role(engine::Module* value){
        if(value==source)return "source";if(value==module)return "module";if(value==sink)return "sink";
        throw std::runtime_error("unrecognized actual registry module");
    }
    void configure_source(int port,json_t* array){
        require(json_is_array(array)&&json_array_size(array)<=16,"source array size");
        source->widths[port]=int(json_array_size(array));source->held[port].fill(0.f);
        for(int i=0;i<source->widths[port];++i)source->held[port][i]=from_bits(json_array_get(array,i));
    }
    void parameters(json_t* values){require(json_is_array(values)&&json_array_size(values)==5,"five parameters required");for(int p=0;p<5;++p)engine->setParamValue(module,p,from_bits(json_array_get(values,p)));}
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
        auto* sources=field(init,"sources");require(json_is_array(sources)&&json_array_size(sources)==9,"nine source controls required");
        for(int p=0;p<9;++p)configure_source(p,json_array_get(sources,p));
        parameters(field(init,"parameter_bits"));module->dataFromJson(field(init,"data"));
        int in=integer(field(init,"input_mask"),0,511),out=integer(field(init,"output_mask"),0,31);
        for(int p=0;p<9;++p)if(in&(1<<p))connect("in"+std::to_string(p),"source",p,"module",p);
        for(int p=0;p<5;++p)if(out&(1<<p))connect("out"+std::to_string(p),"module",p,"sink",p);
    }
    void event(json_t* e){
        const auto op=string_value(field(e,"op"));
        if(op=="step"){engine->stepBlock(1);return;}
        if(op=="source"){configure_source(integer(field(e,"port"),0,8),field(e,"values_bits"));return;}
        if(op=="parameter"){engine->setParamValue(module,integer(field(e,"port"),0,4),from_bits(field(e,"value_bits")));return;}
        if(op=="data_load"){module->dataFromJson(field(e,"value"));return;}
        if(op=="save"){auto* data=module->dataToJson();if(data)json_decref(data);return;}
        if(op=="reset"){engine->resetModule(module);return;}
        if(op=="bypass"){engine->bypassModule(module,boolean(field(e,"value")));return;}
        if(op=="connect"){connect(string_value(field(e,"id")),string_value(field(e,"source")),integer(field(e,"port"),0,8),string_value(field(e,"destination")),integer(field(e,"input"),0,8));return;}
        if(op=="disconnect"){disconnect(string_value(field(e,"id")));return;}
        if(op=="adopt"){
            // Same actual instance. Production counterpart must rebuild/adopt
            // its real Core graph, preserving runtime while receiving control wins.
            parameters(field(e,"parameter_bits"));module->dataFromJson(field(e,"data"));return;
        }
        if(op=="recreate"){
            std::vector<std::string> remove;
            for(auto& entry:cables)if(entry.second->inputModule==module||entry.second->outputModule==module)remove.push_back(entry.first);
            for(auto& id:remove)disconnect(id);
            engine->removeModule(module);delete module;auto m=std::make_unique<VCMixer>();m->id=2;
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
        put(row,"sample_rate",json_integer(int(engine->getSampleRate())));put(row,"sample_time_bits",json_integer(raw_bits(engine->getSampleTime())));
        auto* parametersJ=json_array();for(auto& p:module->params)json_array_append_new(parametersJ,json_integer(raw_bits(p.getValue())));put(row,"parameter_bits",parametersJ);
        put(row,"ch_exp",json_boolean(module->chExp));put(row,"mix_exp",json_boolean(module->mixExp));
        auto* meters=json_array();for(auto& meter:module->chMeters){auto* item=json_object();put(item,"v_bits",json_integer(raw_bits(meter.v)));put(item,"mode",json_integer(meter.mode));put(item,"lambda_bits",json_integer(raw_bits(meter.lambda)));json_array_append_new(meters,item);}put(row,"meters",meters);
        put(row,"divider_clock",json_integer(module->lightDivider.getClock()));put(row,"divider_division",json_integer(module->lightDivider.getDivision()));
        auto* lightsJ=json_array();for(auto& light:module->lights)json_array_append_new(lightsJ,json_integer(raw_bits(light.getBrightness())));put(row,"light_bits",lightsJ);
        auto* saved=module->dataToJson();put(row,"saved_data",saved?saved:json_null());
        put(row,"source_outputs",ports(source->outputs));put(row,"inputs",ports(module->inputs));put(row,"outputs",ports(module->outputs));put(row,"sink_inputs",ports(sink->inputs));
        auto* config=json_array();for(int p=0;p<9;++p){auto* a=json_array();for(int i=0;i<source->widths[p];++i)json_array_append_new(a,json_integer(raw_bits(source->held[p][i])));json_array_append_new(config,a);}put(row,"source_config",config);
        std::array<int,9> in{},src{};std::array<int,5> out{},dst{};
        std::map<std::string,engine::Cable*> actual;
        for(auto cableId:engine->getCableIds()){
            auto* c=engine->getCable(cableId);require(c&&engine->hasCable(c),"invalid actual registry");
            bool found=false;for(auto& entry:cables)if(entry.second==c){require(!found,"duplicate external cable mapping");actual.emplace(entry.first,c);found=true;}
            require(found,"actual cable has no external identity");
            if(c->inputModule==module)++in.at(c->inputId);if(c->outputModule==module)++out.at(c->outputId);
            if(c->outputModule==source)++src.at(c->outputId);if(c->inputModule==sink)++dst.at(c->inputId);
        }
        require(actual.size()==cables.size(),"declared/actual registry cardinality differs");
        auto* registry=json_array();for(auto& entry:actual){auto* c=entry.second;auto* item=json_object();
            put(item,"id",json_string(entry.first.c_str()));put(item,"source",json_string(role(c->outputModule)));
            put(item,"port",json_integer(c->outputId));put(item,"destination",json_string(role(c->inputModule)));
            put(item,"input",json_integer(c->inputId));json_array_append_new(registry,item);}
        int mask=0;for(int p=0;p<5;++p)if(out[p]>0)mask|=1<<p;put(row,"cable_registry",registry);put(row,"output_consumer_mask",json_integer(mask));
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
    catch(const std::exception& error){std::cerr<<"Stock VCA Mix genuine Engine probe failed: "<<error.what()<<'\n';return 1;}
}
