// SPDX-License-Identifier: GPL-3.0-or-later
// Original OpenPatch experimental host adapter. No plugin sources are compiled
// into this worker: init()/createModule()/process()/toJson() run in the DSO.
#include <rack.hpp>
#include <dlfcn.h>
#include <algorithm>
#include <cmath>
#include <iostream>
#include <memory>
#include <set>
#include <stdexcept>
#include <string>
#include <unistd.h>
#include <cstring>

namespace {
FILE* protocol_output = nullptr;
using Json = std::unique_ptr<json_t, decltype(&json_decref)>;
Json own(json_t* value) { return Json(value, &json_decref); }
void require(bool condition, const std::string& error) {
    if (!condition) throw std::runtime_error(error);
}
std::string string_field(json_t* root, const char* key) {
    auto value = json_object_get(root, key);
    require(json_is_string(value), std::string("Expected string: ") + key);
    return json_string_value(value);
}
int integer(json_t* value, int min, int max) {
    require(json_is_integer(value), "Expected integer");
    auto n = json_integer_value(value);
    require(n >= min && n <= max, "Integer outside allowed range");
    return static_cast<int>(n);
}
float number(json_t* value) {
    require(json_is_number(value), "Expected finite number");
    double n = json_number_value(value);
    require(std::isfinite(n) && std::abs(n) <= 1e6, "Number outside finite signal range");
    return static_cast<float>(n);
}
void reply(json_t* value) {
    char* encoded = json_dumps(value, JSON_COMPACT | JSON_ENCODE_ANY);
    require(encoded, "Cannot encode response");
    std::fputs(encoded, protocol_output);
    std::fputc('\n', protocol_output);
    std::fflush(protocol_output);
    free(encoded);
}
bool read_line(std::string& line) {
    line.clear();
    char c;
    while (std::cin.get(c)) {
        if (c == '\n') return true;
        require(line.size() < 1024 * 1024, "Request exceeds 1 MiB");
        line.push_back(c);
    }
    return !line.empty();
}

struct Host {
    rack::Context context;
    rack::plugin::Plugin plugin;
    std::unique_ptr<rack::engine::Module> module;
    rack::engine::Module::ProcessArgs args{};
    std::vector<bool> connected;
    bool registered = false;
    int plaits_engine = -1;
    int plaits_width = 0;
    int growing_voice_mute[16] = {};
    std::vector<unsigned char> binary_output;
    // Keep the DSO mapped through all module/model/context destruction. The OS
    // reclaims it when this one-plugin process exits; hot-unloading is excluded.
    void* library = nullptr;

    Host(const std::string& directory, const std::string& model, int rate,
         const std::string& system, const std::string& user) {
        rack::asset::systemDir = system;
        rack::asset::userDir = user;
        rack::contextSet(&context);
        rack::random::init();
        context.engine = new rack::engine::Engine;
        context.engine->setSampleRate(static_cast<float>(rate));
        args.sampleRate = static_cast<float>(rate);
        args.sampleTime = 1.f / args.sampleRate;
        plugin.path = directory;
        json_error_t error{};
        auto manifest = own(json_load_file((directory + "/plugin.json").c_str(),
                                          JSON_REJECT_DUPLICATES, &error));
        require(bool(manifest), "Cannot read plugin manifest: " + std::string(error.text));
        plugin.fromJson(manifest.get());
        require(plugin.version.compare(0, 2, "2.") == 0, "Only Rack 2 plugins are supported");
#if defined(__APPLE__)
        const char* extension = "/plugin.dylib";
#else
        const char* extension = "/plugin.so";
#endif
        library = dlopen((directory + extension).c_str(), RTLD_NOW | RTLD_LOCAL);
        if (!library) throw std::runtime_error(dlerror());
        plugin.handle = library;
        auto init = reinterpret_cast<void (*)(rack::plugin::Plugin*)>(dlsym(library, "init"));
        require(init, "Plugin has no init entry point");
        init(&plugin);
        plugin.modulesFromJson(json_object_get(manifest.get(), "modules"));
        // Module::fromJson resolves its model through Rack's plugin registry.
        // Register exactly this plugin, as Rack's own loader does after init.
        rack::plugin::plugins.push_back(&plugin);
        auto factory = plugin.getModel(model);
        require(factory, "Model not found: " + model);
        module.reset(factory->createModule());
        require(bool(module), "Model did not create a DSP module");
        // This process owns a single module, with a stable local engine ID.
        module->id = 0;
        context.engine->addModule(module.get());
        registered = true;
        connected.resize(module->outputs.size(), false);
    }

