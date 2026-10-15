/* SPDX-License-Identifier: MIT
 * Binary driver for the pinned C source preflight. Not a numerical oracle:
 * numerical comparisons still use genuine SDK SRC symbols when defined.
 */
#include "samplerate.h"
#include <stdint.h>
#include <stdio.h>
#include <math.h>
long reference_call_index = -1;
long reference_frame_index = -1;
#define READ(value) (fread(&(value),sizeof(value),1,stdin)==1)
#define WRITE(value) do { if(fwrite(&(value),sizeof(value),1,stdout)!=1)return 5; } while(0)
int main(void) {
    int error=0;
    SRC_STATE* state=src_new(SRC_SINC_FASTEST,1,&error);
    uint32_t input_frames,capacity;
    if(!state)return 2;
    while(READ(input_frames)) {
        double ratio;
        float input[16]={0},output[16]={0};
        SRC_DATA data={0};
        uint32_t used,generated,i;
        reference_call_index++;
        if(!READ(capacity)||!READ(ratio)||input_frames>16||capacity>16)return 3;
        for(i=0;i<input_frames;i++)if(!READ(input[i]))return 3;
        data.data_in=input;data.data_out=output;
        data.input_frames=input_frames;data.output_frames=capacity;
        data.src_ratio=ratio;data.end_of_input=0;
        error=src_process(state,&data);
        if(error){fprintf(stderr,"src_process error %d\n",error);return 4;}
        used=(uint32_t)data.input_frames_used;generated=(uint32_t)data.output_frames_gen;
        WRITE(used);WRITE(generated);
        for(i=0;i<generated;i++){if(!isfinite(output[i]))return 6;WRITE(output[i]);}
    }
    src_delete(state);
    return ferror(stdin)?3:0;
}
