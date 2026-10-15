// SPDX-License-Identifier: MIT
// Original adapter. The audited upstream class exists only in a temporary header.
#include "reference_module.hpp"
#include <array>
#include <iostream>
#include <sstream>
#include <string>

using Config = std::array<int, 19>;

static void applyConfig(rack::core::MIDI_Gate& module, const Config& next, const Config& prior) {
    if (!std::equal(next.begin() + 3, next.end(), prior.begin() + 3)) {
        json_t* root = json_object();
        json_t* notes = json_array();
        for (int i = 0; i < 16; ++i) json_array_append_new(notes, json_integer(next[3 + i]));
        json_object_set_new(root, "notes", notes);
        module.dataFromJson(root);
        json_decref(root);
    }
    module.velocityMode = next[0];
    module.mpeMode = next[1];
    // Cancelling Learn on configuration edits is the disclosed host policy;
    // source panic below intentionally does not use that cancellation policy.
    module.learningId = -1;
}

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    rack::core::MIDI_Gate module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    args.sampleTime = 1.f / args.sampleRate;
    Config previous; previous.fill(-999);
    Config savedConfig{};
    json_t* saved = nullptr;
    // Each output is observed by this probe. No process call or velocity read
    // occurs before the first fixture frame. Constructor velocity bytes remain
    // untouched and are read only by actual DSP after a valid positive NoteOn.
    for (auto& output : module.outputs) output.channels = 1;
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream input(line);
        int velocity, mpe, channel, operation, learn, events;
        if (!(input >> velocity >> mpe >> channel >> operation >> learn >> events)) return 3;
        Config config{velocity, mpe, channel};
        for (int i = 0; i < 16; ++i) input >> config[3 + i];
        if (config != previous) {
            applyConfig(module, config, previous);
            previous = config;
        }
        if (operation == 1) module.panic();
        if (operation == 2) {
            if (saved) json_decref(saved);
            saved = module.dataToJson();
            // Endpoint selection is global host policy and outside this oracle.
            json_object_del(saved, "midi");
            savedConfig = config;
            for (int i = 0; i < 16; ++i) savedConfig[3 + i] = module.learnedNotes[i];
        }
        if (operation == 3) {
            if (!saved) return 4;
            module.dataFromJson(saved);
            module.learningId = -1;
            previous = config = savedConfig;
        }
        if (learn > 0) module.learningId = learn - 1;
        int priorLearning = module.learningId;
        for (int i = 0; i < events; ++i) {
            int status, wire, note, value;
            input >> status >> wire >> note >> value;
            rack::midi::Message message;
            message.bytes = {uint8_t((status << 4) | wire), uint8_t(note), uint8_t(value)};
            message.frame = args.frame;
            // Exact pinned InputDevice::onMessage channel rule, before the
            // genuine SDK queue. No device, driver or Rack context is opened.
            if (status != 15 && config[2] != 0 && wire != config[2] - 1) continue;
            module.midiInput.onMessage(message);
        }
        module.process(args);
        int32_t metadata[34]; int index = 0;
        for (int i = 0; i < 16; ++i) metadata[index++] = module.learnedNotes[i];
        metadata[index++] = module.learningId;
        metadata[index++] = priorLearning >= 0 && module.learningId < 0 ? priorLearning : -1;
        for (auto& output : module.outputs) metadata[index++] = output.getChannels();
        std::cout.write(reinterpret_cast<char*>(metadata), sizeof(metadata));
        float voltages[256]; index = 0;
        for (auto& output : module.outputs)
            for (int lane = 0; lane < 16; ++lane)
                voltages[index++] = lane < output.getChannels() ? output.getVoltage(lane) : 0.f;
        std::cout.write(reinterpret_cast<char*>(voltages), sizeof(voltages));
        args.frame++;
    }
    if (saved) json_decref(saved);
}
