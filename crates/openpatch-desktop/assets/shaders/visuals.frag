#version 330 core
in vec2 v_uv;
out vec4 frag_color;
uniform vec2 u_resolution;
uniform float u_time;
uniform float u_level;
uniform float u_wave[256];
uniform float u_spectrum[64];
uniform int u_mode; // 0 aurora, 1 spectrum, 2 waveform, 3 ferrofluid
uniform vec4 u_ferro; // smoothed low, middle, high and peak envelopes

// A bounded procedural surface inspired by ferrofluid in a magnetic field.
// Geometry, lighting and analysis stay on the visual thread / GPU.
// Front-on liquid sculpture. Surface displacement follows three smoothed audio
// bands and recent output peaks, with smooth liquid surface motion.
float ferroShape(vec3 p) {
    float radius=length(p);
    vec3 n=p/max(radius,0.0001);
    float turn=u_time*0.16;
    mat2 rotation=mat2(cos(turn),-sin(turn),sin(turn),cos(turn));
    n.xy=rotation*n.xy;
    n += vec3(sin(n.y*4.0+u_time*.7),sin(n.z*4.0-u_time*.5),cos(n.x*4.0+u_time*.6))*0.05*(0.2+u_ferro.y);
    n=normalize(n);
    vec3 cell=fract(n*(3.2+u_ferro.z*0.65)+vec3(0.5,0.5,0.0))-0.5;
    float tip=pow(max(1.0-length(cell)*1.7,0.0),2.8);
    // A continuous field keeps the liquid normals smooth between audio frames.
    float wave=sin(n.x*7.0+u_time*1.7)*sin(n.y*6.0-u_time*1.3)*u_ferro.y;
    float energy=clamp(u_ferro.x*0.65+u_ferro.w*0.35,0.0,1.0);
    float body=0.73+energy*0.22+sin(n.x*4.0+n.y*3.0+u_time*0.8)*0.045*u_ferro.y;
    float spikes=(0.10+energy*0.70+u_ferro.z*0.22)*tip;
    return radius-body-spikes-wave*0.065*(0.2+energy);
}
float ferroScene(vec3 p) { return ferroShape(p)*0.14; }
vec3 ferroNormal(vec3 p) {
    vec2 e=vec2(0.001,0.0);
    return normalize(vec3(ferroShape(p+e.xyy)-ferroShape(p-e.xyy),ferroShape(p+e.yxy)-ferroShape(p-e.yxy),ferroShape(p+e.yyx)-ferroShape(p-e.yyx)));
}
vec3 studioReflection(vec3 r) {
    // Broad studio softboxes, a narrow silver strip and quiet colored edge cards.
    vec3 c = vec3(0.025,0.033,0.042)*(0.45+max(r.y,0.0));
    float key = pow(max(dot(r,normalize(vec3(-0.45,0.85,0.35))),0.0),22.0);
    float fill = pow(max(dot(r,normalize(vec3(0.7,0.55,-0.35))),0.0),34.0);
    float strip = exp(-pow((r.x+0.18)/0.085,2.0))*smoothstep(0.0,0.5,r.y);
    c += vec3(1.0,0.91,0.77)*key*2.5 + vec3(0.48,0.7,0.8)*fill*1.7;
    c += vec3(0.83,0.9,0.94)*strip*0.8;
    c += vec3(0.50,0.63,0.67)*pow(max(dot(r,normalize(vec3(0.15,0.35,1.0))),0.0),6.0)*0.75;
    c += vec3(0.07,0.23,0.22)*pow(max(r.z,0.0),8.0)*0.5;
    return c;
}
vec3 ferrofluid(vec2 uv) {
    float aspect=u_resolution.x/max(u_resolution.y,1.0);
    vec2 screen=(uv-0.5)*vec2(aspect,1.0)*3.3;
    vec3 color=mix(vec3(0.009,0.015,0.02),vec3(0.035,0.047,0.053),uv.y);
    vec3 origin=vec3(screen,3.0),direction=vec3(0.0,0.0,-1.0);
    vec3 p=origin;float t=1.1;bool hit=false;
    if (length(screen)<1.95) {
        for (int i=0;i<120;i++) {
            p=origin+direction*t;
            float d=ferroScene(p);
            if(d<0.0008){hit=true;break;}
            t+=max(d,0.0005);
            if(t>4.9)break;
        }
    }
    if(hit){
        vec3 n=ferroNormal(p);float facing=max(dot(n,-direction),0.0);
        vec3 reflected=reflect(direction,n);
        float occlusion=clamp(0.6+ferroShape(p+n*0.08)*3.0,0.35,1.0);
        float fresnel=0.18+0.82*pow(1.0-facing,5.0);
        color=vec3(0.030,0.043,0.047)*(0.55+max(n.y,0.0)*0.5)*occlusion;
        color+=studioReflection(reflected)*(0.60+fresnel)*occlusion;
        color+=vec3(0.08,0.25,0.23)*pow(1.0-facing,3.0)*0.24;
        color=color/(1.0+color);
    }else{color*=1.0-exp(-dot(screen,screen)*0.7)*0.48;}
    return pow(max(color,vec3(0.0)),vec3(0.78));
}

void main() {
    if (u_mode == 3) { frag_color=vec4(ferrofluid(v_uv),1.0); return; }
    vec2 uv = v_uv;
    vec2 p = (uv - 0.5) * vec2(u_resolution.x/u_resolution.y, 1.0);
    vec3 col = vec3(0.019, 0.028, 0.054);
    float grid = pow(max(0.0, cos(p.x*48.0)), 32.0)*0.012
               + pow(max(0.0, cos(p.y*48.0)), 32.0)*0.012;
    col += vec3(0.2,0.55,0.7)*grid;
    if (u_mode == 0) {
        for (int i=0; i<7; i++) {
            float f = float(i);
            float y = sin(p.x*(1.5+f*0.16)+u_time*(0.2+f*0.045)+f*0.45)*0.15;
            y += sin(p.x*3.0-u_time*0.3+f)*0.045;
            y += u_wave[int(clamp(uv.x*255.0,0.0,255.0))]*0.30;
            float d = abs(p.y-y+(f-3.0)*0.025);
            vec3 tint = mix(vec3(0.04,0.8,0.74),vec3(0.48,0.25,0.97),f/6.0);
            col += tint*(0.0018/(d+0.004)+exp(-d*18.0)*0.08)*(0.38+u_level*3.0);
        }
    } else if (u_mode == 1) {
        float bin = clamp(floor(uv.x*64.0),0.0,63.0);
        float amplitude = clamp(sqrt(u_spectrum[int(bin)])*2.3,0.0,0.9);
        float bar = step(0.12,fract(uv.x*64.0))*step(uv.y,amplitude+0.045)*step(0.045,uv.y);
        col += mix(vec3(0.08,0.85,0.75),vec3(0.55,0.3,0.98),uv.x)*bar*(0.45+uv.y);
    } else {
        float index = clamp(uv.x*254.0,0.0,254.0);
        float wave = mix(u_wave[int(index)],u_wave[int(index)+1],fract(index));
        float d = abs(uv.y-0.5-wave*1.3);
        col += vec3(0.12,0.9,0.75)*(0.003/(d+0.004)+exp(-d*80.0)*0.1);
    }
    col *= 1.0-smoothstep(0.25,0.85,length(uv-0.5))*0.5;
    frag_color = vec4(col,1.0);
}
