// SPDX-License-Identifier: MIT
// Per-call qualification uses the real SRC symbols exported by libRack.
#include <samplerate.h>
#include <cassert>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <iostream>
template<class T> bool read(T& x){return std::fread(&x,sizeof(x),1,stdin)==1;}
template<class T> void write(const T& x){assert(std::fwrite(&x,sizeof(x),1,stdout)==1);}
int main(int argc,char**argv){
    if(argc==2&&std::strcmp(argv[1],"--identity")==0){std::cout<<src_get_version()<<'\n'<<src_get_name(SRC_SINC_FASTEST)<<'\n'<<src_get_description(SRC_SINC_FASTEST)<<'\n';return 0;}
    int error=0;SRC_STATE* state=src_new(SRC_SINC_FASTEST,1,&error);if(!state)return 2;
    uint32_t input_frames,output_frames;
    while(read(input_frames)){
        double ratio;if(!read(output_frames)||!read(ratio)||input_frames>16||output_frames>16)return 3;
        float input[16]={},output[16]={};for(uint32_t i=0;i<input_frames;i++)if(!read(input[i]))return 3;
        SRC_DATA data{};data.data_in=input;data.data_out=output;data.input_frames=input_frames;data.output_frames=output_frames;data.src_ratio=ratio;data.end_of_input=0;
        error=src_process(state,&data);if(error){std::cerr<<src_strerror(error)<<'\n';return 4;}
        write(uint32_t(data.input_frames_used));write(uint32_t(data.output_frames_gen));
        for(long i=0;i<data.output_frames_gen;i++){assert(std::isfinite(output[i]));write(output[i]);}
    }
    src_delete(state);return std::ferror(stdin)?3:0;
}