    ~Host() {
        if (registered) context.engine->removeModule(module.get());
        module.reset();
        auto& plugins = rack::plugin::plugins;
        plugins.erase(std::remove(plugins.begin(), plugins.end(), &plugin), plugins.end());
        // Context outlives plugin model teardown (member destruction order).
        // Never dlclose here: process-global plugin statics can retain vtables.
    }

    void validate_params(json_t* params) {
        require(json_is_array(params), "params must be an array");
        std::set<int> seen;
        for (size_t i = 0; i < json_array_size(params); ++i) {
            auto item = json_array_get(params, i);
            int id = integer(json_object_get(item, "id"), 0, int(module->params.size()) - 1);
            require(seen.insert(id).second, "Duplicate parameter ID");
            float value = number(json_object_get(item, "value"));
            auto quantity = module->paramQuantities.at(id);
            if (quantity) require(value >= quantity->minValue && value <= quantity->maxValue,
                                  "Parameter outside module bounds");
        }
    }

    void validate_inputs(json_t* inputs) {
        require(json_is_array(inputs), "inputs must be an array");
        std::set<int> seen;
        for (size_t i = 0; i < json_array_size(inputs); ++i) {
            auto item = json_array_get(inputs, i);
            int id = integer(json_object_get(item, "port"), 0, int(module->inputs.size()) - 1);
            require(seen.insert(id).second, "Duplicate input port");
            auto voltages = json_object_get(item, "voltages");
            require(json_is_array(voltages) && json_array_size(voltages) <= 16,
                    "Each input must have zero to sixteen voltage channels");
            for (size_t j = 0; j < json_array_size(voltages); ++j) number(json_array_get(voltages, j));
        }
    }

    void apply_inputs(json_t* inputs) {
        for (size_t i = 0; i < json_array_size(inputs); ++i) {
            auto item = json_array_get(inputs, i);
            int id = int(json_integer_value(json_object_get(item, "port")));
            auto voltages = json_object_get(item, "voltages");
            auto& port = module->inputs[id];
            bool before = port.isConnected();
            std::fill(std::begin(port.voltages), std::end(port.voltages), 0.f);
            port.channels = static_cast<uint8_t>(json_array_size(voltages));
            for (int channel = 0; channel < port.channels; ++channel)
                port.voltages[channel] = float(json_number_value(json_array_get(voltages, channel)));
            if (before != port.isConnected()) module->onPortChange({port.isConnected(), rack::engine::Port::INPUT, id});
        }
    }

    void prepare_voice_width() {
        if (plugin.slug != "AudibleInstruments" || module->model->slug != "Plaits") return;
        int width=std::max(1,module->inputs[7].getChannels());
        // Original Plaits buffers whole 32-lane frames but initializes only the
        // active lanes. Growing polyphony can expose uninitialized lanes from
        // buffered old-width frames. Run the untouched DSP, but mute newly added
        // lanes until its <=256-frame output buffer has retired.
        if (plaits_width>0 && width>plaits_width)
            for (int c=plaits_width;c<width;++c) growing_voice_mute[c]=256;
        plaits_width=width;
    }
    float output_voltage(int port,int channel) {
        if (growing_voice_mute[channel]>0) return 0.f;
        return module->outputs[port].getVoltage(channel);
    }
    void finish_voice_frame() {
        for (int& remaining:growing_voice_mute) if (remaining>0) --remaining;
    }

