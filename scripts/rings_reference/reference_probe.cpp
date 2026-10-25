// Original OpenPatch host adapter, MIT licensed. DSP and resources remain
// unmodified, separately attributed upstream files from source_manifest.json.
// The per-block steps below reproduce the DSP portion of FillBuffer in the
// pinned rings/rings.cc (Copyright 2015 Emilie Gillet, MIT): input zeroing
// when the exciter input is normalized, the input noise gate, the Strummer and
// Part calls and the Clip16 output conversion.
//
// Protocol (stdin, whitespace separated):
//   header: blocks block_size seed
//   per block: polyphony model strum internal_exciter internal_strum
//              internal_note chord tonic note fm structure brightness damping
//              position, then block_size int16 input samples (the codec's
//              right channel, the only one Rings reads)
// Output: one line per block holding block_size "odd even" int16 pairs (the
// codec's left and right channels: Part's out and aux buffers), then the
// strum flag as the Strummer left it (1 or 0).
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <stdexcept>

#include "rings/dsp/dsp.h"
#include "rings/dsp/part.h"
#include "rings/dsp/strummer.h"
#include "stmlib/dsp/dsp.h"
#include "stmlib/utils/random.h"

using namespace rings;

static void require(bool value, const char* message) {
  if (!value) throw std::runtime_error(message);
}

// The firmware owns these as zero-initialized globals (rings.cc). Several
// members (Part::step_counter_, FMVoice::previous_sample_, the reverb low-pass
// states, String::src_phase_, Plucker::comb_filter_gain_) are only zeroed by
// that static storage, so one fresh process per case reproduces the
// construction domain.
static uint16_t reverb_buffer[32768];
static Part part;
static Strummer strummer;
static float in[kMaxBlockSize];
static float out[kMaxBlockSize];
static float aux[kMaxBlockSize];
static const float kNoiseGateThreshold = 0.00003f;
static float in_level = 0.0f;
static short input[kMaxBlockSize];

int main() {
  try {
    size_t blocks, size;
    uint32_t seed;
    require(bool(std::cin >> blocks >> size >> seed), "invalid header");
    require(blocks > 0 && blocks <= 200000, "invalid block count");
    require(size == kMaxBlockSize, "unsupported block size");
    stmlib::Random::Seed(seed);
    // Init(): strummer, then part (string_synth is not used).
    strummer.Init(0.01f, kSampleRate / kMaxBlockSize);
    part.Init(reverb_buffer);
    for (size_t block = 0; block < blocks; ++block) {
      int polyphony, model, strum, internal_exciter, internal_strum,
          internal_note, chord;
      PerformanceState performance_state;
      Patch patch;
      require(bool(std::cin >> polyphony >> model >> strum >> internal_exciter
        >> internal_strum >> internal_note >> chord
        >> performance_state.tonic >> performance_state.note
        >> performance_state.fm >> patch.structure >> patch.brightness
        >> patch.damping >> patch.position), "invalid controls");
      require(polyphony >= 1 && polyphony <= 4, "invalid polyphony");
      require(model >= 0 && model < RESONATOR_MODEL_LAST, "invalid model");
      require(chord >= 0 && chord < kNumChords, "invalid chord");
      performance_state.strum = strum != 0;
      performance_state.internal_exciter = internal_exciter != 0;
      performance_state.internal_strum = internal_strum != 0;
      performance_state.internal_note = internal_note != 0;
      performance_state.chord = chord;
      // Ui::Init applies polyphony then model; the buttons do the same.
      if (polyphony != part.polyphony()) {
        part.set_polyphony(polyphony);
      }
      part.set_model(static_cast<ResonatorModel>(model));
      for (size_t i = 0; i < size; ++i) {
        int r;
        require(bool(std::cin >> r), "invalid samples");
        require(r >= -32768 && r <= 32767, "sample range");
        // CvScaler::DetectAudioNormalization clears the input while the
        // exciter input is normalized.
        input[i] = performance_state.internal_exciter ? 0 : static_cast<short>(r);
      }
      // Apply noise gate.
      for (size_t i = 0; i < size; ++i) {
        float in_sample = static_cast<float>(input[i]) / 32768.0f;
        float error, gain;
        error = in_sample * in_sample - in_level;
        in_level += error * (error > 0.0f ? 0.1f : 0.0001f);
        gain = in_level <= kNoiseGateThreshold
              ? (1.0f / kNoiseGateThreshold) * in_level : 1.0f;
        in[i] = gain * in_sample;
      }
      strummer.Process(in, size, &performance_state);
      part.Process(performance_state, patch, in, out, aux, size);
      for (size_t i = 0; i < size; ++i) {
        int odd = stmlib::Clip16(static_cast<int32_t>(out[i] * 32768.0f));
        int even = stmlib::Clip16(static_cast<int32_t>(aux[i] * 32768.0f));
        std::printf(i ? " %d %d" : "%d %d", odd, even);
      }
      std::printf(" %d\n", performance_state.strum ? 1 : 0);
    }
  } catch (const std::exception& error) {
    std::fprintf(stderr, "%s\n", error.what());
    return 1;
  }
  return 0;
}
