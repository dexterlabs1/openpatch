// SPDX-License-Identifier: MIT
// Original adapter. The upstream class exists only in a temporary header.
#include "reference_module.hpp"
#include <iostream>
#include <iomanip>
#include <cassert>
int main(int argc,char**argv) {
    if(argc!=4)return 2;
    const uint64_t seed0=std::stoull(argv[2],nullptr,0),seed1=std::stoull(argv[3],nullptr,0);
    VCF module;
    rack::random::local().seed(seed0,seed1);
    const auto initial0=rack::random::local().state[0],initial1=rack::random::local().state[1];
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate=std::stof(argv[1]);args.sampleTime=1.f/args.sampleRate;
    const int ids[]={VCF::FREQ_PARAM,VCF::RES_PARAM,VCF::FREQ_CV_PARAM,VCF::RES_CV_PARAM,VCF::DRIVE_PARAM,VCF::DRIVE_CV_PARAM};
    std::cout<<std::setprecision(9);
    int mask;
    uint64_t draws=0;
    while(std::cin>>mask) {
        for(auto id:ids){float v;if(!(std::cin>>v))return 3;module.params[id].setValue(v);}
        for(auto &input:module.inputs){int width;if(!(std::cin>>width)||width<0||width>16)return 3;input.channels=width;for(int lane=0;lane<16;lane++){float v;if(!(std::cin>>v))return 3;input.voltages[lane]=lane<width?v:0.f;}}
        // Rack setChannels ignores disconnected output; establish graph
        // connectivity each frame before the unmodified actual process call.
        module.outputs[0].channels=mask&1?1:0;module.outputs[1].channels=mask&2?1:0;
        module.process(args);
        if(mask)draws+=(std::max(1,module.inputs[VCF::IN_INPUT].getChannels())+3)/4;
        const int width=std::max(1,module.inputs[VCF::IN_INPUT].getChannels());
        for(int out=0;out<2;out++) {
            if(mask&(1<<out)) assert(module.outputs[out].getChannels()==width);
            else assert(module.outputs[out].getChannels()==0);
        }
        // Host publishes width on both outputs; disconnected samples are zero.
        std::cout<<width;
        for(int out=0;out<2;out++)for(int lane=0;lane<16;lane++)std::cout<<' '<<((mask&(1<<out))&&lane<width?module.outputs[out].getVoltage(lane):0.f);
        std::cout<<'\n';args.frame++;
    }
    std::cerr<<initial0<<' '<<initial1<<' '<<rack::random::local().state[0]<<' '<<rack::random::local().state[1]<<' '<<draws<<'\n';
}
