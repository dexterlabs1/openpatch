// Original OpenPatch host adapter, MIT licensed. DSP and resources remain
// unmodified, separately attributed upstream files from source_manifest.json.
//
// Protocol (stdin, whitespace separated):
//   header: blocks block_size
//   per block: easter_egg carrier_shape drive_1 drive_2 algorithm parameter
//              frequency_shift_pot frequency_shift_cv phase_shift note
//              then block_size pairs of int16 input samples (left right)
// Output: one line per block holding block_size "left right" int16 pairs.
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <stdexcept>

#include "warps/dsp/modulator.h"

static void require(bool value, const char* message) {
  if (!value) throw std::runtime_error(message);
}

int main() {
  try {
    // The firmware owns a zero-initialized global Modulator. Several members
    // (amplifier level/gain state, previous phase/skew fields) are only
    // zero-initialized by that static storage, so one fresh process per case
    // reproduces that construction domain.
    static warps::Modulator modulator;
    static warps::ShortFrame input[warps::kMaxBlockSize];
    static warps::ShortFrame output[warps::kMaxBlockSize];
    size_t blocks, size;
    require(bool(std::cin >> blocks >> size), "invalid header");
    require(blocks > 0 && blocks <= 20000, "invalid block count");
    // Firmware block: 60 frames at 96 kHz. The filter bank requires a multiple
    // of 12 and the 6x down-converter's circular branch is used below 64.
    require(size == 60, "unsupported block size");
    modulator.Init(96000.0f);
    for (size_t block = 0; block < blocks; ++block) {
      int easter_egg, carrier_shape;
      warps::Parameters* p = modulator.mutable_parameters();
      require(bool(std::cin >> easter_egg >> carrier_shape >> p->channel_drive[0]
        >> p->channel_drive[1] >> p->modulation_algorithm >> p->modulation_parameter
        >> p->frequency_shift_pot >> p->frequency_shift_cv >> p->phase_shift
        >> p->note), "invalid controls");
      require(carrier_shape >= 0 && carrier_shape <= 3, "invalid carrier shape");
      require(easter_egg == 0 || easter_egg == 1, "invalid mode");
      p->carrier_shape = carrier_shape;
      modulator.set_easter_egg(easter_egg != 0);
      for (size_t i = 0; i < size; ++i) {
        int l, r;
        require(bool(std::cin >> l >> r), "invalid samples");
        require(l >= -32768 && l <= 32767 && r >= -32768 && r <= 32767, "sample range");
        input[i].l = static_cast<short>(l);
        input[i].r = static_cast<short>(r);
      }
      modulator.Process(input, output, size);
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
