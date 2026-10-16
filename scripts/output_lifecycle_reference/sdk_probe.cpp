// SPDX-License-Identifier: MIT
// Observe genuine Engine cable/bypass operations, without DSP or a host mirror.
#include "reference_module.hpp"
#include <array>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>

struct Sink final : rack::engine::Module {
    Sink() { config(0, 8, 0, 0); }
    void process(const ProcessArgs&) override { std::abort(); }
};

int main() {
    if (rack::APP_VERSION != "2.6.6" || std::fegetround() != FE_TONEAREST) return 2;
    std::cerr << "SDK=2.6.6\nengine_frames=0\nFE_TONEAREST=1\n";
#if defined(__aarch64__) || defined(__arm64__)
    std::uint64_t fpcr; asm volatile("mrs %0, fpcr" : "=r"(fpcr));
    std::cerr << "FPCR=" << fpcr << '\n';
#endif
    rack::settings::threadCount = 1;
    rack::settings::cpuMeter = false;
    rack::Context context;
    rack::contextSet(&context);
    {
        rack::engine::Engine engine;
        context.engine = &engine;
        struct DetachContext {
            rack::Context& context;
            ~DetachContext() { context.engine = nullptr; }
        } detach{context};
        engine.setSampleRate(48000.f);
        auto* module = new MidSide;
        auto* sink = new Sink;
        module->id = 1; sink->id = 2;
        engine.addModule(module); engine.addModule(sink);
        std::array<rack::engine::Cable*, 8> cables{};
        for (int i = 0; i < 8; ++i) {
            auto* cable = cables[i] = new rack::engine::Cable;
            cable->id = 100 + i;
            cable->outputModule = module; cable->outputId = i / 2;
            cable->inputModule = sink; cable->inputId = i;
        }
        auto connect = [&](int i, bool present) {
            const bool current = engine.hasCable(cables[i]);
            if (present && !current) engine.addCable(cables[i]);
            if (!present && current) engine.removeCable(cables[i]);
        };
        std::cout << std::setprecision(9);
        std::string line;
        std::size_t cases = 0;
        while (std::getline(std::cin, line)) {
            std::istringstream row(line);
            int mask; std::array<int, 4> counts{};
            std::array<std::array<float, 16>, 4> raw{};
            if (!(row >> mask) || mask < 0 || mask > 255) return 3;
            for (int p = 0; p < 4; ++p) {
                if (!(row >> counts[p]) || counts[p] < 0 || counts[p] > 16 ||
                    ((counts[p] > 0) != bool(mask & (3 << (p * 2))))) return 3;
            }
            for (auto& port : raw) for (float& value : port) {
                if (!(row >> value) || !std::isfinite(value)) return 3;
            }
            int length;
            if (!(row >> length) || length < 0 || length > 4096) return 3;

            // Explicit test-state preparation; every later transition is a real
            // Engine API call. Seed all raw slots after channel bookkeeping.
            engine.bypassModule(module, false);
            for (int i = 0; i < 8; ++i) connect(i, false);
            for (int i = 0; i < 8; ++i) if (mask & (1 << i)) connect(i, true);
            for (int p = 0; p < 4; ++p) {
                module->outputs[p].setChannels(counts[p]);
                for (int c = 0; c < 16; ++c) module->outputs[p].setVoltage(raw[p][c], c);
                if (module->outputs[p].getChannels() != counts[p]) return 4;
            }
            for (int i = 0; i < length; ++i) {
                int op; if (!(row >> op) || op < 0 || op > 18) return 3;
                if (op < 8) connect(op, true);
                else if (op < 16) connect(op - 8, false);
                else if (op == 16) engine.bypassModule(module, !module->isBypassed());
                else engine.bypassModule(module, op == 18);
            }
            std::string extra; if (row >> extra) return 3;
            if (engine.getFrame() != 0 || engine.getBlock() != 0 || !module->bypassRoutes.empty()) return 5;
            int actual_mask = 0;
            for (int i = 0; i < 8; ++i) if (engine.hasCable(cables[i])) actual_mask |= 1 << i;
            bool separator = false;
            auto emit = [&](float value) {
                if (separator) std::cout << ' ';
                std::cout << value; separator = true;
            };
            for (const auto& output : module->outputs) emit(output.channels);
            for (auto& output : module->outputs) for (int c = 0; c < 16; ++c) emit(output.getVoltage(c));
            emit(module->isBypassed()); emit(actual_mask); std::cout << '\n';
            ++cases;
        }
        std::cerr << "cases=" << cases << "\nengine_frames_final=" << engine.getFrame()
                  << "\nengine_blocks_final=" << engine.getBlock() << '\n';
        for (int i = 0; i < 8; ++i) { connect(i, false); delete cables[i]; }
        engine.removeModule(module); engine.removeModule(sink);
        delete module; delete sink;
        context.engine = nullptr;
    }
    rack::contextSet(nullptr);
    return 0;
}
