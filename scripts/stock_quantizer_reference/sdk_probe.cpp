// SPDX-License-Identifier: MIT
// Observation adapter for the genuine pinned class. No quantizer implementation.
#include "reference_module.hpp"
#include <array>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <memory>
#include <sstream>
#include <string>

static void load(Quantizer& module, int length, unsigned mask) {
    json_t* object = json_object();
    if (length >= 0) {
        json_t* notes = json_array();
        for (int i = 0; i < length; ++i)
            json_array_append_new(notes, json_boolean(i < 12 && (mask & (1u << i))));
        json_object_set_new(object, "enabledNotes", notes);
    }
    module.dataFromJson(object);
    json_decref(object);
}
static bool serialized_bank_matches(Quantizer& module) {
    json_t* object = module.dataToJson();
    json_t* notes = json_object_get(object, "enabledNotes");
    bool matches = json_is_array(notes) && json_array_size(notes) == 12;
    for (int i = 0; i < 12 && matches; ++i) {
        json_t* note = json_array_get(notes, i);
        matches = json_is_boolean(note) && bool(json_boolean_value(note)) == module.enabledNotes[i];
    }
    json_decref(object);
    return matches;
}
int main(int argc, char** argv) {
    if (argc != 2 || rack::APP_VERSION != "2.6.6" || std::fegetround() != FE_TONEAREST) return 2;
    std::cerr << "FE_TONEAREST=1\n";
#if defined(__aarch64__) || defined(__arm64__)
    std::uint64_t fpcr; asm volatile("mrs %0, fpcr" : "=r"(fpcr));
    std::cerr << "FPCR=" << fpcr << '\n';
#endif
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]); args.sampleTime = 1.f / args.sampleRate;
    if (!std::isfinite(args.sampleRate) || args.sampleRate <= 0) return 3;
    std::unique_ptr<Quantizer> module;
    bool playing_valid = false;
    std::cout << std::setprecision(9);
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream input(line); std::array<float, 24> row{};
        for (float& value : row) {
            std::string token; if (!(input >> token)) return 3;
            char* end = nullptr; value = std::strtof(token.c_str(), &end);
            if (end == token.c_str() || *end != '\0' || !std::isfinite(value)) return 3;
        }
        std::string extra; if (input >> extra) return 3;
        auto integer = [&](int at, int low, int high) {
            return row[at] >= low && row[at] <= high && row[at] == std::floor(row[at]);
        };
        if (!integer(0,0,6) || row[1] < -1.f || row[1] > 1.f || !integer(2,0,16) ||
            !integer(19,-1,14) || !integer(20,0,16383) || !integer(21,-12,12) ||
            !integer(22,0,1) || !integer(23,0,1)) return 3;
        const int event = int(row[0]), width = int(row[2]);
        const bool connected = row[22] != 0.f, bypassed = row[23] != 0.f;
        if (event == 1 || !module) {
            module = std::make_unique<Quantizer>(); playing_valid = false;
            rack::engine::Module::SampleRateChangeEvent changed{};
            changed.sampleRate = args.sampleRate; changed.sampleTime = args.sampleTime;
            module->onSampleRateChange(changed);
            for (bool enabled : module->enabledNotes) if (!enabled) return 4;
            // playingNotes is genuinely indeterminate until process(), never read here.
        }
        module->params[Quantizer::OFFSET_PARAM].setValue(row[1]);
        if (event == 2) module->onReset();
        // Event 3 keeps the actual SDK instance; Rust exercises real rebuild/adopt.
        if (event == 4) load(*module, int(row[19]), unsigned(row[20]));
        if (event == 5) module->rotateNotes(int(row[21]));
        if (event == 6) load(*module, 12, unsigned(row[20]));
        if (event != 0 && !serialized_bank_matches(*module)) return 4;
        auto& pitch = module->inputs[Quantizer::PITCH_INPUT]; pitch.channels = width;
        for (int lane = 0; lane < 16; ++lane) {
            const float value = row[3+lane];
            if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
            pitch.voltages[lane] = value;
        }
        auto& output = module->outputs[Quantizer::PITCH_OUTPUT];
        if (!connected) output.channels = 0;
        else if (!output.channels) output.channels = 1;
        if (bypassed) {
            module->processBypass(args);
        } else {
            for (int lane = 0; lane < std::max(width,1); ++lane) {
                const float adjusted = pitch.getVoltage(lane) + row[1];
                const float scaled = adjusted * 24.f;
                const double q = std::floor(scaled);
                if (!std::isfinite(adjusted) || !std::isfinite(scaled) ||
                    q < -2147483640.0 || q > 2147483647.0) return 5;
            }
            module->process(args); playing_valid = true;
        }
        unsigned enabled = 0, playing = 0;
        for (int i = 0; i < 12; ++i) {
            if (module->enabledNotes[i]) enabled |= 1u << i;
            if (playing_valid && module->playingNotes[i]) playing |= 1u << i;
        }
        std::cout << output.getChannels();
        for (int lane = 0; lane < 16; ++lane) std::cout << ' ' << output.getVoltage(lane);
        std::cout << ' ' << enabled;
        for (int range : module->ranges) std::cout << ' ' << range;
        std::cout << ' ' << unsigned(playing_valid) << ' ' << playing << ' '
                  << module->params[Quantizer::OFFSET_PARAM].getValue() << '\n';
    }
    return 0;
}