    Json describe() {
        auto result = own(json_pack("{s:s,s:i,s:s,s:s,s:s,s:s,s:i}",
            "protocol", "openpatch-rack-worker", "version", 1,
            "plugin", plugin.slug.c_str(), "plugin_version", plugin.version.c_str(),
            "license", plugin.license.c_str(), "model", module->model->slug.c_str(),
            "sample_rate", int(args.sampleRate)));
        auto params = json_array();
        for (size_t i = 0; i < module->params.size(); ++i) {
            auto q = module->paramQuantities.at(i);
            auto item = json_pack("{s:i,s:s,s:f}", "id", int(i), "name", q ? q->name.c_str() : "",
                                  "value", double(module->params[i].getValue()));
            if (q) {
                // Unbounded quantities are represented by null bounds.
                json_object_set_new(item, "min", std::isfinite(q->minValue) ? json_real(q->minValue) : json_null());
                json_object_set_new(item, "max", std::isfinite(q->maxValue) ? json_real(q->maxValue) : json_null());
                json_object_set_new(item, "default", json_real(q->defaultValue));
                json_object_set_new(item, "unit", json_string(q->unit.c_str()));
                json_object_set_new(item, "snap", json_boolean(q->snapEnabled));
            }
            json_array_append_new(params, item);
        }
        json_object_set_new(result.get(), "params", params);
        json_object_set_new(result.get(), "loaded_plugins", json_integer(rack::plugin::plugins.size()));
        json_object_set_new(result.get(), "instantiated_modules", json_integer(context.engine->getNumModules()));
        for (bool input : {true, false}) {
            auto ports = json_array();
            size_t count = input ? module->inputs.size() : module->outputs.size();
            for (size_t i = 0; i < count; ++i) {
                auto info = input ? module->inputInfos.at(i) : module->outputInfos.at(i);
                json_array_append_new(ports, json_pack("{s:i,s:s}", "id", int(i),
                    "name", info ? info->name.c_str() : ""));
            }
            json_object_set_new(result.get(), input ? "inputs" : "outputs", ports);
        }
        return result;
    }

    Json save() {
        auto state = own(json_pack("{s:s,s:i,s:s,s:s,s:s,s:i}",
            "format", "openpatch-rack-module", "version", 1,
            "plugin", plugin.slug.c_str(), "plugin_version", plugin.version.c_str(),
            "model", module->model->slug.c_str(), "sample_rate", int(args.sampleRate)));
        context.engine->prepareSaveModule(module.get());
        json_object_set_new(state.get(), "module", context.engine->moduleToJson(module.get()));
        auto inputs = json_array();
        for (size_t i = 0; i < module->inputs.size(); ++i) {
            auto values = json_array();
            for (int c = 0; c < module->inputs[i].getChannels(); ++c)
                json_array_append_new(values, json_real(module->inputs[i].getVoltage(c)));
            auto item = json_pack("{s:i}", "port", int(i));
            json_object_set_new(item, "voltages", values);
            json_array_append_new(inputs, item);
        }
        json_object_set_new(state.get(), "inputs", inputs);
        return state;
    }

    void restore(json_t* state) {
        require(string_field(state, "format") == "openpatch-rack-module", "Unknown state format");
        integer(json_object_get(state, "version"), 1, 1);
        require(string_field(state, "plugin") == plugin.slug &&
                string_field(state, "plugin_version") == plugin.version &&
                string_field(state, "model") == module->model->slug, "State plugin/model/version mismatch");
        require(integer(json_object_get(state, "sample_rate"), 8000, 192000) == int(args.sampleRate),
                "State sample rate mismatch");
        auto serialized = json_object_get(state, "module");
        require(string_field(serialized, "plugin") == plugin.slug &&
                string_field(serialized, "model") == module->model->slug &&
                string_field(serialized, "version") == plugin.version, "Nested module identity mismatch");
        validate_params(json_object_get(serialized, "params"));
        auto inputs = json_object_get(state, "inputs");
        validate_inputs(inputs);
        require(json_array_size(inputs) == module->inputs.size(), "State must specify every input");
        auto copy = own(json_deep_copy(serialized));
        json_object_set_new(copy.get(), "id", json_integer(module->id));
        context.engine->moduleFromJson(module.get(), copy.get());
        apply_inputs(inputs);
    }

