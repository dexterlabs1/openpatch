// SPDX-License-Identifier: MIT
// Original observation adapter; the unmodified pinned class prefix and license
// are archived separately in ignored evidence, never linked into OpenPatch.
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

static bool fresh(const SEQ3& m) {
    if (!m.running || m.clockPassthrough || m.phase != 0.f || m.index != 0) return false;
    if (m.clockButtonTrigger.s != rack::dsp::BooleanTrigger::UNINITIALIZED ||
        m.runButtonTrigger.s != rack::dsp::BooleanTrigger::UNINITIALIZED ||
        m.resetButtonTrigger.s != rack::dsp::BooleanTrigger::UNINITIALIZED ||
        m.clockTrigger.s != rack::dsp::SchmittTrigger::UNINITIALIZED ||
        m.runTrigger.s != rack::dsp::SchmittTrigger::UNINITIALIZED ||
        m.resetTrigger.s != rack::dsp::SchmittTrigger::UNINITIALIZED) return false;
    if (m.runPulse.remaining != 0.f || m.clockPulse.remaining != 0.f || m.resetPulse.remaining != 0.f) return false;
    for (int i = 0; i < 8; ++i) if (!m.gates[i] || m.gateTriggers[i].s != rack::dsp::BooleanTrigger::UNINITIALIZED) return false;
    for (const auto& light : m.lights) if (light.value != 0.f) return false;
    return true;
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
    const int inputIds[] = {SEQ3::TEMPO_INPUT, SEQ3::CLOCK_INPUT, SEQ3::RUN_INPUT, SEQ3::RESET_INPUT, SEQ3::STEPS_INPUT};
    const int outputIds[] = {SEQ3::CV_OUTPUTS, SEQ3::CV_OUTPUTS + 1, SEQ3::CV_OUTPUTS + 2, SEQ3::TRIG_OUTPUT,
        SEQ3::STEP_OUTPUTS, SEQ3::STEP_OUTPUTS + 1, SEQ3::STEP_OUTPUTS + 2, SEQ3::STEP_OUTPUTS + 3,
        SEQ3::STEP_OUTPUTS + 4, SEQ3::STEP_OUTPUTS + 5, SEQ3::STEP_OUTPUTS + 6, SEQ3::STEP_OUTPUTS + 7,
        SEQ3::STEPS_OUTPUT, SEQ3::CLOCK_OUTPUT, SEQ3::RUN_OUTPUT, SEQ3::RESET_OUTPUT};
    std::unique_ptr<SEQ3> module;
    int length = 8;
    unsigned processedHeld = 0;
    std::cout << std::setprecision(9);
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream input(line); std::array<float, 120> row{};
        for (float& value : row) {
            std::string token; if (!(input >> token)) return 3;
            char* end = nullptr; value = std::strtof(token.c_str(), &end);
            if (end == token.c_str() || *end != '\0' || !std::isfinite(value)) return 3;
        }
        std::string extra; if (input >> extra) return 3;
        const int event = static_cast<int>(row[0]);
        if (event < 0 || event > 5 || row[0] != event) return 3;
        if (event == 1 || !module) {
            module = std::make_unique<SEQ3>();
            rack::engine::Module::SampleRateChangeEvent changed{};
            changed.sampleRate = args.sampleRate; changed.sampleTime = args.sampleTime;
            module->onSampleRateChange(changed);
            if (!fresh(*module)) return 4;
            length = static_cast<int>(std::round(row[3]));
            processedHeld = 0;
        }
        module->params[SEQ3::TEMPO_PARAM].setValue(row[1]);
        module->params[SEQ3::TEMPO_CV_PARAM].setValue(row[2]);
        module->params[SEQ3::TRIG_PARAM].setValue(row[3]);
        module->params[SEQ3::STEPS_CV_PARAM].setValue(row[4]);
        for (int cell = 0; cell < 24; ++cell) module->params[SEQ3::CV_PARAMS + cell].setValue(row[5 + cell]);
        const unsigned held = static_cast<unsigned>(row[29]);
        if (held > 2047 || row[29] != held) return 3;
        module->params[SEQ3::CLOCK_PARAM].setValue((held & 1) ? 1.f : 0.f);
        module->params[SEQ3::RUN_PARAM].setValue((held & 2) ? 1.f : 0.f);
        module->params[SEQ3::RESET_PARAM].setValue((held & 4) ? 1.f : 0.f);
        for (int gate = 0; gate < 8; ++gate) module->params[SEQ3::GATE_PARAMS + gate].setValue((held & (8u << gate)) ? 1.f : 0.f);
        for (int i = 0; i < 5; ++i) {
            const int at = 30 + 17 * i, width = static_cast<int>(row[at]);
            if (width < 0 || width > 16 || row[at] != width) return 3;
            auto& port = module->inputs[inputIds[i]]; port.channels = width;
            for (int lane = 0; lane < 16; ++lane) {
                const float value = row[at + 1 + lane];
                if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
                port.voltages[lane] = value;
            }
        }
        // The class writes all outputs regardless of consumer count. Expose
        // every mono output; no synthetic output-mask policy is applied.
        for (int id : outputIds) module->outputs[id].channels = 1;
        if (event == 2) module->onReset();
        // Event3 keeps this actual instance in place. Production Rust rebuilds
        // its actual processor and adopts state under the same controls.
        if (event == 4) {
            json_t* state = json_object();
            for (int i = 115; i < 118; ++i) {
                const float v = row[i];
                if (v != -1.f && (v < 0.f || v != std::floor(v) || v > (i == 116 ? 255.f : 1.f))) {
                    json_decref(state); return 3;
                }
            }
            if (row[115] >= 0.f) json_object_set_new(state, "running", json_boolean(row[115] == 1.f));
            if (row[116] >= 0.f) {
                json_t* gates = json_array(); const unsigned mask = static_cast<unsigned>(row[116]);
                for (int i = 0; i < 8; ++i) json_array_append_new(gates, json_integer((mask >> i) & 1));
                json_object_set_new(state, "gates", gates);
            }
            if (row[117] >= 0.f) json_object_set_new(state, "clockPassthrough", json_boolean(row[117] == 1.f));
            module->dataFromJson(state); json_decref(state);
        }
        const int rotation = static_cast<int>(row[118]);
        if (row[118] != rotation || rotation < -8 || rotation > 8) return 3;
        if (event == 5) module->rotateStates(rotation);
        if (row[119] != 0.f && row[119] != 1.f) return 3;
        const bool bypass = row[119] == 1.f;
        // OpenPatch generic bypass host shim only; not a Rack-engine bypass claim.
        if (!bypass) {
            module->process(args);
            processedHeld = held;
            length = static_cast<int>(module->outputs[SEQ3::STEPS_OUTPUT].getVoltage()) + 1;
        }
        ++args.frame;
        bool first = true;
        auto emit = [&](float value) { if (!first) std::cout << ' '; first = false; std::cout << value; };
        for (int id : outputIds) emit(module->outputs[id].channels);
        for (int id : outputIds) emit(bypass ? 0.f : module->outputs[id].getVoltage());
        emit(module->phase); emit(module->index); emit(length); emit(module->running);
        for (bool gate : module->gates) emit(gate);
        emit(module->clockPassthrough);
        emit(module->clockButtonTrigger.s); emit(module->runButtonTrigger.s); emit(module->resetButtonTrigger.s);
        for (const auto& trigger : module->gateTriggers) emit(trigger.s);
        emit(module->clockTrigger.s); emit(module->runTrigger.s); emit(module->resetTrigger.s);
        emit(module->runPulse.remaining); emit(module->clockPulse.remaining); emit(module->resetPulse.remaining);
        for (auto& light : module->lights) emit(light.getBrightness());
        emit(processedHeld); std::cout << '\n';
    }
    return 0;
}
