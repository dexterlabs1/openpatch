// SPDX-License-Identifier: MIT
// Original driver; upstream DSP exists only in temporary reference_module.hpp.
#include "reference_module.hpp"
#include <iomanip>
#include <iostream>

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    ADSR module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    args.sampleTime = 1.f / args.sampleRate;
    // Rack's engine normally marks output connectivity. No warm-up process call.
    module.outputs[ADSR::ENVELOPE_OUTPUT].channels = 1;
    std::cout << std::setprecision(9);
    int push;
    while (std::cin >> push) {
        module.params[ADSR::PUSH_PARAM].setValue(push ? 1.f : 0.f);
        for (int i = 0; i < 8; ++i) {
            float value;
            if (!(std::cin >> value)) return 3;
            module.params[i].setValue(value);
        }
        for (auto& input : module.inputs) {
            int count;
            if (!(std::cin >> count) || count < 0 || count > 16) return 3;
            input.channels = count;
            for (int lane = 0; lane < 16; ++lane) {
                float voltage;
                if (!(std::cin >> voltage)) return 3;
                // Inactive Rack port lanes are zeroed by Engine. Poison supplied
                // by the fixture must not become a connected voltage.
                input.voltages[lane] = lane < count ? voltage : 0.f;
            }
        }
        module.process(args);
        auto& output = module.outputs[ADSR::ENVELOPE_OUTPUT];
        std::cout << output.getChannels();
        for (int lane = 0; lane < 16; ++lane)
            std::cout << ' ' << (lane < output.getChannels() ? output.getVoltage(lane) : 0.f);
        for (int i = 0; i < 5; ++i) std::cout << ' ' << module.lights[i].getBrightness();
        std::cout << '\n';
        args.frame++;
    }
}