    Json process(json_t* request) {
        int frames = integer(json_object_get(request, "frames"), 1, 4096);
        auto stream = json_object_get(request, "input_frames");
        auto controls = json_object_get(request, "controls");
        const int mapping[7] = {7, 5, 4, 1, 3, 2, 6};
        if (stream || controls) {
            require(plugin.slug == "AudibleInstruments" && module->model->slug == "Plaits",
                    "Live input frames currently support original Plaits only");
            require(frames <= 256 && json_is_array(stream) && int(json_array_size(stream)) == frames,
                    "Live stream requires one input frame per sample, at most 256 samples");
            require(json_is_array(controls) && json_array_size(controls) == 6, "Expected six Plaits controls");
            for (int i = 0; i < 6; ++i) {
                float v = number(json_array_get(controls, i));
                require(v >= (i == 1 ? -4 : 0) && v <= (i == 0 ? 15 : i == 1 ? 4 : 1), "Invalid Plaits control");
                if (i == 0) require(std::floor(v) == v, "Engine must be an integer");
            }
            for (int frame = 0; frame < frames; ++frame) {
                auto inputs = json_array_get(stream, frame);
                require(json_is_array(inputs) && json_array_size(inputs) == 7, "Expected seven Plaits input ports");
                for (int port = 0; port < 7; ++port) {
                    auto values = json_array_get(inputs, port);
                    require(json_is_array(values) && json_array_size(values) <= 16, "Input width exceeds sixteen");
                    for (size_t c = 0; c < json_array_size(values); ++c) number(json_array_get(values, c));
                }
            }
        }
        auto ports = json_object_get(request, "outputs");
        require(json_is_array(ports) && json_array_size(ports) > 0, "Select at least one output");
        std::vector<int> selected;
        for (size_t i = 0; i < json_array_size(ports); ++i) {
            int id = integer(json_array_get(ports, i), 0, int(module->outputs.size()) - 1);
            require(std::find(selected.begin(), selected.end(), id) == selected.end(), "Duplicate output");
            selected.push_back(id);
        }
        if (stream) {
            int engine = int(json_number_value(json_array_get(controls, 0)));
            if (engine != plaits_engine) {
                auto data = own(json_pack("{s:i,s:b}", "model", engine, "lowCpu", false));
                module->dataFromJson(data.get());
                plaits_engine = engine;
            }
            const int params[5] = {2, 3, 4, 5, 10};
            for (int i = 0; i < 5; ++i) context.engine->setParamValue(module.get(), params[i], number(json_array_get(controls, i+1)));
            for (int id : {6, 7, 8}) context.engine->setParamValue(module.get(), id, 1.f);
        }
        for (size_t i = 0; i < connected.size(); ++i) {
            bool after = std::find(selected.begin(), selected.end(), int(i)) != selected.end();
            if (connected[i] != after) {
                connected[i] = after;
                module->outputs[i].channels = after ? 1 : 0;
                module->onPortChange({after, rack::engine::Port::OUTPUT, int(i)});
            }
        }
        auto result = own(json_pack("{s:i,s:I}", "frames", frames, "start_frame", args.frame));
        auto outputs = json_array();
        for (int id : selected) {
            auto item = json_pack("{s:i,s:[]}", "port", id, "samples");
            json_array_append_new(outputs, item);
        }
        json_object_set_new(result.get(), "outputs", outputs);
        for (int frame = 0; frame < frames; ++frame) {
            if (stream) {
                auto inputs = json_array_get(stream, frame);
                for (int i = 0; i < 7; ++i) {
                    auto values = json_array_get(inputs, i);
                    auto& port = module->inputs[mapping[i]];
                    bool before = port.isConnected();
                    port.channels = uint8_t(json_array_size(values));
                    std::fill(std::begin(port.voltages), std::end(port.voltages), 0.f);
                    for (int c = 0; c < port.channels; ++c) port.voltages[c] = float(json_number_value(json_array_get(values, c)));
                    if (before != port.isConnected()) module->onPortChange({port.isConnected(), rack::engine::Port::INPUT, mapping[i]});
                }
            }
            // One genuine module, constant input CV between requests. The worker
            // does not implement Rack graph/cable timing or a realtime callback.
            prepare_voice_width();
            module->process(args);
            ++args.frame;
            for (size_t i = 0; i < selected.size(); ++i) {
                auto& output = module->outputs[selected[i]];
                require(output.getChannels() >= 0 && output.getChannels() <= 16, "Invalid plugin output width");
                auto values = json_array();
                for (int c = 0; c < output.getChannels(); ++c) {
                    float value = output_voltage(selected[i],c);
                    require(std::isfinite(value), "Plugin emitted nonfinite audio");
                    json_array_append_new(values, json_real(value));
                }
                json_array_append_new(json_object_get(json_array_get(outputs, i), "samples"), values);
            }
            finish_voice_frame();
        }
        return result;
    }

