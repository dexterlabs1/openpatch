// SPDX-License-Identifier: MIT
// Original adapter. The included constructor/process are verbatim pinned RackMCP.
#include <rack.hpp>
#include "reference_listen.hpp"
#include <array>
#include <cstring>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
using namespace rack;
static uint32_t bits(float x) { uint32_t n; std::memcpy(&n,&x,4); return n; }
static float value(uint32_t n) { float x; std::memcpy(&x,&n,4); return x; }
struct Source : engine::Module {
    std::array<std::array<float,16>,2> values{};
    std::array<int,2> counts{};
    Source(){config(0,0,2,0);}
    void process(const ProcessArgs&) override {
        for(int p=0;p<2;++p){outputs[p].setChannels(counts[p]);
            for(int c=0;c<16;++c)outputs[p].setVoltage(c<counts[p]?values[p][c]:0.f,c);}
    }
};
struct Sink : engine::Module { Sink(){config(0,2,0,0);} };
struct Harness {
    Context context;
    engine::Engine* engine=nullptr;
    Source* source=nullptr;
    rackmcp::ListenModule* module=nullptr;
    Sink* sink=nullptr;
    Harness(int rate,int inputMask,int outputMask){
        if(contextGet())throw std::runtime_error("unexpected context");
        contextSet(&context);settings::threadCount=1;settings::cpuMeter=false;
        engine=new engine::Engine;context.engine=engine;engine->setSampleRate(rate);
        source=new Source;source->id=1;engine->addModule(source);
        module=new rackmcp::ListenModule;module->id=2;engine->addModule(module);
        sink=new Sink;sink->id=3;engine->addModule(sink);
        for(int p=0;p<2;++p){
            if(inputMask&(1<<p)){auto* c=new engine::Cable;c->id=100+p;c->outputModule=source;c->outputId=p;c->inputModule=module;c->inputId=p;engine->addCable(c);}
            if(outputMask&(1<<p)){auto* c=new engine::Cable;c->id=200+p;c->outputModule=module;c->outputId=p;c->inputModule=sink;c->inputId=p;engine->addCable(c);}
        }
    }
    ~Harness(){engine->clear();context.engine=nullptr;delete engine;contextSet(nullptr);}
    template<class Ports> void ports(const Ports& ports){for(const auto& p:ports){std::cout<<' '<<int(p.channels);for(int i=0;i<16;++i)std::cout<<' '<<bits(p.voltages[i]);}}
    void snapshot(){auto tap=module->tap();std::cout<<tap->framesSeen.load()<<' '<<tap->dropped.load()<<' '<<tap->leftConnected.load()<<' '<<tap->rightConnected.load()<<' '<<(module->lights[0].getBrightness()>0)<<' '<<bits(module->lights[0].getBrightness());ports(module->inputs);ports(module->outputs);std::cout<<' '<<tap->ring.size()<<'\n';}
};
int main(){try{
    std::unique_ptr<Harness> h;std::string op;
    while(std::cin>>op){
        if(op=="new"){int rate,in,out;std::cin>>rate>>in>>out;h.reset();h.reset(new Harness(rate,in,out));}
        else if(op=="source"){int p,n;std::cin>>p>>n;h->source->counts.at(p)=n;h->source->values.at(p).fill(0.f);for(int i=0;i<n;++i){uint32_t b;std::cin>>b;h->source->values.at(p).at(i)=value(b);}}
        else if(op=="arm"){int armed;std::cin>>armed;h->module->tap()->armed.store(armed!=0);}
        else if(op=="bypass"){int bypass;std::cin>>bypass;h->engine->bypassModule(h->module,bypass!=0);}
        else if(op=="step"){int n;std::cin>>n;for(int i=0;i<n;++i)h->engine->stepBlock(1);}
        else if(op=="drain"){size_t n;std::cin>>n;std::vector<rackmcp::StereoFrame> f(n);size_t read=h->module->tap()->ring.pop(f.data(),n);std::cout<<"drain "<<read;for(size_t i=0;i<read;++i)std::cout<<' '<<bits(f[i].l)<<' '<<bits(f[i].r);std::cout<<'\n';}
        else throw std::runtime_error("unknown input operation");
        if(!std::cin)throw std::runtime_error("invalid fixture");h->snapshot();
    }
    return std::cin.eof()?0:1;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
