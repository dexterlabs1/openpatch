// Original pinned firmware executes below; this file only drives and observes it.
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <string>
#include <sstream>
#include <cstring>
// Hardware register types are irrelevant to this raw kernel. Preserve source LED bits.
#define GRIDS_HARDWARE_CONFIG_H_
namespace grids { enum { LED_BD = 8, LED_SD = 4, LED_HH = 2, INPUT_CLOCK = 2, INPUT_RESET = 4 }; }
#define private public
#include "grids/pattern_generator.h"
#include "grids/clock.h"
#include "avrlib/random.h"
#undef private
#include "grids/resources.h"
#include "grids/pattern_generator.cc"
#include "grids/clock.cc"
#include "grids/resources.cc"
#include "avrlib/random.cc"

using P = grids::PatternGenerator;
using C = grids::Clock;
namespace original_isr {
grids::Clock& clock = grids::clock;
grids::PatternGenerator& pattern_generator = grids::pattern_generator;
using namespace grids;
struct InputsStub {
  uint8_t value = 0;
  uint8_t Read() const { return static_cast<uint8_t>(~value); }
} inputs;
struct MidiStub { bool readable() const { return false; } uint8_t ImmediateRead() const { return 0; } } midi;
uint32_t tap_duration = 0;
int8_t swing_amount = 0;
#include "original_clock_inputs.h"
void scan(bool clock_input, bool reset_input) {
  inputs.value = (clock_input ? INPUT_CLOCK : 0) | (reset_input ? INPUT_RESET : 0);
  ++tap_duration;
  HandleClockResetInputs();
  pattern_generator.IncrementPulseCounter();
}
}

static void snapshot(unsigned index) {
  std::printf("[%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%u,%d,%u,%u,%u,%u,%u,%u,%u,%u]\n",
    index, P::state_, P::pulse_, P::step_,
    P::euclidean_step_[0], P::euclidean_step_[1], P::euclidean_step_[2],
    P::part_perturbation_[0], P::part_perturbation_[1], P::part_perturbation_[2],
    P::pulse_duration_counter_, P::first_beat_, P::beat_, avrlib::Random::state(),
    int(P::swing_amount()), P::options_.pack(), C::locked_, C::bpm_, C::phase_,
    C::phase_increment_, C::falling_edge_, C::raising_edge(), C::past_falling_edge());
}

int main(int argc, char** argv) {
  if (argc > 1) {
    if (std::string(argv[1]) == "--undefined-clock") C::Update(512, grids::CLOCK_RESOLUTION_24_PPQN);
    else if (std::string(argv[1]) == "--undefined-map") std::printf("%u\n", P::ReadDrumMap(96, 0, 0, 0));
    else if (std::string(argv[1]) == "--undefined-options-4") P::options_.unpack(4);
    else if (std::string(argv[1]) == "--undefined-options-7") P::options_.unpack(7);
    else return 2;
    std::printf("%u\n", C::phase_increment_);
    return 0;
  }
  unsigned seed;
  if (!(std::cin >> seed)) return 2;
  avrlib::Random::Seed(static_cast<uint16_t>(seed));
  C::Init();
  P::factory_testing_ = 5;
  std::string line;
  std::getline(std::cin, line);
  unsigned index = 0;
  while (std::getline(std::cin, line)) {
    if (line.empty()) continue;
    std::istringstream row(line);
    unsigned op, repeat;
    if (!(row >> op >> repeat)) return 3;
    unsigned a[7] = {};
    for (unsigned& value : a) { if (!(row >> value)) break; }
    for (unsigned n = 0; n < repeat; ++n) {
      switch (op) {
        case 0:
          P::options_.unpack(a[0]);
          for (unsigned i=0;i<3;++i) {
            P::settings_.options.euclidean_length[i] = a[1+i];
            P::settings_.density[i] = a[4+i];
          }
          break;
        case 1: P::TickClock(a[0]); break;
        case 2: P::Retrigger(); break;
        case 3: P::IncrementPulseCounter(); break;
        case 4: P::ClockFallingEdge(); break;
        case 5: P::Reset(); break;
        case 10: C::Update(a[0], static_cast<grids::ClockResolution>(a[1])); break;
        case 11: C::Tick(); break;
        case 12: C::Wrap(static_cast<int8_t>(a[0])); break;
        case 13: C::Reset(); break;
        case 14: C::Lock(); break;
        case 15: C::Unlock(); break;
        case 20:
          for (unsigned part=0;part<3;++part) for (unsigned step=0;step<32;++step)
            std::printf("[\"m\",%u,%u,%u,%u,%u]\n",step,part,a[0],a[1],P::ReadDrumMap(step,part,a[0],a[1]));
          break;
        case 40: original_isr::scan(a[0], a[1]); break;
        case 30:
          for(unsigned i=0;i<1024;++i) std::printf("[\"e\",%u,%u]\n",i,grids::lut_res_euclidean[i]);
          for(unsigned i=0;i<512;++i) std::printf("[\"t\",%u,%u]\n",i,grids::lut_res_tempo_phase_increment[i]);
          for(unsigned i=0;i<25;++i) for(unsigned j=0;j<96;++j)
            std::printf("[\"n\",%u,%u,%u]\n",i,j,grids::node_table[i][j]);
          break;
        default: return 4;
      }
      snapshot(index);
      if (op == 40) std::printf("[\"s\",%u,%u,%d,%u]\n", index, original_isr::tap_duration, int(original_isr::swing_amount), original_isr::inputs.value);
      ++index;
    }
  }
}
