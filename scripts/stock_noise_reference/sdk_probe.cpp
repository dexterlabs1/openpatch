// Genuine Rack Engine/Sink adapter for a transformed pinned Fundamental Noise
// class. reference_module.hpp is generated only after independent freeze.
#include <rack.hpp>
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

static void require(bool ok, const char* reason) { if (!ok) throw std::runtime_error(reason); }
static json_t* field(json_t* obj, const char* key) {
    auto* p = json_object_get(obj, key); require(p, "missing field"); return p;
}
static int integer(json_t* p, int lo, int hi) {
    require(json_is_integer(p), "integer required"); auto n = json_integer_value(p);
    require(lo <= n && n <= hi, "integer out of range"); return int(n);
}
static std::string string_value(json_t* p) {
    require(json_is_string(p), "string required"); return json_string_value(p);
}
static bool boolean(json_t* p) { require(json_is_boolean(p), "Boolean required"); return json_is_true(p); }
static uint32_t word(json_t* p) {
    require(json_is_integer(p), "u32 required"); auto n=json_integer_value(p);
    require(n>=0 && uint64_t(n)<=UINT32_MAX, "u32 range"); return uint32_t(n);
}
static uint32_t bits(float x) { uint32_t b; std::memcpy(&b, &x, sizeof(b)); return b; }
static float from_bits(uint32_t b) { float x; std::memcpy(&x, &b, sizeof(x)); return x; }
static void put(json_t* obj, const char* key, json_t* value) {
    require(value && json_object_set_new(obj, key, value) == 0, "JSON write failed");
}

// The adapter controls call sites, not Fundamental's state machine. A single
// uniform cursor spans PinkNoiseGenerator and the black output.
struct DrawEvent { const char* kind; uint32_t value; };
struct DrawState {
    std::vector<uint32_t> normal_tape,uniform_tape;
    uint64_t normal_count = 0, uniform_count = 0;
    uint32_t last_normal = 0, last_uniform = 0;
    std::vector<DrawEvent> events;
};
static DrawState* draw_state = nullptr;
static float controlled_normal() {
    require(draw_state, "draw context absent");
    require(!draw_state->normal_tape.empty(),"normal tape absent");
    auto b = draw_state->normal_tape[(draw_state->normal_count++) % draw_state->normal_tape.size()];
    draw_state->last_normal = b; draw_state->events.push_back({"normal", b}); return from_bits(b);
}
static float controlled_uniform_pink() {
    require(draw_state, "draw context absent");
    require(!draw_state->uniform_tape.empty(),"uniform tape absent");
    auto b = draw_state->uniform_tape[(draw_state->uniform_count++) % draw_state->uniform_tape.size()];
    draw_state->last_uniform = b; draw_state->events.push_back({"pink", b}); return from_bits(b);
}
static float controlled_uniform_black() {
    require(draw_state, "draw context absent");
    require(!draw_state->uniform_tape.empty(),"uniform tape absent");
    auto b = draw_state->uniform_tape[(draw_state->uniform_count++) % draw_state->uniform_tape.size()];
    draw_state->last_uniform = b; draw_state->events.push_back({"black", b}); return from_bits(b);
}
#include "reference_module.hpp"

static void verify_engine_fpcr() {
#if defined(__aarch64__) || defined(__arm64__)
    uint64_t value = 0; __asm__ __volatile__("mrs %0, fpcr" : "=r"(value));
    require((value & (uint64_t(1) << 24)) != 0, "Engine FZ required");
    require((value & (uint64_t(3) << 22)) == 0, "Engine nearest rounding required");
    static bool printed = false;
    if (!printed) { std::cerr << "{\"engine_fpcr\":" << value << "}\n"; printed = true; }
#else
    require(false, "ARM64 Engine FPCR observation required");
#endif
}
struct HostSink : engine::Module {
    struct PortImage { uint8_t channels = 0; float voltages[16] = {}; };
    std::array<PortImage,7> seen{};
    HostSink() { config(0, 7, 0, 0); }
    void process(const ProcessArgs&) override {
        verify_engine_fpcr();
        for(int p=0;p<7;++p) {
            seen[p].channels=inputs[p].channels;
            for(int lane=0;lane<16;++lane) seen[p].voltages[lane]=inputs[p].voltages[lane];
        }
    }
};

