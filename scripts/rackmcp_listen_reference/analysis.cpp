// SPDX-License-Identifier: MIT
// Original probe. Link the untouched pinned RackMCP listen_analysis.cpp.
#include "core/listen_analysis.hpp"
#include <cstring>
#include <fstream>
#include <iostream>
#include <iterator>
#include <stdexcept>
#include <vector>
using namespace rackmcp::listen;
uint32_t bits(float x) { uint32_t y; std::memcpy(&y, &x, 4); return y; }
uint64_t bits(double x) { uint64_t y; std::memcpy(&y, &x, 8); return y; }
int main(int argc, char** argv) {
    if (argc != 6) return 2;
    std::ifstream input(argv[1], std::ios::binary);
    std::vector<char> bytes((std::istreambuf_iterator<char>(input)), {});
    if (!input || bytes.size() % 8) return 3;
    std::vector<float> l(bytes.size()/8), r(l.size());
    for (size_t i=0; i<l.size(); ++i) {
        std::memcpy(&l[i], &bytes[i*8], 4); std::memcpy(&r[i], &bytes[i*8+4], 4);
    }
    const auto result = analyze(l.data(), r.data(), l.size(), std::stof(argv[2]), std::stoi(argv[3]) != 0, std::stoi(argv[4]) != 0);
    bool first=true;
    std::cout << "{";
    auto emit = [&](const std::string& key, uint64_t value) {
        if (!first) std::cout << ','; first=false;
        std::cout << '"' << key << "\":" << value;
    };
    emit("sampleRate", bits(result.sampleRate)); emit("frames", result.frames);
    for (size_t i=0;i<2;++i) {
        const auto& c=result.channels[i]; const std::string p="channels."+std::to_string(i)+".";
        emit(p+"present",c.present); emit(p+"peakVolts",bits(c.peakVolts));
        emit(p+"rmsVolts",bits(c.rmsVolts)); emit(p+"dcVolts",bits(c.dcVolts));
        emit(p+"clippedSamples",c.clippedSamples); emit(p+"loudnessValid",c.loudnessValid);
        emit(p+"loudnessLufs",bits(c.loudnessLufs));
        emit(p+"peakDbfs",bits(voltsToDbfs(c.peakVolts)));
        emit(p+"rmsDbfs",bits(voltsToDbfs(c.rmsVolts)));
    }
#define FIELD(name) emit(#name, bits(result.name))
#define FLAG(name) emit(#name, result.name)
    FLAG(correlationValid); FIELD(correlation); FLAG(loudnessValid); FIELD(integratedLufs);
    FLAG(momentaryMaxValid); FIELD(momentaryMaxLufs); emit("bands.len",result.bands.size());
    for(size_t i=0;i<result.bands.size();++i) {
        const auto& b=result.bands[i]; const std::string p="bands."+std::to_string(i)+".";
        emit(p+"centerHz",bits(b.centerHz)); emit(p+"levelDb",bits(b.levelDb));
    }
    FLAG(centroidValid); FIELD(centroidHz); FLAG(onsetCount); FLAG(tempoValid); FIELD(tempoBpm);
    FIELD(tempoConfidence); FLAG(pitchValid); FIELD(pitchHz); FIELD(pitchConfidence);
    float cents=0; const auto note=noteName(result.pitchHz,cents); emit("noteCents",bits(cents));
    std::cout << ",\"noteName\":\"" << note << "\"}" << std::endl;
    // Use real R bytes even when its connection flag is false: raw codec check.
    const auto wav=encodeWavFloatStereo(l.data(),r.data(),l.size(),std::stoul(argv[2]));
    std::ofstream out(argv[5],std::ios::binary); out.write((const char*)wav.data(),wav.size());
    // Separately verify the source's nullptr / native None export policy.
    const auto mono=encodeWavFloatStereo(l.data(),nullptr,l.size(),std::stoul(argv[2]));
    std::ofstream mout(std::string(argv[5])+".mono",std::ios::binary); mout.write((const char*)mono.data(),mono.size());
    return out && mout ? 0 : 4;
}
