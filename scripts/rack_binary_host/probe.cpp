// OpenPatch existing-binary feasibility probe. Original adapter, MIT.
// Links the genuine Rack SDK runtime; NEVER compiles Bogaudio source.
#include <rack.hpp>
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <array>
#include <cstdio>
#include <memory>
#include <stdexcept>
#include <string>

using namespace rack;
static void require(bool ok, const char* message) {
    if (!ok) throw std::runtime_error(message);
}
struct JsonDelete { void operator()(json_t* p) const { if (p) json_decref(p); } };
using Json = std::unique_ptr<json_t, JsonDelete>;
static bool textIs(json_t* object,const char* key,const char* expected) {
    auto* value=json_object_get(object,key);
    return json_is_string(value) && std::string(json_string_value(value))==expected;
}
struct Source : engine::Module {
    std::array<float,16> values{};
    int width = 1;
    Source() { config(0,0,1,0); }
    void process(const ProcessArgs&) override {
        outputs[0].setChannels(width);
        for (int c=0; c<width; ++c) outputs[0].setVoltage(values[c],c);
    }
};
struct Sink : engine::Module { Sink() { config(0,1,0,0); } };
struct Host {
    Context context;
    engine::Engine* engine = nullptr;
    Source* source = nullptr;
    engine::Module* module = nullptr;
    Sink* sink = nullptr;
    Host() {
        contextSet(&context);
        engine = new engine::Engine;
        context.engine = engine;
        engine->setSampleRate(48000.f);
    }
    ~Host() {
        // Engine::clear destroys registered cables/modules. Keep APP and the
        // dynamic library alive until all plugin-owned virtual calls finish.
        engine->clear();
        context.engine = nullptr;
        delete engine;
        contextSet(nullptr);
    }
    void cable(engine::Module* from,int out,engine::Module* to,int in,int id) {
        auto c = std::make_unique<engine::Cable>();
        c->id=id;c->outputModule=from;c->outputId=out;c->inputModule=to;c->inputId=in;
        engine->addCable(c.get());c.release();
    }
    void setup(plugin::Model* model) {
        auto s=std::make_unique<Source>();s->id=1;engine->addModule(s.get());source=s.release();
        auto m=std::unique_ptr<engine::Module>(model->createModule());
        require(bool(m),"module factory returned null");
        require(m->params.size()==2 && m->inputs.size()==3 && m->outputs.size()==1,"unexpected Offset shape");
        m->id=2;engine->addModule(m.get());module=m.release();
        auto k=std::make_unique<Sink>();k->id=3;engine->addModule(k.get());sink=k.release();
        cable(source,0,module,2,10);cable(module,0,sink,0,11);
    }
    void options(bool first,bool unlimited) {
        Json data(module->dataToJson());
        require(json_is_object(data.get()),"custom state must be object");
        json_object_set_new(data.get(),"offset_first",json_boolean(first));
        json_object_set_new(data.get(),"disableOutputLimit",json_boolean(unlimited));
        module->dataFromJson(data.get());
    }
    Json observation(const char* name, int width, float start) {
        source->width=width;
        for(int c=0;c<width;++c) source->values[c]=start+float(c);
        // Settle the plugin's modulation cadence and genuine cable propagation.
        engine->stepBlock(2048);
        require(sink->inputs[0].getChannels()==width,"polyphonic cable width mismatch");
        Json result(json_object());
        json_object_set_new(result.get(),"name",json_string(name));
        json_object_set_new(result.get(),"width",json_integer(width));
        json_t* values=json_array();json_object_set_new(result.get(),"voltages",values);
        for(int c=0;c<width;++c) {
            float v=sink->inputs[0].getVoltage(c);
            require(std::isfinite(v),"nonfinite plugin output");
            json_array_append_new(values,json_real(v));
        }
        return result;
    }
};
int main(int argc,char** argv) {
    try {
        require(argc==4 || argc==5,"usage: probe PLUGIN_DIR PRIVATE_USER_DIR OUTPUT [RESTORE_JSON]");
        settings::headless=true;settings::threadCount=1;settings::cpuMeter=false;
        asset::userDir=argv[2];settings::settingsPath=asset::userDir+"/settings.json";
        json_error_t error{};
        Json manifest(json_load_file((std::string(argv[1])+"/plugin.json").c_str(),JSON_REJECT_DUPLICATES,&error));
        require(bool(manifest),"manifest parse failed");
        require(textIs(manifest.get(),"slug","Bogaudio"),"unexpected plugin");
        require(textIs(manifest.get(),"version","2.6.47"),"unexpected plugin version");
        void* handle=dlopen((std::string(argv[1])+"/plugin.dylib").c_str(),RTLD_NOW|RTLD_LOCAL);
        if(!handle) throw std::runtime_error(dlerror());
        auto init=reinterpret_cast<void(*)(plugin::Plugin*)>(dlsym(handle,"init"));
        require(init!=nullptr,"missing init export");
        // Intentionally retain registry/models/library until child exit. There
        // is no unload/reload support in this bounded feasibility executable.
        auto* plugin=new plugin::Plugin;
        plugin->path=argv[1];plugin->handle=handle;plugin->fromJson(manifest.get());
        Host host;
        init(plugin);plugin->modulesFromJson(json_object_get(manifest.get(),"modules"));
        // Module::fromJson resolves the saved plugin/model through Rack's
        // process registry. Do not invoke global plugin::init or scan others.
        plugin::plugins.push_back(plugin);
        auto* model=plugin->getModel("Bogaudio-Offset");require(model!=nullptr,"Offset model absent");
        host.setup(model);
        if(argc==5) {
            Json saved(json_load_file(argv[4],JSON_REJECT_DUPLICATES,&error));
            require(json_is_object(saved.get()),"restore parse failed");
            host.engine->moduleFromJson(host.module,saved.get());
        } else {
            host.engine->setParamValue(host.module,0,.25f);
            host.engine->setParamValue(host.module,1,.25f);
            host.options(true,true);
        }
        Json state(host.engine->moduleToJson(host.module));
        Json report(json_object());
        json_object_set_new(report.get(),"protocol",json_integer(1));
        json_object_set_new(report.get(),"rack_runtime",json_string(APP_VERSION.c_str()));
        json_t* runtimes=json_array();
        json_object_set_new(report.get(),"loaded_rack_runtimes",runtimes);
        for(uint32_t i=0;i<_dyld_image_count();++i) {
            std::string path=_dyld_get_image_name(i);
            if(path.size()>=13 && path.substr(path.size()-13)=="libRack.dylib")
                json_array_append_new(runtimes,json_string(path.c_str()));
        }
        Dl_info origin{};require(dladdr(reinterpret_cast<void*>(init),&origin)!=0,"cannot identify init image");
        json_object_set_new(report.get(),"loaded_plugin",json_string(origin.dli_fname));
        json_object_set_new(report.get(),"state",state.release());
        json_t* cases=json_array();json_object_set_new(report.get(),"cases",cases);
        json_array_append_new(cases,host.observation("saved_mono",1,4).release());
        json_array_append_new(cases,host.observation("saved_poly16",16,-5).release());
        json_array_append_new(cases,host.observation("saved_unlimited",1,100).release());
        host.options(false,false);
        json_array_append_new(cases,host.observation("order_changed",1,4).release());
        json_array_append_new(cases,host.observation("limit_enabled",1,100).release());
        host.engine->setParamValue(host.module,1,.5f);
        json_array_append_new(cases,host.observation("gain_changed",1,1).release());
        require(json_dump_file(report.get(),argv[3],JSON_INDENT(2)|JSON_SORT_KEYS)==0,"cannot write report");
        return 0;
    } catch(const std::exception& error) {
        std::fprintf(stderr,"binary-host probe: %s\n",error.what());return 1;
    }
}
