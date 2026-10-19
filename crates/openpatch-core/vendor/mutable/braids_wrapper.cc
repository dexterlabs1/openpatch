// SPDX-License-Identifier: MIT
// Original OpenPatch host adapter for the MIT-licensed Braids firmware DSP.
#include "braids/macro_oscillator.h"
#include "stmlib/utils/random.h"
#include <cstdlib>
#include <cstring>
#include <new>
#include <algorithm>

struct BraidsVoice {
    braids::MacroOscillator oscillator;
};
extern "C" {
void* openpatch_braids_new(uint32_t seed) {
    void* memory = std::calloc(1, sizeof(BraidsVoice));
    if (!memory) return nullptr;
    auto* voice = new(memory) BraidsVoice;
    voice->oscillator.Init();
    voice->oscillator.Seed(seed ? seed : 1);
    return voice;
}
void openpatch_braids_free(void* memory) {
    if (memory) { static_cast<BraidsVoice*>(memory)->~BraidsVoice(); std::free(memory); }
}
void openpatch_braids_reset(void* memory, uint32_t seed) {
    auto* voice = static_cast<BraidsVoice*>(memory);
    voice->~BraidsVoice();
    std::memset(memory, 0, sizeof(BraidsVoice));
    new(memory) BraidsVoice;
    voice->oscillator.Init(); voice->oscillator.Seed(seed ? seed : 1);
}
// The audited firmware classes contain only inline arrays and scalar state;
// no owned pointers, polymorphism, heap handles or references are present.
void openpatch_braids_copy(void* destination, const void* source) {
    std::memcpy(destination, source, sizeof(BraidsVoice));
}
void openpatch_braids_render(void* memory, int model, int pitch, int timbre,
                             int color, int strike, int16_t* output) {
    auto* voice = static_cast<BraidsVoice*>(memory);
    voice->oscillator.set_shape(static_cast<braids::MacroOscillatorShape>(std::clamp(model,0,46)));
    voice->oscillator.set_pitch(static_cast<int16_t>(std::clamp(pitch,0,16383)));
    voice->oscillator.set_parameters(static_cast<int16_t>(std::clamp(timbre,0,32767)),static_cast<int16_t>(std::clamp(color,0,32767)));
    if (strike) voice->oscillator.Strike();
    const uint8_t sync[24] = {};
    voice->oscillator.Render(sync, output, 24);
}
}