    Json stream(json_t* request) {
        require(plugin.slug == "AudibleInstruments" && module->model->slug == "Plaits", "Binary bridge supports original Plaits only");
        int frames=integer(json_object_get(request,"frames"),1,256);
        int length=integer(json_object_get(request,"input_bytes"),frames*7,frames*7*65);
        auto controls=json_object_get(request,"controls");
        require(json_is_array(controls) && json_array_size(controls)==6,"Expected six Plaits controls");
        float settings[6];
        for (int i=0;i<6;++i) {
            settings[i]=number(json_array_get(controls,i));
            require(settings[i]>=(i==1?-4:0) && settings[i]<=(i==0?15:i==1?4:1),"Invalid Plaits control");
        }
        require(std::floor(settings[0])==settings[0],"Engine must be an integer");
        std::vector<unsigned char> bytes(length);
        std::cin.read(reinterpret_cast<char*>(bytes.data()),length);
        require(std::cin.gcount()==length,"Incomplete binary input block");
        size_t cursor=0;
        // Validate the complete variable-width block before mutating the module.
        for (int frame=0;frame<frames;++frame) for (int port=0;port<7;++port) {
            require(cursor<bytes.size(),"Missing binary input width");
            int width=bytes[cursor++];require(width<=16 && cursor+width*4<=bytes.size(),"Invalid binary input width");
            for (int c=0;c<width;++c) { float v;std::memcpy(&v,bytes.data()+cursor,4);cursor+=4;require(std::isfinite(v)&&std::abs(v)<=1e6,"Invalid binary input voltage"); }
        }
        require(cursor==bytes.size(),"Unexpected trailing binary input bytes");
        if (plaits_engine!=int(settings[0])) {
            auto data=own(json_pack("{s:i,s:b}","model",int(settings[0]),"lowCpu",false));
            module->dataFromJson(data.get());plaits_engine=int(settings[0]);
        }
        const int params[5]={2,3,4,5,10};
        for (int i=0;i<5;++i) context.engine->setParamValue(module.get(),params[i],settings[i+1]);
        for (int id:{6,7,8}) context.engine->setParamValue(module.get(),id,1.f);
        for (int port=0;port<2;++port) if (!connected[port]) {
            connected[port]=true;module->outputs[port].channels=1;
            module->onPortChange({true,rack::engine::Port::OUTPUT,port});
        }
        binary_output.clear();binary_output.reserve(frames*130);cursor=0;
        const int mapping[7]={7,5,4,1,3,2,6};
        for (int frame=0;frame<frames;++frame) {
            for (int i=0;i<7;++i) {
                auto& port=module->inputs[mapping[i]];bool before=port.isConnected();
                int width=bytes[cursor++];port.channels=uint8_t(width);
                std::fill(std::begin(port.voltages),std::end(port.voltages),0.f);
                for (int c=0;c<width;++c) {std::memcpy(&port.voltages[c],bytes.data()+cursor,4);cursor+=4;}
                if (before!=port.isConnected()) module->onPortChange({port.isConnected(),rack::engine::Port::INPUT,mapping[i]});
            }
            prepare_voice_width();module->process(args);++args.frame;
            for (int port=0;port<2;++port) {
                auto& output=module->outputs[port];int width=output.getChannels();require(width>=0&&width<=16,"Invalid binary output width");
                binary_output.push_back(uint8_t(width));
                for (int c=0;c<width;++c) {
                    float v=output_voltage(port,c);require(std::isfinite(v),"Plugin emitted nonfinite voltage");
                    unsigned char raw[4];std::memcpy(raw,&v,4);binary_output.insert(binary_output.end(),raw,raw+4);
                }
            }
            finish_voice_frame();
        }
        return own(json_pack("{s:i,s:i,s:s}","frames",frames,"bytes",int(binary_output.size()),"encoding","f32le-voices"));
    }

