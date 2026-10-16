// SPDX-License-Identifier: MIT
// Original offline numerical experiment, not an audio callback implementation.
//
// Method: Eli Brandt, "Hard Sync Without Aliasing", ICMC 2001, sections
// 4.2 and 6.1-6.3: window a sinc, perform real-cepstral minimum-phase
// reconstruction, and integrate/normalize the resulting impulse.
// https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf
// Even-length cepstral midpoint handling is documented independently at
// https://www.mathworks.com/help/signal/ref/rceps.html
//
// The four-term Blackman-Harris cosine coefficients below are the standard
// mathematical window coefficients. No Rack implementation or SDK output
// coefficients are incorporated. PFFFT is an external, separately licensed
// FFTPACK-derived dependency; retain its upstream license when distributing it.
// This file uses only its public real-transform API and ordered packing.
//
// Reproduction: compile this file against the pinned standalone PFFFT source,
// with floating-point contraction disabled and without fast-math. Record the
// PFFFT revision, compiler, architecture and flags alongside any comparison.
// The binary output is exactly 512 little-endian IEEE binary32 coefficients;
// a caller may append the playback endpoint 1.0. --hex emits one bit pattern
// per line instead. Output does not read, fit, or extract reference data.
//
// Default --drop-midpoint is an explicit alternative phase profile for the
// numerical experiment. --retain-midpoint selects standard even-length
// real-cepstral reconstruction, retaining both self-conjugate coefficients.
// Neither profile implies numerical equivalence to another implementation.

#include <pffft.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>

namespace {
constexpr std::size_t length = 512;
constexpr float crossings = 32.f;
constexpr float log_floor = -30.f;
constexpr double pi = 3.141592653589793238462643383279502884;
using Samples = std::array<float, length>;

class RealTransform {
    std::unique_ptr<PFFFT_Setup, decltype(&pffft_destroy_setup)> setup_;
    alignas(64) Samples work_{};

public:
    RealTransform()
        : setup_(pffft_new_setup(static_cast<int>(length), PFFFT_REAL),
                 &pffft_destroy_setup) {
        if (!setup_) throw std::runtime_error("PFFFT setup failed");
    }

    void forward(const Samples& time, Samples& spectrum) {
        pffft_transform_ordered(setup_.get(), time.data(), spectrum.data(),
                                work_.data(), PFFFT_FORWARD);
    }

