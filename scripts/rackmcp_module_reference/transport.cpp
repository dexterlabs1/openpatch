// SPDX-License-Identifier: MIT
// Original transport/status driver; SequencerModule.cpp remains an untouched TU.
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
#include <stdexcept>
#include <string>
#include <vector>

rack::plugin::Plugin* pluginInstance = nullptr; // Unused source UI asset symbol.
extern rack::plugin::Model* modelSequencer;
static uint32_t bits(float value) { uint32_t out; std::memcpy(&out, &value, 4); return out; }
static uint64_t doubleBits(double value) { uint64_t out; std::memcpy(&out, &value, 8); return out; }
struct RestoreContext {
    rack::Context* previous = rack::contextGet();
    ~RestoreContext() { rack::contextSet(previous); }
};
struct RegisterPlugin {
    rack::plugin::Plugin* plugin;
    explicit RegisterPlugin(rack::plugin::Plugin* p) : plugin(p) { rack::plugin::plugins.push_back(plugin); }
    ~RegisterPlugin() {
        auto& registry = rack::plugin::plugins;
        registry.erase(std::remove(registry.begin(), registry.end(), plugin), registry.end());
    }
};
static json_t* readJson(const char* path) {
    json_error_t error;
    auto* value = json_load_file(path, 0, &error);
    if (!value) throw std::runtime_error(error.text);
    return value;
}
static void load(rack::engine::Engine& host, rackmcp::SequencerModule& seq, json_t* data) {
    // Genuine SDK envelope and exclusive loader, with the real plugin/model registered.
    auto* envelope = seq.toJson();
    if (!json_is_object(envelope)) throw std::runtime_error("SDK Module::toJson failed");
    json_object_set(envelope, "data", data);
    host.moduleFromJson(&seq, envelope);
    json_decref(envelope);
}
static json_t* status(rackmcp::SequencerModule& seq) {
    auto* actual = seq.statusJson(seq.id);
    auto* saved = seq.dataToJson();
    auto* result = json_object();
    // Every actual statusJson field is retained, with numeric reals encoded as
    // IEEE words so comparison cannot acquire a decimal formatting tolerance.
    for (const char* key : {"moduleId", "running", "clockSource", "activePattern", "pendingPattern",
                            "step", "loop", "patternCount", "laneCount"})
        json_object_set(result, key, json_object_get(actual, key));
    json_object_set(result, "selectedPattern", json_object_get(saved, "selectedPattern"));
    json_object_set_new(result, "bpm_bits", json_integer(bits(float(json_number_value(json_object_get(actual, "bpm"))))));
    json_object_set_new(result, "swing_bits", json_integer(bits(float(json_number_value(json_object_get(actual, "swing"))))));
    const double bar = json_number_value(json_object_get(actual, "bar"));
    json_object_set_new(result, "bar_bits", json_integer(doubleBits(bar)));
    // Published beat is exactly the source status bar * 4 (a power-of-two operation).
    json_object_set_new(result, "beat_bits", json_integer(bits(float(bar * 4.0))));
    json_decref(actual);
    json_decref(saved);
    return result;
}
static void commands(rackmcp::SequencerModule& seq, json_t* actions) {
    size_t index;
    json_t* action;
    json_array_foreach(actions, index, action) {
        const std::string name = json_string_value(json_object_get(action, "action"));
        if (name == "seek") seq.seek(json_number_value(json_object_get(action, "bar")));
        else {
            const int pattern = name == "select" ? int(json_integer_value(json_object_get(action, "pattern"))) : -1;
            seq.transport(name, pattern, json_is_true(json_object_get(action, "now")));
        }
    }
}
int main(int argc, char** argv) {
    if (argc != 3) return 2;
    if (rack::APP_VERSION != "2.6.6") throw std::runtime_error("requires genuine Rack SDK 2.6.6");
    rack::random::init();
    auto* fixture = readJson(argv[1]);
    RestoreContext restore;
    rack::Context context;
    rack::contextSet(&context);
    context.engine = new rack::engine::Engine;
    auto& host = *context.engine;
    rack::plugin::Plugin plugin;
    plugin.slug = "RackMCP";
    plugin.version = "2.0.0";
    plugin.addModel(modelSequencer);
    RegisterPlugin registered(&plugin);
    auto seq = std::make_unique<rackmcp::SequencerModule>();
    seq->id = 100;
    seq->model = modelSequencer;
    rack::engine::Module producer, sink;
    producer.config(0, 0, 2, 0); producer.id = 101;
    sink.config(0, 18, 0, 0); sink.id = 102;
    host.addModule(seq.get()); host.addModule(&producer); host.addModule(&sink);
    std::vector<std::unique_ptr<rack::engine::Cable>> outputCables;
    for (int index = 0; index < 18; ++index) {
        auto cable = std::make_unique<rack::engine::Cable>();
        cable->id = 1000 + index;
        cable->outputModule = seq.get(); cable->outputId = index;
        cable->inputModule = &sink; cable->inputId = index;
        host.addCable(cable.get()); outputCables.push_back(std::move(cable));
    }
    rack::engine::Cable clockCable, resetCable;
    clockCable.id = 2000; resetCable.id = 2001;
    clockCable.outputModule = &producer; clockCable.outputId = 0; clockCable.inputModule = seq.get(); clockCable.inputId = 0;
    resetCable.outputModule = &producer; resetCable.outputId = 1; resetCable.inputModule = seq.get(); resetCable.inputId = 1;
    host.addCable(&resetCable);
    load(host, *seq, json_object_get(fixture, "data"));
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = float(json_number_value(json_object_get(fixture, "rate")));
    args.sampleTime = 1.f / args.sampleRate;
    bool connected = false;
    float previousClock = 0.f, previousReset = 0.f;
    std::ifstream packets(argv[2]);
    std::string line;
    while (std::getline(packets, line)) {
        json_error_t error;
        auto* packet = json_loads(line.c_str(), 0, &error);
        if (!packet) throw std::runtime_error(error.text);
        const bool ext = json_is_true(json_object_get(packet, "external"));
        const bool newlyConnected = ext && !connected;
        if (ext != connected) {
            if (ext) host.addCable(&clockCable); else host.removeCable(&clockCable);
            connected = ext;
        }
        seq->params[0].setValue(float(json_number_value(json_object_get(packet, "bpm"))));
        seq->params[1].setValue(float(json_number_value(json_object_get(packet, "swing"))));
        seq->params[3].setValue(float(json_number_value(json_object_get(packet, "button"))));
        commands(*seq, json_object_get(packet, "commands"));
        auto* row = json_object();
        json_object_set_new(row, "before", status(*seq));
        // Same declared receive boundary as the existing wrapper comparison:
        // producer n arrives at n+1; a newly connected route begins at +0.
        seq->inputs[0].setVoltage(ext && !newlyConnected ? previousClock : 0.f);
        seq->inputs[1].setVoltage(previousReset);
        seq->process(args);
        json_object_set_new(row, "after", status(*seq));
        auto* outputs = json_array();
        for (auto& port : seq->outputs) {
            const int channels = port.getChannels();
            json_array_append_new(outputs, json_integer(channels));
            for (int channel = 0; channel < 16; ++channel)
                json_array_append_new(outputs, json_integer(bits(channel < channels ? port.getVoltage(channel) : 0.f)));
        }
        json_object_set_new(row, "outputs", outputs);
        auto* serialized = json_dumps(row, JSON_COMPACT | JSON_SORT_KEYS);
        if (!serialized) throw std::runtime_error("trace JSON serialization failed");
        std::cout << serialized << '\n';
        std::free(serialized);
        json_decref(row);
        previousClock = float(json_number_value(json_object_get(packet, "clock")));
        previousReset = float(json_number_value(json_object_get(packet, "reset")));
        json_decref(packet);
        ++args.frame;
    }
    if (connected) host.removeCable(&clockCable);
    host.removeCable(&resetCable);
    for (auto& cable : outputCables) host.removeCable(cable.get());
    host.removeModule(&sink); host.removeModule(&producer); host.removeModule(seq.get());
    seq.reset();
    json_decref(fixture);
}