struct Harness {
    Context context;
    engine::Engine* engine = nullptr;
    Noise* module = nullptr;
    HostSink* sink = nullptr;
    std::map<std::string, engine::Cable*> cables;
    int64_t next_cable = 100;
    int module_incarnation = 0;
    DrawState draws;
    Harness() {
        require(contextGet() == nullptr, "unexpected Rack Context"); contextSet(&context);
        try { engine = new engine::Engine; context.engine = engine; }
        catch (...) { contextSet(nullptr); throw; }
        draws.events.reserve(16); // Max one normal + eight Voss + one black.
        draw_state = &draws;
    }
    ~Harness() {
        draw_state = nullptr;
        if (engine) { engine->clear(); context.engine = nullptr; delete engine; }
        contextSet(nullptr);
    }
    void setup(int rate) {
        settings::threadCount = 1; settings::cpuMeter = false;
        engine->setSampleRate(rate);
        auto m = std::make_unique<Noise>(); m->id = 2; engine->addModule(m.get()); module = m.release();
        auto k = std::make_unique<HostSink>(); k->id = 3; engine->addModule(k.get()); sink = k.release();
        require(module->params.empty() && module->inputs.empty() && module->outputs.size() == 7 &&
                module->lights.empty(), "actual Noise dimensions");
    }
    void connect(const std::string& id, const std::string& src, int port,
                 const std::string& dst, int input) {
        require(!cables.count(id), "duplicate cable id");
        require(src == "module" && dst == "sink" && port >= 0 && port < 7 &&
                input >= 0 && input < 7, "unsupported cable route");
        auto c = std::make_unique<engine::Cable>(); c->id = next_cable++;
        c->outputModule = module; c->outputId = port;
        c->inputModule = sink; c->inputId = input;
        engine->addCable(c.get()); cables.emplace(id, c.release());
    }
    void disconnect(const std::string& id) {
        auto it = cables.find(id); require(it != cables.end(), "unknown cable id");
        require(engine->hasCable(it->second), "actual cable absent");
        engine->removeCable(it->second); delete it->second; cables.erase(it);
    }
    void initialize(json_t* init) {
        auto* normal=field(init,"normal_bits");auto* uniform=field(init,"uniform_bits");
        require(json_is_array(normal)&&json_array_size(normal)==8192,"normal tape size");
        require(json_is_array(uniform)&&json_array_size(uniform)==8192,"uniform tape size");
        for(size_t i=0;i<8192;++i) {
            draws.normal_tape.push_back(word(json_array_get(normal,i)));
            draws.uniform_tape.push_back(word(json_array_get(uniform,i)));
        }
        require(json_is_string(field(init,"normal_profile")),"profile name required");
        int mask = integer(field(init, "output_mask"), 0, 127);
        for (int p = 0; p < 7; ++p)
            if (mask & (1 << p)) connect("out" + std::to_string(p), "module", p, "sink", p);
    }
    void event(json_t* event) {
        draws.events.clear();
        std::string op = string_value(field(event, "op"));
        if (op == "step") {
            auto* count=json_object_get(event,"frames");
            int frames=count?integer(count,1,2048):1;
            for(int i=0;i<frames;++i) { draws.events.clear();engine->stepBlock(1); }
            return;
        }
        if (op == "connect") {
            connect(string_value(field(event,"id")), string_value(field(event,"source")),
                    integer(field(event,"port"),0,6), string_value(field(event,"destination")),
                    integer(field(event,"input"),0,6)); return;
        }
        if (op == "disconnect") { disconnect(string_value(field(event,"id"))); return; }
        if (op == "reset") { engine->resetModule(module); return; }
        if (op == "bypass") { engine->bypassModule(module, boolean(field(event,"value"))); return; }
        if (op == "adopt") { // Same actual module incarnation; no Noise controls exist.
            require(engine->hasModule(module), "adopted module absent"); return;
        }
        if (op == "recreate") {
            std::vector<std::string> incident;
            for (auto& entry : cables) if (entry.second->outputModule == module) incident.push_back(entry.first);
            for (auto& id : incident) disconnect(id);
            engine->removeModule(module); delete module;
            auto m = std::make_unique<Noise>(); m->id = 2; engine->addModule(m.get()); module = m.release();
            ++module_incarnation;
            return;
        }
        throw std::runtime_error("unknown event");
    }
    template<class Ports> static json_t* ports(const Ports& values) {
        auto* list = json_array();
        for (const auto& p : values) {
            auto* row = json_object(); put(row,"channels",json_integer(p.channels));
            auto* raw = json_array();
            for (int i = 0; i < 16; ++i) json_array_append_new(raw,json_integer(bits(p.voltages[i])));
            put(row,"values_bits",raw); json_array_append_new(list,row);
        }
        return list;
    }
    static json_t* raw_array(const float* data, size_t n) {
        auto* list = json_array();
        for (size_t i = 0; i < n; ++i) json_array_append_new(list,json_integer(bits(data[i])));
        return list;
    }
    json_t* snapshot(const std::string& id, int event_index, const std::string& op) {
        auto* row = json_object();
        put(row,"case",json_string(id.c_str())); put(row,"event",json_integer(event_index));
        put(row,"op",json_string(op.c_str())); put(row,"frame",json_integer(engine->getFrame()));
        put(row,"module_incarnation",json_integer(module_incarnation));
        put(row,"sample_rate",json_integer(int(engine->getSampleRate())));
        put(row,"sample_time_bits",json_integer(bits(engine->getSampleTime())));
        put(row,"bypassed",json_boolean(module->isBypassed()));
        put(row,"outputs",ports(module->outputs)); put(row,"sink_inputs",ports(sink->inputs));
        put(row,"sink_seen",ports(sink->seen));
        std::array<int,7> out{},in{}; std::map<std::string,engine::Cable*> actual;
        for (auto cable_id : engine->getCableIds()) {
            auto* c = engine->getCable(cable_id); require(c && engine->hasCable(c),"invalid actual cable");
            bool found = false;
            for (auto& entry : cables) if (entry.second == c) {
                require(!found,"duplicate external cable identity"); actual.emplace(entry.first,c); found=true;
            }
            require(found && c->outputModule == module && c->inputModule == sink,"unmapped actual cable");
            ++out.at(c->outputId); ++in.at(c->inputId);
        }
        require(actual.size() == cables.size(),"actual/external cable cardinality");
        auto* registry = json_array();
        for (auto& entry : actual) {
            auto* c = entry.second; auto* item = json_object();
            put(item,"id",json_string(entry.first.c_str())); put(item,"source",json_string("module"));
            put(item,"port",json_integer(c->outputId)); put(item,"destination",json_string("sink"));
            put(item,"input",json_integer(c->inputId)); json_array_append_new(registry,item);
        }
        put(row,"cable_registry",registry);
        auto* out_counts = json_array(); auto* in_counts = json_array(); int mask = 0;
        for (int p=0;p<7;++p) {
            json_array_append_new(out_counts,json_integer(out[p])); json_array_append_new(in_counts,json_integer(in[p]));
            if(out[p]) mask |= 1 << p;
        }
        put(row,"output_cables",out_counts); put(row,"sink_cables",in_counts);
        put(row,"output_consumer_mask",json_integer(mask));
        put(row,"normal_draws",json_integer(draws.normal_count));
        put(row,"uniform_draws",json_integer(draws.uniform_count));
        put(row,"last_normal_bits",json_integer(draws.last_normal));
        put(row,"last_uniform_bits",json_integer(draws.last_uniform));
        auto* draw_events = json_array();
        for (auto& draw : draws.events) {
            auto* item=json_object(); put(item,"kind",json_string(draw.kind));
            put(item,"bits",json_integer(draw.value)); json_array_append_new(draw_events,item);
        }
        put(row,"draw_events",draw_events);
        put(row,"red_x_bits",json_integer(bits(module->redFilter.x[0])));
        put(row,"red_y_bits",json_integer(bits(module->redFilter.y[0])));
        put(row,"last_white_bits",json_integer(bits(module->lastWhite)));
        put(row,"last_pink_bits",json_integer(bits(module->lastPink)));
        put(row,"pink_frame",json_integer(module->pinkNoiseGenerator.frame));
        put(row,"pink_values_bits",raw_array(module->pinkNoiseGenerator.values,8));
        put(row,"gray_frame",json_integer(module->grayFilter.frame));
        put(row,"gray_input_bits",raw_array(module->grayFilter.inputBuffer,1024));
        put(row,"gray_output_bits",raw_array(module->grayFilter.outputBuffer,1024));
        put(row,"black_divider_clock",json_integer(module->blackDivider.getClock()));
        put(row,"black_divider_division",json_integer(module->blackDivider.getDivision()));
        auto* lights = json_array(); for (auto& light : module->lights)
            json_array_append_new(lights,json_integer(bits(light.getBrightness())));
        put(row,"light_bits",lights); return row;
    }
};
static void emit(Harness& h,const std::string& id,int event,const std::string& op) {
    std::unique_ptr<json_t,decltype(&json_decref)> row(h.snapshot(id,event,op),json_decref);
    char* encoded=json_dumps(row.get(),JSON_SORT_KEYS|JSON_COMPACT);
    require(encoded,"JSON encode failed"); std::cout << encoded << '\n'; std::free(encoded);
}
int main() {
    try {
        std::string line;
        while (std::getline(std::cin,line)) {
            require(!line.empty(),"blank case line"); json_error_t error{};
            std::unique_ptr<json_t,decltype(&json_decref)> c(
                json_loads(line.c_str(),JSON_REJECT_DUPLICATES,&error),json_decref);
            require(bool(c),"invalid JSON case"); auto id=string_value(field(c.get(),"id"));
            auto* events=field(c.get(),"events"); require(json_is_array(events),"events array required");
            Harness h; h.setup(integer(field(c.get(),"sample_rate"),8000,192000));
            h.initialize(field(c.get(),"initial")); emit(h,id,-1,"initial");
            for (size_t i=0;i<json_array_size(events);++i) {
                auto* e=json_array_get(events,i); h.event(e);
                emit(h,id,int(i),string_value(field(e,"op")));
            }
        }
        require(std::cin.eof(),"stdin read failure"); return 0;
    }
    catch (const std::exception& error) {
        std::cerr << "Stock Noise genuine Engine probe failed: " << error.what() << '\n'; return 1;
    }
}
