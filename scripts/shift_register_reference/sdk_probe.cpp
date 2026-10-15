// SPDX-License-Identifier: MIT
// Original headless test driver. reference_module.hpp exists only in a TEMP
// directory and contains the separately downloaded, licensed upstream class.
#include "reference_module.hpp"
#include <iomanip>
#include <iostream>

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    SHASR module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    args.sampleTime = 1.f / args.sampleRate;
    module.params[SHASR::RND_PARAM].setValue(0.f);
    // Arm the SDK BooleanTriggers through an actual low processing frame.
    // No CV inputs are connected, so all Schmitt histories stay uninitialized.
    module.params[SHASR::PUSH_PARAM].setValue(0.f);
    module.params[SHASR::CLEAR_PARAM].setValue(0.f);
    module.process(args);
    for (auto& output : module.outputs) output.channels = 1;
    std::cout << std::setprecision(9);
    int push, clear;
    while (std::cin >> push >> clear) {
        module.params[SHASR::PUSH_PARAM].setValue(push ? 1.f : 0.f);
        module.params[SHASR::CLEAR_PARAM].setValue(clear ? 1.f : 0.f);
        for (auto& input : module.inputs) {
            int count;
            if (!(std::cin >> count) || count < 0 || count > 16) return 3;
            // Rack normally sets connected-port channel bookkeeping in Engine.
            input.channels = count;
            for (float& voltage : input.voltages) if (!(std::cin >> voltage)) return 3;
        }
        module.process(args);
        for (int i = 0; i < 8; ++i) {
            if (i) std::cout << ' ';
            std::cout << module.outputs[SHASR::SH_OUTPUTS + i].getVoltage(0);
        }
        std::cout << '\n';
        args.frame++;
    }
}
