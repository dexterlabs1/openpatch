// SPDX-License-Identifier: MIT
// Original binary adapter. Actual upstream classes are generated outside Git.
#include "reference_vco.hpp"
#include "reference_lfo.hpp"
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <stdexcept>
#include <limits>
#include <type_traits>

namespace wire {
template<class T> T get() {
    T value{};
    if (std::fread(&value, sizeof(value), 1, stdin) != 1)
        throw std::runtime_error("truncated input");
    return value;
}
template<class T> void put(T value) {
    if (std::fwrite(&value, sizeof(value), 1, stdout) != 1)
        throw std::runtime_error("output failure");
}
void bank(Wavetable& table, uint32_t waveLength, uint32_t count) {
    if (count > (1u<<20) || waveLength > (1u<<14))
        throw std::runtime_error("bank exceeds fixture limits");
    std::vector<float> samples(count);
    for (auto& value : samples) {
        value = get<float>();
        if (!std::isfinite(value)) throw std::runtime_error("nonfinite bank");
    }
    // Force actual setWaveLen() to rebuild even if replacing same-size waves.
    table.loading = true;
    table.samples = std::move(samples);
    table.setWaveLen(0);
    table.setWaveLen(waveLength);
    table.filename = "Synthetic.f32";
    table.loading = false;
}
}

template<class ModuleType> void run(float rate, uint32_t frames, uint32_t flags,
                                  uint32_t waveLength, uint32_t sampleCount,
                                  bool defaultBank) {
    ModuleType module;
    if (!defaultBank) wire::bank(module.wavetable, waveLength, sampleCount);
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate=rate; args.sampleTime=1.f/rate;
    for (uint32_t frame=0; frame<frames; ++frame) {
        uint32_t event=wire::get<uint32_t>();
        if (event==1) module.onReset();
        else if (event==2) {
            auto length=wire::get<uint32_t>(), count=wire::get<uint32_t>();
            wire::bank(module.wavetable,length,count);
        }
        else if (event==3) module.wavetable.loading=true;
        else if (event==4) module.wavetable.loading=false;
        else if (event==5) wire::bank(module.wavetable,0,0);
        else if (event==6) module.wavetable.reset();
        else if (event==7) module.wavetable.setWaveLen(wire::get<uint32_t>());
        // 8 = Rust adapter rebuilds/adopts current state before setting params;
        // actual mutable SDK module requires no corresponding operation.
        else if (event!=0 && event!=8) throw std::runtime_error("unknown event");
        for (auto& param: module.params) param.setValue(wire::get<float>());
        for (auto& port: module.inputs) {
            auto width=wire::get<uint32_t>();
            if (width>16) throw std::runtime_error("invalid input width");
            port.channels=width;
            for (uint32_t lane=0;lane<16;++lane) {
                float value=wire::get<float>();
                if (!std::isfinite(value)) throw std::runtime_error("nonfinite port");
                port.voltages[lane]=lane<width?value:0.f;
            }
        }
        // Mark connected so genuine Rack setChannels() updates output width.
        module.outputs[0].channels=1;
        args.frame=frame;
        module.process(args);
        uint32_t width=module.outputs[0].getChannels();
        wire::put(width);
        for(uint32_t lane=0;lane<16;++lane)
            wire::put(lane<width?module.outputs[0].getVoltage(lane):0.f);
        if (!(flags&1)) continue;
        for(auto& phase:module.phases) for(int lane=0;lane<4;++lane) wire::put(phase[lane]);
        if constexpr(std::is_same<ModuleType,WTVCO>::value) {
            for(auto& direction:module.syncDirections) for(int lane=0;lane<4;++lane) wire::put(direction[lane]);
            for(auto& value:module.lastSyncValues) for(int lane=0;lane<4;++lane) wire::put(value[lane]);
            wire::put(0.f); wire::put(0.f);
            wire::put(module.lastPos);
            for(int group=0;group<4;++group) wire::put(uint32_t(0));
            wire::put(uint32_t(0));
        }
        else {
            for(int lane=0;lane<32;++lane) wire::put(0.f);
            wire::put(module.clockFreq); wire::put(module.clockTimer.getTime());
            wire::put(module.lastPos);
            for(auto& trigger:module.resetTriggers)
                wire::put(uint32_t(rack::simd::movemask(trigger.isHigh())));
            wire::put(uint32_t(module.clockTrigger.s));
        }
        wire::put(uint32_t(module.wavetable.loading));
        wire::put(uint32_t(module.wavetable.waveLen));
        wire::put(uint32_t(module.wavetable.samples.size()));
    }
    if(std::fgetc(stdin)!=EOF) throw std::runtime_error("trailing input bytes");
}
int main() {
    try {
        uint32_t endian=1;
        if (*reinterpret_cast<uint8_t*>(&endian)!=1 || sizeof(float)!=4 || !std::numeric_limits<float>::is_iec559)
            throw std::runtime_error("requires little-endian IEEE f32");
        if(wire::get<uint32_t>()!=0x3154574f) throw std::runtime_error("bad OWT1 magic");
        auto kind=wire::get<uint32_t>();
        float rate=wire::get<float>();
        auto frames=wire::get<uint32_t>(), flags=wire::get<uint32_t>();
        auto waveLength=wire::get<uint32_t>(), count=wire::get<uint32_t>();
        auto defaults=wire::get<uint32_t>();
        if(!std::isfinite(rate)||rate<=0||flags>1||defaults>1||kind>1)
            throw std::runtime_error("invalid header");
        if(defaults&&count) throw std::runtime_error("default bank has sample bytes");
        if(kind==0) run<WTVCO>(rate,frames,flags,waveLength,count,defaults);
        else run<WTLFO>(rate,frames,flags,waveLength,count,defaults);
        return std::fflush(stdout)==0?0:3;
    } catch(const std::exception& error) {
        std::fprintf(stderr,"wavetable oracle: %s\n",error.what()); return 2;
    }
}
