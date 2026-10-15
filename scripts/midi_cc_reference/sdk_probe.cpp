// SPDX-License-Identifier: MIT
// Original fixture adapter; the pinned Rack class is compiled from a temporary file.
#include "reference_module.hpp"
#include <array>
#include <iostream>
#include <sstream>
#include <string>

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    rack::core::MIDICC_CV module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]); args.sampleTime = 1.f / args.sampleRate;
    std::array<int,20> previous; previous.fill(-999);
    json_t* saved = nullptr;
    std::string line;
    while (std::getline(std::cin,line)) {
        std::istringstream in(line);
        int smooth,mpe,lsb,channel,mask,op,learn,n;
        if (!(in>>smooth>>mpe>>lsb>>channel>>mask>>op>>learn>>n)) return 3;
        std::array<int,20> config{smooth,mpe,lsb,channel};
        for (int i=0;i<16;i++) in>>config[4+i];
        if (config != previous) {
            // Actual saved-array loader supplies collision handling. Options are
            // public configuration fields; channel filtering belongs to InputDevice.
            if (!std::equal(config.begin()+4,config.end(),previous.begin()+4)) {
                json_t* root=json_object(); json_t* maps=json_array();
                for (int i=0;i<16;i++) json_array_append_new(maps,json_integer(config[4+i]));
                json_object_set_new(root,"ccs",maps);module.dataFromJson(root);json_decref(root);
            }
            module.smooth=smooth;module.mpeMode=mpe;module.lsbMode=lsb;
            module.learningId=-1; previous=config;
        }
        if (op==1) {
            json_t* root=json_object(); json_t* vals=json_array();
            for (int i=0,v;i<128;i++) {in>>v;json_array_append_new(vals,json_integer(v));}
            json_object_set_new(root,"values",vals);module.dataFromJson(root);json_decref(root);module.learningId=-1;
        }
        if (op==2) {
            if(saved)json_decref(saved); saved=module.dataToJson();
            // The per-module hardware port JSON is outside this oracle: no
            // device/driver is selected and no MIDI endpoint is ever opened.
            json_object_del(saved,"midi");
        }
        if (op==3) {if(!saved)return 4;module.dataFromJson(saved);module.learningId=-1;}
        if (learn>0) module.learningId=learn-1;
        int priorLearning=module.learningId;
        for(int i=0;i<n;i++) {
            int status,wire,cc,value;in>>status>>wire>>cc>>value;
            rack::midi::Message message;message.bytes={uint8_t((status<<4)|wire),uint8_t(cc),uint8_t(value)};message.frame=args.frame;
            // Exact InputDevice::onMessage channel rule from the pinned source,
            // applied before the genuine SDK InputQueue (no device/context).
            if(status!=15 && channel!=0 && wire!=channel-1)continue;
            module.midiInput.onMessage(message);
        }
        for(int i=0;i<16;i++)module.outputs[i].channels=(mask&(1<<i))?1:0;
        module.process(args);
        int32_t metadata[162];int index=0;
        for(int i=0;i<16;i++)metadata[index++]=module.learnedCcs[i];
        for(int i=0;i<128;i++)metadata[index++]=module.ccValues[i][0];
        metadata[index++]=module.learningId;
        metadata[index++]=(priorLearning>=0 && module.learningId<0)?priorLearning:-1;
        for(int i=0;i<16;i++)metadata[index++]=module.outputs[i].getChannels();
        std::cout.write(reinterpret_cast<char*>(metadata),sizeof(metadata));
        float voltages[256];index=0;
        for(int i=0;i<16;i++)for(int lane=0;lane<16;lane++)voltages[index++]=lane<module.outputs[i].getChannels()?module.outputs[i].getVoltage(lane):0.f;
        std::cout.write(reinterpret_cast<char*>(voltages),sizeof(voltages));
        args.frame++;
    }
    if(saved)json_decref(saved);
}
