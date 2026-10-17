// Test adapter only. Actual pinned engine/parser are linked, not reimplemented.
#include "core/sequencer.hpp"
#include <jansson.h>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <vector>
using namespace rackmcp;
static uint32_t bits(float v) { uint32_t b; std::memcpy(&b,&v,4); return b; }
static uint64_t bits(double v) { uint64_t b; std::memcpy(&b,&v,8); return b; }
int main(int argc,char** argv) {
    if(argc!=3) return 2;
    json_error_t error;
    json_t* root=json_load_file(argv[1],0,&error);
    if(!root) throw std::runtime_error(error.text);
    std::vector<seq::Data> versions(json_array_size(root));
    for(size_t i=0;i<versions.size();++i) {
        std::string err;
        if(!seq::validateData(json_array_get(root,i),err)) throw std::runtime_error(err);
        seq::parseData(json_array_get(root,i),versions[i]);
    }
    json_decref(root);
    seq::Engine engine;
    size_t current=0; bool loaded=false;
    auto adopt=[&](size_t index) {
        current=index;
        if(versions[current].selectedPattern!=engine.activePattern())
            engine.requestPattern(versions[current].selectedPattern,!loaded);
        loaded=true;
    };
    adopt(0);
    std::ifstream packets(argv[2]);
    int action,index,run,ext,rising; double value; float rate,bpm,swing;
    while(packets>>action>>index>>value>>rate>>bpm>>swing>>run>>ext>>rising) {
        switch(action) {
            case 1: engine.reset(); break;
            case 2: engine.seek(value); break;
            case 3: engine.requestPattern(index,value!=0); break;
            case 4: adopt(index); break;
        }
        seq::Outputs out={};
        engine.process(versions[current],rate,bpm,swing,run,ext,rising,out);
        std::cout<<engine.activePattern()<<' '<<engine.pendingPattern()<<' '
                 <<engine.stepIndex()<<' '<<engine.loopCount()<<' '<<bits(engine.beatPosition());
        for(int t=0;t<seq::TRACKS;++t) {
            std::cout<<' '<<out.voices[t];
            for(int v=0;v<seq::MAX_VOICES;++v) std::cout<<' '<<bits(out.pitch[t][v]);
            std::cout<<' '<<bits(out.gate[t])<<' '<<bits(out.vel[t])<<' '<<bits(out.accent[t]);
        }
        std::cout<<' '<<out.clock<<' '<<out.reset;
        for(const auto& lane:versions[current].lanes)
            std::cout<<' '<<bits(lane.used?seq::laneValue(lane,engine.beatPosition()/4):0.f);
        std::cout<<'\n';
    }
}
