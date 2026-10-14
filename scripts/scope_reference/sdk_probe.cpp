// Independent acquisition driver using installed Rack SDK ports and Schmitt triggers.
// No Fundamental implementation is included or copied.
#include <dsp/digital.hpp>
#include <engine/Port.hpp>
#include <array>
#include <cstring>
#include <iostream>
#include <limits>

struct Range { float lo = INFINITY, hi = -INFINITY; };
using Axis = std::array<std::array<Range, 256>, 16>;
void hash_float(uint64_t& hash, float value) {
    uint32_t bits; std::memcpy(&bits, &value, sizeof(bits));
    hash = (hash ^ bits) * 1099511628211ULL;
}
int main(int argc, char** argv) {
    const int width = std::stoi(argv[1]);
    const bool triggered = std::stoi(argv[2]), xy = std::stoi(argv[3]);
    rack::engine::Input inputs[3];
    rack::dsp::SchmittTrigger detectors[16];
    std::array<Axis, 2> bins;
    std::array<std::array<Range, 16>, 2> pending;
    int index = 0, samples = 0, adopt;
    while (std::cin >> adopt) {
        for (auto& input : inputs) {
            int count; std::cin >> count; input.channels = count;
            for (int lane = 0; lane < 16; ++lane) {
                float value; std::cin >> value;
                input.setVoltage(lane < count ? value : 0.f, lane);
            }
        }
        if (index == 256) {
            bool ready = xy || !triggered;
            if (!ready) {
                auto& trigger = inputs[2].isConnected() ? inputs[2] : inputs[0];
                for (int lane = 0; lane < trigger.getChannels(); ++lane)
                    ready |= detectors[lane].process(trigger.getVoltage(lane), 2.f, 2.f + .001f);
            }
            if (ready) {
                index = samples = 0;
                for (auto& detector : detectors) detector.reset();
            }
        }
        if (index < 256) {
            for (int axis = 0; axis < 2; ++axis)
                for (int lane = 0; lane < inputs[axis].getChannels(); ++lane) {
                    auto& range = pending[axis][lane];
                    range.lo = std::min(range.lo, inputs[axis].getVoltage(lane));
                    range.hi = std::max(range.hi, inputs[axis].getVoltage(lane));
                }
            if (++samples == width) {
                for (int axis = 0; axis < 2; ++axis)
                    for (int lane = 0; lane < 16; ++lane) {
                        bins[axis][lane][index] = pending[axis][lane];
                        pending[axis][lane] = Range{};
                    }
                ++index; samples = 0;
            }
        }
        uint64_t hash = 14695981039346656037ULL;
        for (const auto& axis : bins) for (const auto& lane : axis) for (auto range : lane) {
            hash_float(hash, range.lo); hash_float(hash, range.hi);
        }
        // Compare the input voltages which passthrough must preserve. Physical
        // SDK output connection bookkeeping is deliberately outside this host.
        for (int axis = 0; axis < 2; ++axis)
            for (int lane = 0; lane < 16; ++lane) hash_float(hash, inputs[axis].getVoltage(lane));
        std::cout << index << ' ' << samples << ' ' << inputs[0].getChannels()
                  << ' ' << inputs[1].getChannels() << ' ' << inputs[0].getChannels()
                  << ' ' << inputs[1].getChannels() << ' ' << inputs[2].getChannels()
                  << ' ' << hash << '\n';
    }
}