    void inverse(const Samples& spectrum, Samples& time) {
        pffft_transform_ordered(setup_.get(), spectrum.data(), time.data(),
                                work_.data(), PFFFT_BACKWARD);
        for (auto& value : time) value /= static_cast<float>(length);
    }
};

Samples windowed_sinc() {
    Samples result{};
    const float window_pi = static_cast<float>(pi);
    constexpr std::array<float, 4> weights{.35875f, -.48829f, .14128f, -.01168f};
    for (std::size_t point = 0; point != length; ++point) {
        const float unit = static_cast<float>(point) / static_cast<float>(length - 1);
        const float location = crossings * unit - crossings / 2.f;
        // Convert the analytic sinc angle to binary32 before scalar sin.
        const float angle = static_cast<float>(pi * location);
        const float sinc = angle == 0.f ? 1.f : std::sin(angle) / angle;
        float window = 0.f;
        for (std::size_t harmonic = 0; harmonic != weights.size(); ++harmonic) {
            const float argument = (2.f * static_cast<float>(harmonic) * window_pi) * unit;
            window += weights[harmonic] * std::cos(argument);
        }
        result[point] = sinc * window;
    }
    return result;
}

float log_amplitude(float amplitude) {
    // A fixed floor makes the logarithm well-defined at exact spectral zeros.
    return std::max(log_floor, std::log(std::abs(amplitude)));
}

void logarithmic_magnitude(Samples& spectrum) {
    // Ordered real PFFFT packing places two independent real bins first:
    // [DC, Nyquist, real(1), imag(1), ..., real(N/2-1), imag(N/2-1)].
    spectrum[0] = log_amplitude(spectrum[0]);
    spectrum[1] = log_amplitude(spectrum[1]);
    for (std::size_t bin = 1; bin != length / 2; ++bin) {
        const float real = spectrum[2 * bin];
        const float imaginary = spectrum[2 * bin + 1];
        spectrum[2 * bin] = log_amplitude(std::sqrt(real * real + imaginary * imaginary));
        spectrum[2 * bin + 1] = 0.f;
    }
}

void exponential_spectrum(Samples& spectrum) {
    spectrum[0] = std::exp(spectrum[0]);
    spectrum[1] = std::exp(spectrum[1]);
    for (std::size_t bin = 1; bin != length / 2; ++bin) {
        const float modulus = std::exp(spectrum[2 * bin]);
        const float angle = spectrum[2 * bin + 1];
        spectrum[2 * bin] = modulus * std::cos(angle);
        spectrum[2 * bin + 1] = modulus * std::sin(angle);
    }
}

Samples design(bool retain_midpoint) {
    RealTransform transform;
    alignas(64) Samples time = windowed_sinc();
    alignas(64) Samples spectrum{};
    transform.forward(time, spectrum);
    logarithmic_magnitude(spectrum);
    transform.inverse(spectrum, time);

    // Keep DC once, keep causal positive quefrencies twice, remove negative
    // quefrencies. The midpoint policy is explicit rather than incidental.
    for (std::size_t index = 1; index != length; ++index) {
        const float weight = index < length / 2 ? 2.f
            : (index == length / 2 && retain_midpoint ? 1.f : 0.f);
        time[index] *= weight;
    }
    transform.forward(time, spectrum);
    exponential_spectrum(spectrum);
    transform.inverse(spectrum, time);

    float area = 0.f;
    for (auto& value : time) {
        area += value;
        value = area;
    }
    if (!std::isfinite(area) || area <= 0.f)
        throw std::runtime_error("invalid impulse area");
    for (auto& value : time) {
        value /= area;
        if (!std::isfinite(value)) throw std::runtime_error("nonfinite coefficient");
    }
    return time;
}

void emit(const Samples& values, bool hexadecimal) {
    static_assert(sizeof(float) == sizeof(std::uint32_t), "requires binary32");
    static_assert(std::numeric_limits<float>::is_iec559, "requires IEEE arithmetic");
    for (float value : values) {
        std::uint32_t bits;
        std::memcpy(&bits, &value, sizeof(bits));
        if (hexadecimal) {
            if (std::printf("0x%08x\n", static_cast<unsigned int>(bits)) < 0)
                throw std::runtime_error("hex output failed");
        } else {
            const std::array<unsigned char, 4> bytes{
                static_cast<unsigned char>(bits),
                static_cast<unsigned char>(bits >> 8),
                static_cast<unsigned char>(bits >> 16),
                static_cast<unsigned char>(bits >> 24),
            };
            if (std::fwrite(bytes.data(), 1, bytes.size(), stdout) != bytes.size())
                throw std::runtime_error("binary output failed");
        }
    }
    if (std::fflush(stdout) != 0) throw std::runtime_error("output flush failed");
}
} // namespace

int main(int argc, char** argv) {
    try {
        bool retain_midpoint = false;
        bool hexadecimal = false;
        for (int index = 1; index < argc; ++index) {
            const std::string argument = argv[index];
            if (argument == "--retain-midpoint") retain_midpoint = true;
            else if (argument == "--drop-midpoint") retain_midpoint = false;
            else if (argument == "--hex") hexadecimal = true;
            else if (argument == "--help") {
                std::puts("generate_pffft [--retain-midpoint|--drop-midpoint] [--hex]");
                return 0;
            } else throw std::runtime_error("unknown argument: " + argument);
        }
        emit(design(retain_midpoint), hexadecimal);
        return 0;
    } catch (const std::exception& error) {
        std::fprintf(stderr, "independent MinBLEP design: %s\n", error.what());
        return 1;
    }
}