    Json command(json_t* request) {
        std::string op = string_field(request, "op");
        if (op == "describe") return describe();
        if (op == "save") return save();
        if (op == "process") return process(request);
        if (op == "restore") restore(json_object_get(request, "state"));
        else if (op == "set_inputs") {
            auto inputs = json_object_get(request, "inputs");
            validate_inputs(inputs);
            apply_inputs(inputs);
        } else if (op == "set_params") {
            auto params = json_object_get(request, "params");
            validate_params(params);
            for (size_t i = 0; i < json_array_size(params); ++i) {
                auto item = json_array_get(params, i);
                int id = int(json_integer_value(json_object_get(item, "id")));
                context.engine->setParamValue(module.get(), id, number(json_object_get(item, "value")));
            }
        } else throw std::runtime_error("Unknown operation: " + op);
        return own(json_object());
    }
};
}

int main(int argc, char** argv) {
    try {
        // Third-party printf/std::cout output must not corrupt JSON transport.
        protocol_output = fdopen(dup(STDOUT_FILENO), "w");
        require(protocol_output && dup2(STDERR_FILENO, STDOUT_FILENO) >= 0,
                "Cannot initialize protocol output");
        require(argc == 6, "Usage: worker PLUGIN_DIR MODEL SAMPLE_RATE SYSTEM_DIR USER_DIR");
        size_t consumed = 0;
        int rate = std::stoi(argv[3], &consumed);
        require(consumed == std::string(argv[3]).size() && rate >= 8000 && rate <= 192000,
                "Sample rate must be 8000..192000 Hz");
        Host host(argv[1], argv[2], rate, argv[4], argv[5]);
        auto ready = host.describe();
        auto envelope = own(json_object());
        json_object_set(envelope.get(), "ready", ready.get());
        reply(envelope.get());
        std::string line;
        while (read_line(line)) {
            try {
                json_error_t error{};
                auto request = own(json_loads(line.c_str(), JSON_REJECT_DUPLICATES, &error));
                require(bool(request) && json_is_object(request.get()), "Invalid JSON request");
                if (string_field(request.get(),"op")=="stream") {
                    auto result=host.stream(request.get());
                    auto response=own(json_object());json_object_set(response.get(),"result",result.get());reply(response.get());
                    std::fwrite(host.binary_output.data(),1,host.binary_output.size(),protocol_output);std::fflush(protocol_output);
                    continue;
                }
                auto result = host.command(request.get());
                auto response = own(json_object());
                json_object_set(response.get(), "result", result.get());
                reply(response.get());
            } catch (const std::exception& error) {
                auto response = own(json_pack("{s:s}", "error", error.what()));
                reply(response.get());
            }
        }
        return 0;
    } catch (const std::exception& error) {
        std::cerr << "Rack worker: " << error.what() << '\n';
        return 1;
    }
}
