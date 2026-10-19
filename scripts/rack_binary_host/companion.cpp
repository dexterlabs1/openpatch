// OpenPatch actual-existing-binary companion, protocol v2. Original code, MIT.
// The only plugin implementation is the explicitly supplied Bogaudio dylib.
#include "wire.h"
#include <rack.hpp>
#include <dlfcn.h>
#include <fcntl.h>
#include <mach-o/dyld.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>

using namespace rack;
namespace wire = openpatch_host;
namespace {

// Stable bounded diagnostic codes in header.error and an owned slot.error.
enum class Fault : uint32_t {
    Arguments=1, Mapping=2, Header=3, Plugin=4, Restore=5, Slot=6,
    Output=7, StateCapture=8, Internal=9
};
struct Failure : std::runtime_error {
    Fault code;
    Failure(Fault c,const char* message) : std::runtime_error(message),code(c) {}
};
void require(bool ok,Fault code,const char* message) {
    if(!ok) throw Failure(code,message);
}
struct JsonDelete { void operator()(json_t* p) const { if(p) json_decref(p); } };
using Json=std::unique_ptr<json_t,JsonDelete>;
constexpr size_t MAX_STATE_BYTES=1024*1024;

struct Mapping {
    uint8_t* bytes=nullptr;
    int fd=-1;
    ~Mapping() {
        if(bytes) munmap(bytes,wire::BYTES);
        if(fd>=0) close(fd);
    }
    void openFile(const char* path) {
        fd=open(path,O_RDWR|O_CLOEXEC|O_NOFOLLOW);
        require(fd>=0,Fault::Mapping,"cannot open shared file");
        struct stat info{};
        require(fstat(fd,&info)==0,Fault::Mapping,"cannot stat shared file");
        require(S_ISREG(info.st_mode) && info.st_size==off_t(wire::BYTES)
                && info.st_uid==getuid() && (info.st_mode&07777)==0600,
                Fault::Mapping,"shared file must be caller-owned regular 0600 with exact wire size");
        void* address=mmap(nullptr,wire::BYTES,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
        require(address!=MAP_FAILED,Fault::Mapping,"cannot map shared file");
        bytes=static_cast<uint8_t*>(address);
    }
};
bool zeroBytes(const uint8_t* p,size_t begin,size_t end) {
    for(size_t i=begin;i<end;++i) if(p[i]!=0) return false;
    return true;
}
bool textIs(json_t* object,const char* key,const char* expected) {
    json_t* value=json_object_get(object,key);
    return json_is_string(value) && std::string(json_string_value(value))==expected;
}
enum class Profile : uint32_t { Offset=1, XFade=2 };
const char* modelSlug(Profile profile) { return profile==Profile::Offset ? "Bogaudio-Offset" : "Bogaudio-XFade"; }
size_t parameterCount(Profile profile) { return profile==Profile::Offset ? 2 : 3; }
bool validParameter(Profile profile,uint16_t id,float value) {
    if(!std::isfinite(value) || id>=parameterCount(profile)) return false;
    if(profile==Profile::Offset || id==0) return value>=-1.f && value<=1.f;
    return value>=0.f && value<=1.f && (id!=2 || value==0.f || value==1.f);
}
struct Plan {
    uint64_t incarnation,generation,revision;
    uint32_t mask;
    Profile profile;
};
Plan validateHeader(const uint8_t* p) {
    require(std::memcmp(p,wire::MAGIC,8)==0
            && wire::get<uint32_t>(p,8)==wire::VERSION
            && wire::get<uint32_t>(p,12)==wire::BYTES
            && wire::get<uint32_t>(p,16)==wire::QUANTUM
            && wire::get<uint32_t>(p,20)==wire::RATE
            && wire::get<uint32_t>(p,24)==wire::SLOTS
            && wire::get<uint32_t>(p,28)==wire::EVENTS
            && wire::get<uint32_t>(p,56)==wire::INPUTS
            && wire::get<uint32_t>(p,60)==wire::OUTPUTS
            && wire::get<uint32_t>(p,64)==wire::CHANNELS,
            Fault::Header,"unsupported shared header protocol/layout/rate/ports");
    Plan plan{wire::get<uint64_t>(p,32),wire::get<uint64_t>(p,40),
              wire::get<uint64_t>(p,48),wire::get<uint32_t>(p,wire::INPUT_MASK),static_cast<Profile>(wire::get<uint32_t>(p,88))};
    require(plan.incarnation && plan.generation && plan.revision,
            Fault::Header,"header fences must be nonzero");
    require(plan.profile==Profile::Offset || plan.profile==Profile::XFade,Fault::Header,"unsupported capability ID");
    require((plan.mask&(plan.profile==Profile::Offset ? 4u : 2u))!=0 && (plan.mask&~7u)==0,
            Fault::Header,"unsupported fixed input connection mask");
    require(zeroBytes(p,92,wire::HEADER),Fault::Header,"nonzero reserved header bytes");
    // Mutable words are read atomically, never included in a header memcmp.
    require(wire::load(p,wire::STATUS)==wire::BOOT && wire::load(p,wire::STOP)==0
            && wire::load(p,wire::ERROR)==0 && wire::load(p,wire::PROCESSED)==0,
            Fault::Header,"shared header must start in clean BOOT state");
    return plan;
}

struct Source : engine::Module {
    const uint8_t* block=nullptr;
    uint32_t frame=0;
    Source() { config(0,0,wire::INPUTS,0); }
    void process(const ProcessArgs&) override {
        for(uint32_t p=0;p<wire::INPUTS;++p) {
            const size_t index=p*wire::QUANTUM+frame;
            const uint8_t channels=block[wire::COUNTS+index];
            outputs[p].setChannels(channels);
            for(uint32_t c=0;c<channels;++c)
                outputs[p].setVoltage(wire::get<float>(block,wire::INPUT_VALUES
                        +(index*wire::CHANNELS+c)*sizeof(float)),c);
        }
    }
};
struct Sink : engine::Module { Sink() { config(0,1,0,0); } };
struct Host {
    Context context;
    std::unique_ptr<engine::Engine> engine;
    Source* source=nullptr;
    engine::Module* module=nullptr;
    Sink* sink=nullptr;
    Host() {
        contextSet(&context);
        try {
            engine=std::make_unique<engine::Engine>();
            context.engine=engine.get();
            engine->setSampleRate(float(wire::RATE));
        } catch(...) {
            context.engine=nullptr;
            engine.reset();
            contextSet(nullptr);
            throw;
        }
    }
    ~Host() {
        // All plugin virtual calls finish with Context and the dylib alive.
        engine->clear();
        context.engine=nullptr;
        engine.reset();
        contextSet(nullptr);
    }
    void cable(engine::Module* from,int out,engine::Module* to,int in,int id) {
        auto c=std::make_unique<engine::Cable>();
        c->id=id;c->outputModule=from;c->outputId=out;c->inputModule=to;c->inputId=in;
        engine->addCable(c.get());c.release();
    }
    void setup(plugin::Model* model,uint32_t mask,Profile profile) {
        auto s=std::make_unique<Source>();s->id=1;
        engine->addModule(s.get());source=s.release();
        auto m=std::unique_ptr<engine::Module>(model->createModule());
        require(bool(m),Fault::Plugin,"module factory returned null");
        require(m->params.size()==parameterCount(profile) && m->inputs.size()==wire::INPUTS
                && m->outputs.size()==wire::OUTPUTS,Fault::Plugin,"unexpected qualified model shape");
        m->id=2;engine->addModule(m.get());module=m.release();
        auto k=std::make_unique<Sink>();k->id=3;
        engine->addModule(k.get());sink=k.release();
        for(uint32_t p=0;p<wire::INPUTS;++p)
            if(mask&(1u<<p)) cable(source,p,module,p,10+p);
        cable(module,0,sink,0,13);
    }
};

Json loadState(const char* path,Profile profile) {
    struct stat info{};
    require(stat(path,&info)==0 && S_ISREG(info.st_mode) && info.st_size>0
            && info.st_size<=off_t(MAX_STATE_BYTES),Fault::Restore,"state input missing or exceeds 1 MiB");
    json_error_t error{};
    Json saved(json_load_file(path,JSON_REJECT_DUPLICATES,&error));
    require(json_is_object(saved.get()) && json_object_size(saved.get())==6,Fault::Restore,"invalid state JSON object/fields");
    require(textIs(saved.get(),"plugin","Bogaudio")
            && textIs(saved.get(),"model",modelSlug(profile))
            && textIs(saved.get(),"version","2.6.47"),Fault::Restore,"state identity/version mismatch");
    require(json_is_integer(json_object_get(saved.get(),"id"))
            && json_integer_value(json_object_get(saved.get(),"id"))==2,
            Fault::Restore,"state module id must be 2");
    json_t* params=json_object_get(saved.get(),"params");
    require(json_is_array(params) && json_array_size(params)==parameterCount(profile),
            Fault::Restore,"state requires both Offset parameters");
    uint32_t seen=0;
    for(size_t i=0;i<parameterCount(profile);++i) {
        json_t* param=json_array_get(params,i);
        json_t* id=json_object_get(param,"id");
        json_t* value=json_object_get(param,"value");
        require(json_is_object(param) && json_object_size(param)==2 && json_is_integer(id) && json_is_number(value),
                Fault::Restore,"invalid state parameter");
        const json_int_t n=json_integer_value(id);
        const double v=json_number_value(value);
        require(n>=0 && n<json_int_t(parameterCount(profile)) && !(seen&(1u<<n)) && validParameter(profile,uint16_t(n),float(v)) && double(float(v))==v,
                Fault::Restore,"state parameter id/value out of range");
        seen|=1u<<n;
    }
    json_t* data=json_object_get(saved.get(),"data");
    require(json_is_object(data),Fault::Restore,"state custom data must be object");
    if(profile==Profile::Offset) {
        require(json_object_size(data)==2 && json_is_boolean(json_object_get(data,"offset_first"))
                && json_is_boolean(json_object_get(data,"disableOutputLimit")),Fault::Restore,"state requires exact Offset custom flags");
    } else require(json_object_size(data)==0,Fault::Restore,"XFade profile supports default custom state only");
    return saved;
}

std::string canonicalPluginPath(const char* path) {
    std::unique_ptr<char,decltype(&std::free)> resolved(realpath(path,nullptr),&std::free);
    require(bool(resolved),Fault::Plugin,"cannot canonicalize plugin image path");
    return resolved.get();
}

void writeIdentity(void (*init)(plugin::Plugin*),const char* pluginDir,const char* userDir,Profile profile) {
    Dl_info origin{};
    require(dladdr(reinterpret_cast<void*>(init),&origin)!=0 && origin.dli_fname,
            Fault::Plugin,"cannot identify plugin init image");
    const std::string expected=std::string(pluginDir)+"/plugin.dylib";
    require(canonicalPluginPath(origin.dli_fname)==canonicalPluginPath(expected.c_str()),
            Fault::Plugin,"init export did not originate in supplied plugin image");
    Json report(json_object());
    require(bool(report),Fault::Plugin,"cannot allocate identity receipt");
    auto field=[&](const char* key,json_t* value) {
        require(value && json_object_set_new(report.get(),key,value)==0,
                Fault::Plugin,"cannot construct identity receipt");
    };
    field("protocol",json_integer(wire::VERSION));
    field("rack_runtime",json_string(APP_VERSION.c_str()));
    field("loaded_plugin",json_string(origin.dli_fname));
    field("plugin_slug",json_string("Bogaudio"));
    field("plugin_version",json_string("2.6.47"));
    field("model_slug",json_string(modelSlug(profile)));
    field("profile_id",json_integer(static_cast<uint32_t>(profile)));
    field("sample_rate",json_integer(wire::RATE));
    field("quantum",json_integer(wire::QUANTUM));
    json_t* runtimes=json_array();
    field("loaded_rack_runtimes",runtimes);
    for(uint32_t i=0;i<_dyld_image_count();++i) {
        const char* image=_dyld_get_image_name(i);
        require(image!=nullptr,Fault::Plugin,"cannot identify loaded image");
        const std::string path=image;
        const size_t slash=path.find_last_of('/');
        if(path.substr(slash==std::string::npos ? 0 : slash+1)=="libRack.dylib")
            require(json_array_append_new(runtimes,json_string(image))==0,
                    Fault::Plugin,"cannot append runtime identity");
    }
    require(json_array_size(runtimes)==1,Fault::Plugin,"exactly one loaded Rack runtime is required");
    std::unique_ptr<char,decltype(&std::free)> encoded(
            json_dumps(report.get(),JSON_INDENT(2)|JSON_SORT_KEYS),&std::free);
    require(bool(encoded) && std::strlen(encoded.get())<=16384,
            Fault::Plugin,"identity receipt exceeds 16 KiB");
    require(json_dump_file(report.get(),(std::string(userDir)+"/identity.json").c_str(),
            JSON_INDENT(2)|JSON_SORT_KEYS)==0,Fault::Plugin,"cannot write identity receipt");
}

void validateSlot(const uint8_t* slot,const Plan& plan,uint64_t sequence) {
    require(wire::get<uint32_t>(slot,4)==0 && wire::get<uint64_t>(slot,8)==sequence
            && wire::get<uint64_t>(slot,16)==plan.incarnation
            && wire::get<uint64_t>(slot,24)==plan.generation
            && wire::get<uint64_t>(slot,32)==plan.revision,
            Fault::Slot,"input slot error/sequence/fence mismatch");
    require(zeroBytes(slot,44,64),Fault::Slot,"nonzero reserved slot header bytes");
    for(uint32_t p=0;p<wire::INPUTS;++p) {
        for(uint32_t f=0;f<wire::QUANTUM;++f) {
            const size_t index=p*wire::QUANTUM+f;
            const uint32_t count=slot[wire::COUNTS+index];
            require((plan.mask&(1u<<p)) ? count>=1 && count<=wire::CHANNELS : count==0,
                    Fault::Slot,"input channel count contradicts fixed cable plan");
            for(uint32_t c=0;c<wire::CHANNELS;++c) {
                const float v=wire::get<float>(slot,wire::INPUT_VALUES
                        +(index*wire::CHANNELS+c)*sizeof(float));
                require(std::isfinite(v) && (c<count || v==0.f),
                        Fault::Slot,"nonfinite or nonzero unused input voltage");
            }
        }
    }
    const uint32_t events=wire::get<uint32_t>(slot,40);
    require(events<=wire::EVENTS,Fault::Slot,"too many parameter events");
    uint16_t previousOffset=0;
    uint32_t previousOrder=0;
    for(uint32_t i=0;i<events;++i) {
        const uint8_t* event=slot+wire::EVENT_VALUES+i*16;
        const uint16_t offset=wire::get<uint16_t>(event,0);
        const uint16_t param=wire::get<uint16_t>(event,2);
        const float value=wire::get<float>(event,4);
        const uint32_t order=wire::get<uint32_t>(event,8);
        require(offset<wire::QUANTUM && validParameter(plan.profile,param,value) && wire::get<uint32_t>(event,12)==0,
                Fault::Slot,"invalid parameter event");
        require(i==0 || offset>previousOffset || (offset==previousOffset && order>previousOrder),
                Fault::Slot,"parameter events must be strictly sorted by (offset, order)");
        previousOffset=offset;previousOrder=order;
    }
    require(zeroBytes(slot,wire::EVENT_VALUES+events*16,wire::OUTPUT_COUNTS),
            Fault::Slot,"nonzero unused parameter event bytes");
}

void process(Host& host,uint8_t* slot) {
    const uint32_t events=wire::get<uint32_t>(slot,40);
    uint32_t nextEvent=0;
    host.source->block=slot;
    // Payload belongs exclusively to this child until OUTPUT_READY publication.
    std::memset(slot+wire::OUTPUT_COUNTS,0,wire::SLOT-wire::OUTPUT_COUNTS);
    for(uint32_t frame=0;frame<wire::QUANTUM;++frame) {
        while(nextEvent<events) {
            const uint8_t* event=slot+wire::EVENT_VALUES+nextEvent*16;
            if(wire::get<uint16_t>(event,0)!=frame) break;
            host.engine->setParamValue(host.module,wire::get<uint16_t>(event,2),
                    wire::get<float>(event,4));
            ++nextEvent;
        }
        host.source->frame=frame;
        // Preserve genuine Engine routing and Bogaudio's modulation cadence.
        host.engine->stepBlock(1);
        const int count=host.sink->inputs[0].getChannels();
        require(count>=0 && count<=int(wire::CHANNELS),Fault::Output,"invalid plugin output width");
        slot[wire::OUTPUT_COUNTS+frame]=uint8_t(count);
        for(int c=0;c<count;++c) {
            const float v=host.sink->inputs[0].getVoltage(c);
            require(std::isfinite(v),Fault::Output,"nonfinite plugin output voltage");
            wire::put<float>(slot,wire::OUTPUT_VALUES
                    +(frame*wire::CHANNELS+c)*sizeof(float),v);
        }
    }
    host.source->block=nullptr;
}

void capture(Host& host,const char* path) {
    Json state(host.engine->moduleToJson(host.module));
    require(json_is_object(state.get()),Fault::StateCapture,"module state serialization failed");
    std::unique_ptr<char,decltype(&std::free)> encoded(
            json_dumps(state.get(),JSON_INDENT(2)|JSON_SORT_KEYS),&std::free);
    require(bool(encoded) && std::strlen(encoded.get())<=MAX_STATE_BYTES,
            Fault::StateCapture,"serialized module state exceeds 1 MiB");
    require(json_dump_file(state.get(),path,JSON_INDENT(2)|JSON_SORT_KEYS)==0,
            Fault::StateCapture,"cannot write captured module state");
}

} // namespace

int main(int argc,char** argv) {
    const pid_t originalParent=getppid();
    Mapping shared;
    uint8_t* ownedSlot=nullptr;
    Fault stage=Fault::Arguments;
    try {
        require(argc==6,Fault::Arguments,
                "usage: companion PLUGIN_DIR PRIVATE_USER_DIR SHARED_FILE STATE_IN STATE_OUT");
        require(originalParent>1,Fault::Internal,"companion requires a live supervising parent");
        shared.openFile(argv[3]);
        const Plan plan=validateHeader(shared.bytes);
        stage=Fault::Plugin;
        require(APP_VERSION=="2.6.6",Fault::Plugin,"Rack runtime must be 2.6.6");
        settings::headless=true;settings::threadCount=1;settings::cpuMeter=false;
        asset::userDir=argv[2];settings::settingsPath=asset::userDir+"/settings.json";
        json_error_t error{};
        Json manifest(json_load_file((std::string(argv[1])+"/plugin.json").c_str(),
                JSON_REJECT_DUPLICATES,&error));
        require(json_is_object(manifest.get()) && textIs(manifest.get(),"slug","Bogaudio")
                && textIs(manifest.get(),"version","2.6.47"),
                Fault::Plugin,"plugin manifest must be Bogaudio 2.6.47");
        void* handle=dlopen((std::string(argv[1])+"/plugin.dylib").c_str(),RTLD_NOW|RTLD_LOCAL);
        if(!handle) {
            const char* diagnostic=dlerror();
            throw Failure(Fault::Plugin,diagnostic ? diagnostic : "plugin dlopen failed");
        }
        auto init=reinterpret_cast<void(*)(plugin::Plugin*)>(dlsym(handle,"init"));
        require(init!=nullptr,Fault::Plugin,"plugin init export missing");
        // Retain registry, models and handle through process exit, including all
        // Engine/module destruction. No scan, unload or reload path exists.
        auto* plugin=new plugin::Plugin;
        plugin->path=argv[1];plugin->handle=handle;plugin->fromJson(manifest.get());
        {
            Host host;
            init(plugin);
            plugin->modulesFromJson(json_object_get(manifest.get(),"modules"));
            plugin::plugins.push_back(plugin);
            auto* model=plugin->getModel(modelSlug(plan.profile));
            require(model!=nullptr,Fault::Plugin,"Offset model absent");
            host.setup(model,plan.mask,plan.profile);
            stage=Fault::Restore;
            Json saved=loadState(argv[4],plan.profile);
            host.engine->moduleFromJson(host.module,saved.get());
            Json restored(host.engine->moduleToJson(host.module));
            require(json_equal(saved.get(),restored.get()),Fault::Restore,
                    "restored persistent state differs from supplied full snapshot");
            stage=Fault::Plugin;
            writeIdentity(init,argv[1],argv[2],plan.profile);
            stage=Fault::Internal;
            wire::store(shared.bytes,wire::STATUS,wire::READY);
            uint64_t sequence=0;
            uint32_t processed=0;
            for(;;) {
                // Reparenting means the transport owner exited. The independent
                // mmap remains valid, so explicitly retire this orphan instead
                // of idling forever after a killed parent or runner timeout.
                require(getppid()==originalParent,Fault::Internal,"supervising parent exited");
                const uint32_t stop=wire::load(shared.bytes,wire::STOP);
                require(stop<=1,Fault::Header,"invalid stop request");
                if(stop) break;
                uint8_t* slot=shared.bytes+wire::HEADER+(sequence%wire::SLOTS)*wire::SLOT;
                const uint32_t state=wire::load(slot,0);
                require(state<=wire::WRITING && state!=wire::PROCESSING,
                        Fault::Slot,"invalid slot ownership state");
                if(state!=wire::INPUT_READY || !wire::claim(slot,wire::INPUT_READY,wire::PROCESSING)) {
                    usleep(50);
                    continue;
                }
                ownedSlot=slot;
                validateSlot(slot,plan,sequence);
                require(processed<std::numeric_limits<uint32_t>::max(),
                        Fault::Internal,"processed counter exhausted; restart required");
                process(host,slot);
                wire::put<uint32_t>(slot,4,0);
                wire::store(slot,0,wire::OUTPUT_READY);
                // No slot payload access after this release publication.
                ownedSlot=nullptr;
                ++sequence;++processed;
                wire::store(shared.bytes,wire::PROCESSED,processed);
            }
            stage=Fault::StateCapture;
            capture(host,argv[5]);
        }
        wire::store(shared.bytes,wire::STATUS,wire::STOPPED);
        return 0;
    } catch(const std::exception& error) {
        const auto* failure=dynamic_cast<const Failure*>(&error);
        const uint32_t code=uint32_t(failure ? failure->code : stage);
        if(ownedSlot) {
            std::memset(ownedSlot+wire::OUTPUT_COUNTS,0,wire::SLOT-wire::OUTPUT_COUNTS);
            wire::put<uint32_t>(ownedSlot,4,code);
            wire::store(ownedSlot,0,wire::OUTPUT_READY);
            ownedSlot=nullptr;
        }
        if(shared.bytes) {
            wire::store(shared.bytes,wire::ERROR,code);
            wire::store(shared.bytes,wire::STATUS,wire::FAULT);
        }
        std::fprintf(stderr,"binary-host companion fault %u: %.400s\n",code,error.what());
        return 1;
    } catch(...) {
        const uint32_t code=uint32_t(Fault::Internal);
        if(ownedSlot) {
            std::memset(ownedSlot+wire::OUTPUT_COUNTS,0,wire::SLOT-wire::OUTPUT_COUNTS);
            wire::put<uint32_t>(ownedSlot,4,code);
            wire::store(ownedSlot,0,wire::OUTPUT_READY);
        }
        if(shared.bytes) {
            wire::store(shared.bytes,wire::ERROR,code);
            wire::store(shared.bytes,wire::STATUS,wire::FAULT);
        }
        std::fprintf(stderr,"binary-host companion fault %u: unknown exception\n",code);
        return 1;
    }
}
