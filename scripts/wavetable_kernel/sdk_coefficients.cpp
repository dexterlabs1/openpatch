// SPDX-License-Identifier: MIT
// Original offline API adapter. Links to a locally supplied GPL Rack SDK only
// for numerical design; neither SDK code nor this executable ships in the app.
#include <dsp/minblep.hpp>
#include <common.hpp>
#include <array>
#include <cstdio>
int main() {
    std::array<float,512> coefficients{};
    rack::dsp::minBlepImpulse(16,16,coefficients.data());
    std::fprintf(stderr,"Rack SDK %s; minBlepImpulse(16,16)\n", rack::APP_VERSION.c_str());
    return std::fwrite(coefficients.data(), sizeof(float), coefficients.size(), stdout)==coefficients.size()?0:1;
}
