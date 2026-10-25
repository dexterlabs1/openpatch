// Original OpenPatch host adapter, MIT licensed. DSP and resources remain
// unmodified, separately attributed upstream files from source_manifest.json.
//
// Protocol (stdin, whitespace separated):
//   header: blocks block_size prepare_calls seed
//   per block: mode quality freeze trigger gate position size pitch density
//              texture dry_wet stereo_spread feedback reverb
//              then block_size pairs of int16 input samples (left right)
// Per block the adapter applies the controls, calls Process once (the codec
// interrupt), then Prepare prepare_calls times (the firmware main loop).
// Output: one line per block holding block_size "left right" int16 pairs.
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <stdexcept>

#include "clouds/dsp/granular_processor.h"
#include "stmlib/utils/random.h"

static void require(bool value, const char* message) {
  if (!value) throw std::runtime_error(message);
}

// The firmware owns a zero-initialized global processor and two zeroed
// static memory blocks of exactly these sizes (clouds.cc). Several members
// (grain-rate phasor, reverb low-pass states, looper tail fields) are only
// zeroed by that static storage, so one fresh process per case reproduces
// the construction domain.
static clouds::GranularProcessor processor;
static uint8_t block_mem[118784];
static uint8_t block_ccm[65536 - 128];
static clouds::ShortFrame input[clouds::kMaxBlockSize];
static clouds::ShortFrame output[clouds::kMaxBlockSize];

int main() {
  try {
    size_t blocks, size, prepare_calls;
    uint32_t seed;
    require(bool(std::cin >> blocks >> size >> prepare_calls >> seed), "invalid header");
    require(blocks > 0 && blocks <= 40000, "invalid block count");
    require(size == 32, "unsupported block size");
    require(prepare_calls >= 1 && prepare_calls <= 64, "invalid prepare count");
    stmlib::Random::Seed(seed);
    processor.Init(block_mem, sizeof(block_mem), block_ccm, sizeof(block_ccm));
    for (size_t block = 0; block < blocks; ++block) {
      int mode, quality, freeze, trigger, gate;
      clouds::Parameters* p = processor.mutable_parameters();
      require(bool(std::cin >> mode >> quality >> freeze >> trigger >> gate
        >> p->position >> p->size >> p->pitch >> p->density >> p->texture
        >> p->dry_wet >> p->stereo_spread >> p->feedback >> p->reverb),
        "invalid controls");
      require(mode >= 0 && mode <= 3 && quality >= 0 && quality <= 3, "invalid mode");
      p->freeze = freeze != 0;
      p->trigger = trigger != 0;
      p->gate = gate != 0;
      processor.set_playback_mode(static_cast<clouds::PlaybackMode>(mode));
      processor.set_quality(quality);
      for (size_t i = 0; i < size; ++i) {
        int l, r;
        require(bool(std::cin >> l >> r), "invalid samples");
        require(l >= -32768 && l <= 32767 && r >= -32768 && r <= 32767, "sample range");
        input[i].l = static_cast<short>(l);
        input[i].r = static_cast<short>(r);
      }
      processor.Process(input, output, size);
      for (size_t i = 0; i < prepare_calls; ++i) {
        processor.Prepare();
      }
      for (size_t i = 0; i < size; ++i) {
        std::printf(i ? " %d %d" : "%d %d", int(output[i].l), int(output[i].r));
      }
      std::printf("\n");
    }
  } catch (const std::exception& error) {
    std::fprintf(stderr, "%s\n", error.what());
    return 1;
  }
  return 0;
}
