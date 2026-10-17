// Original OpenPatch test adapter, MIT. DSP files are unmodified pinned sources.
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
#include "stages/segment_generator.h"
#include "stmlib/utils/random.h"

static void require(bool condition, const char* message) {
  if (!condition) throw std::runtime_error(message);
}
static uint32_t bits(float f) {
  uint32_t result;
  std::memcpy(&result, &f, sizeof(result));
  return result;
}
int main() {
  try {
    // Firmware globals have static zero storage. Init alone omits some fields.
    static stages::SegmentGenerator generators[6];
    static stmlib::HysteresisQuantizer2 notes[42];
    unsigned quantized, seed;
    require(bool(std::cin >> quantized >> seed) && quantized <= 1, "invalid header");
    for (auto& note : notes) note.Init(13, 0.03f, false);
    for (unsigned i = 0; i < 6; ++i) generators[i].Init(quantized ? notes + i : nullptr);
    stmlib::Random::Seed(seed);
    std::array<stmlib::GateFlags, 6> previous{};
    size_t frame = 0;
    std::string operation;
    while (std::cin >> operation) {
      unsigned slot;
      require(bool(std::cin >> slot) && slot < 6, "invalid slot");
      if (operation == "C" || operation == "I") {
        unsigned triggered, count;
        require(bool(std::cin >> triggered >> count) && triggered < 2 && count >= 1 && count <= 36, "invalid configuration");
        std::array<stages::segment::Configuration, 36> configuration;
        for (unsigned i = 0; i < count; ++i) {
          unsigned kind, loop;
          require(bool(std::cin >> kind >> loop) && kind < 3 && loop < 2, "invalid segment");
          configuration[i] = {static_cast<stages::segment::Type>(kind), bool(loop)};
        }
        if (operation == "I") {
          require(count == 1, "single needs one stage");
          generators[slot].ConfigureSingleSegment(triggered, configuration[0]);
        } else {
          generators[slot].Configure(triggered, configuration.data(), count);
        }
      } else if (operation == "P") {
        unsigned index; float primary, secondary;
        require(bool(std::cin >> index >> primary >> secondary) && index < 36, "invalid parameter index");
        // Exclude source table overreads BEFORE entering original DSP.
        require(std::isfinite(primary) && primary >= -1 && primary < 2, "guard: source primary endpoint/domain");
        require(std::isfinite(secondary) && secondary >= 0 && secondary < 1, "guard: source secondary endpoint/domain");
        generators[slot].set_segment_parameters(index, primary, secondary);
      } else if (operation == "S") {
        unsigned monitored;
        require(bool(std::cin >> monitored) && monitored < 36, "invalid slave index");
        generators[slot].ConfigureSlave(monitored);
      } else if (operation == "B") {
        size_t size, blocks, period, high, onset; unsigned count;
        require(bool(std::cin >> size >> blocks >> period >> high >> onset >> count), "invalid block");
        require(size >= 1 && size <= 64 && blocks <= 50000 && high <= period && count < 6, "invalid block dimensions");
        std::array<unsigned, 5> slaves{};
        for (unsigned i = 0; i < count; ++i)
          require(bool(std::cin >> slaves[i]) && slaves[i] < 6 && slaves[i] != slot, "invalid slave slot");
        std::vector<stmlib::GateFlags> gates(size);
        std::vector<stages::SegmentGenerator::Output> output(size);
        std::array<std::vector<stages::SegmentGenerator::Output>, 5> monitored;
        for (size_t block = 0; block < blocks; ++block) {
          for (size_t i = 0; i < size; ++i) {
            const size_t t = frame + i;
            const bool level = period && t >= onset && (t - onset) % period < high;
            gates[i] = stmlib::ExtractGateFlags(previous[slot], level);
            previous[slot] = gates[i];
          }
          generators[slot].Process(gates.data(), output.data(), size);
          for (unsigned i = 0; i < count; ++i) {
            monitored[i] = output;
            generators[slaves[i]].Process(gates.data(), monitored[i].data(), size);
          }
          for (size_t i = 0; i < size; ++i) {
            auto emit = [&](unsigned channel, const stages::SegmentGenerator::Output& value) {
              require(std::isfinite(value.value) && std::isfinite(value.phase), "nonfinite source output");
              std::cout << '[' << frame + i << ',' << channel << ',' << unsigned(gates[i]) << ','
                << bits(value.value) << ',' << bits(value.phase) << ',' << value.segment << ','
                << stmlib::Random::state() << "]\n";
            };
            emit(slot, output[i]);
            for (unsigned j = 0; j < count; ++j) emit(slaves[j], monitored[j][i]);
          }
          frame += size;
        }
      } else throw std::runtime_error("unknown operation");
      require(bool(std::cin), "truncated operation");
    }
    require(bool(std::cout), "output write failed");
    return 0;
  } catch (const std::exception& error) {
    std::cerr << error.what() << '\n';
    return 1;
  }
}
