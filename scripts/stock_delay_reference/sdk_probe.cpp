// SPDX-License-Identifier: MIT
// Only adapters live here. reference_module.hpp is the untouched upstream
// Delay class prefix except its plugin.hpp include is replaced with rack.hpp.
#include "reference_module.hpp"
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <memory>
#ifdef REFERENCE_GUARD
extern "C" {
long reference_call_index=-1;
long reference_frame_index=-1;
}
#endif

template<class T> bool read(T& x) { return std::fread(&x,sizeof(x),1,stdin)==1; }
template<class T> void write(const T& x) { assert(std::fwrite(&x,sizeof(x),1,stdout)==1); }
int main(int argc,char** argv) {
    if(argc!=2)return 2;
    static_assert(sizeof(float)==4 && sizeof(uint32_t)==4,"binary protocol");
    auto module=std::make_unique<Delay>();
    rack::engine::Module::ProcessArgs args{};
    if(std::strcmp(argv[1],"--bypass-widths")==0) {
        args.sampleRate=48000.f;args.sampleTime=1.f/args.sampleRate;
        const unsigned widths[]={0,1,5,16};
        std::printf("[");
        for(unsigned test=0;test<4;test++) {
            auto bypass=std::make_unique<Delay>();
            const unsigned width=widths[test];
            auto& audio=bypass->inputs[Delay::IN_INPUT];audio.channels=width;
            for(unsigned lane=0;lane<16;lane++)audio.voltages[lane]=lane<width?float(lane+1)*.25f:0.f;
            for(auto& output:bypass->outputs)output.channels=1;
            bypass->processBypass(args);
            std::printf("%s{\"audio_channels\":%u,\"outputs\":[",test?",":"",width);
            for(unsigned out=0;out<2;out++) {
                auto& output=bypass->outputs[out];
                std::printf("%s{\"channels\":%d,\"voltages\":[",out?",":"",output.getChannels());
                for(unsigned lane=0;lane<16;lane++)std::printf("%s%.9g",lane?",":"",output.getVoltage(lane));
                std::printf("]}");
            }
            std::printf("]}");
        }
        std::printf("]\n");return 0;
    }
    args.sampleRate=std::stof(argv[1]);args.sampleTime=1.f/args.sampleRate;
    uint32_t control;
    while(read(control)) {
        if(control>1)return 3; // Native-only adoption; source continues intact.
        for(auto& param:module->params){float value;if(!read(value))return 3;param.setValue(value);}
        for(auto& input:module->inputs) {
            uint32_t width;if(!read(width)||width>16)return 3;
            input.channels=width;
            for(unsigned lane=0;lane<16;lane++) {
                float value=0.f;if(lane<width&&!read(value))return 3;
                input.voltages[lane]=value;
            }
        }
        for(auto& output:module->outputs)output.channels=1;
#ifdef REFERENCE_GUARD
        reference_frame_index=args.frame;
        // Read actual class state only to label its next SRC call. The actual
        // process body and converter call remain unchanged.
        if(module->outBuffer.empty())reference_call_index++;
#endif
        module->process(args);
        for(auto& output:module->outputs){assert(output.getChannels()==1);write(uint32_t(output.getChannels()));}
        for(auto& output:module->outputs){float value=output.getVoltage();assert(std::isfinite(value));write(value);}
        args.frame++;
    }
    if(std::ferror(stdin))return 3;
}
