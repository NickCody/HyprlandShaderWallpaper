const float PI = 3.1415926535897932384626433832795;
const float PI_2 = 1.57079632679489661923;
const float PI_4 = 0.785398163397448309616;

// Change these parameters for different effects
//
float u_rotated_scale = 0.01;
float u_primary_scale = 0.01;
float u_rot_left_divisor = -500.0;
float u_rot_right_divisor = 500.0;

//
// Description : Array and textureless GLSL 2D simplex noise function.
//      Author : Ian McEwan, Ashima Arts.
//  Maintainer : ijm
//     Lastmod : 20110822 (ijm)
//     License : Copyright (C) 2011 Ashima Arts. All rights reserved.
//               Distributed under the MIT License. See LICENSE file.
//               https://github.com/ashima/webgl-noise
// simpled by guowei
// https://github.com/guoweish/glsl-noise-simplex

vec3 mod289(vec3 x) {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

vec2 mod289(vec2 x) {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

vec3 permute(vec3 x) {
    return mod289(((x*34.0)+1.0)*x);
}

float snoise(vec2 v)
{
    const vec4 C = vec4(
        0.211324865405187,  // (3.0-sqrt(3.0))/6.0
        0.366025403784439,  // 0.5*(sqrt(3.0)-1.0)
        -0.577350269189626,  // -1.0 + 2.0 * C.x
        0.024390243902439
    ); // 1.0 / 41.0

    // First corner
    vec2 i  = floor(v + dot(v, C.yy) );
    vec2 x0 = v -   i + dot(i, C.xx);

    // Other corners
    vec2 i1;
    //i1.x = step( x0.y, x0.x ); // x0.x > x0.y ? 1.0 : 0.0
    //i1.y = 1.0 - i1.x;
    i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
    // x0 = x0 - 0.0 + 0.0 * C.xx ;
    // x1 = x0 - i1 + 1.0 * C.xx ;
    // x2 = x0 - 1.0 + 2.0 * C.xx ;
    vec4 x12 = x0.xyxy + C.xxzz;
    x12.xy -= i1;

    // Permutations
    i = mod289(i); // Avoid truncation effects in permutation
    vec3 p = permute( permute( i.y + vec3(0.0, i1.y, 1.0 )) + i.x + vec3(0.0, i1.x, 1.0 ));

    vec3 m = max(0.5 - vec3(dot(x0,x0), dot(x12.xy,x12.xy), dot(x12.zw,x12.zw)), 0.0);
    m = m*m ;
    m = m*m ;

    // Gradients: 41 points uniformly over a line, mapped onto a diamond.
    // The ring size 17*17 = 289 is close to a multiple of 41 (41*7 = 287)

    vec3 x = 2.0 * fract(p * C.www) - 1.0;
    vec3 h = abs(x) - 0.5;
    vec3 ox = floor(x + 0.5);
    vec3 a0 = x - ox;

    // Normalise gradients implicitly by scaling m
    // Approximation of: m *= inversesqrt( a0*a0 + h*h );
    m *= 1.79284291400159 - 0.85373472095314 * ( a0*a0 + h*h );

    // Compute final noise value at P
    vec3 g;
    g.x  = a0.x  * x0.x  + h.x  * x0.y;
    g.yz = a0.yz * x12.xz + h.yz * x12.yw;
    return 130.0 * dot(m, g);
}

vec2 rotate(vec2 v, float a) {
    float s = sin(a);
    float c = cos(a);
    mat2 m = mat2(c, -s, s, c);
    return m * v;
}

vec2 rotateOrigin(vec2 v, vec2 center, float a) {
    vec2 t = v - center;
    vec2 r = rotate(t, a);
    return r + center;
}

vec3 srgb_to_linear(vec3 c) {
    vec3 low = c / 12.92;
    vec3 high = pow((c + 0.055) / 1.055, vec3(2.4));
    vec3 cutoff = step(vec3(0.04045), c);
    return mix(low, high, cutoff);
}

vec3 rgb_to_hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));

    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

float hue_distance(float h, float target) {
    float raw = abs(h - target);
    return min(raw, 1.0 - raw);
}


void mainImage(out vec4 out_color, vec2 fragCoord) {
    vec3 white = vec3(1.0, 1.0, 1.0);
    vec3 black = vec3(0.0, 0.0, 0.0);

    vec2 rotated_resolution = iResolution.xy * u_rotated_scale;
    vec2 primary_resolution = iResolution.xy * u_primary_scale;

    vec2 rotated_fragCoord = gl_FragCoord.xy * u_rotated_scale;
    vec2 primary_fragCoord = gl_FragCoord.xy * u_primary_scale;

    vec2 rotated_center = rotated_resolution.xy/2.0;
    vec2 primary_center = primary_resolution.xy/2.0;

    vec2 coord0 = primary_fragCoord+primary_center;
    vec2 coord1 = rotateOrigin(rotated_fragCoord, rotated_center, iTime/u_rot_left_divisor);
    vec2 coord2 = rotateOrigin(rotated_fragCoord, rotated_center, iTime/u_rot_right_divisor);

    float n0 = snoise(coord0);
    float n1 = snoise(coord1);
    float n2 = snoise(coord2);
    float c = (n1 + n2)/2.0;

    float n = snoise(coord0 * c);

    float r = n;
    float g = n;
    float b = n;
    vec3 color = vec3(r, g, b);
    vec3 final_color = color;
    
    // Sample the photographic base layer from channel 0.
    vec2 uv = fragCoord / iResolution.xy;
    vec3 baseImage = texture(iChannel0, uv).rgb;
    vec3 baseHSV = rgb_to_hsv(baseImage);

    vec3 influence = mix(vec3(0.35), vec3(1.0), clamp(final_color, 0.0, 1.0));
    vec3 targetPurple = vec3(0.604, 0.392, 0.573);
    float targetHue = rgb_to_hsv(targetPurple).x;
    float hueWidth = 0.12;
    float hueDist = hue_distance(baseHSV.x, targetHue);
    float hueWeight = 1.0 - smoothstep(0.0, hueWidth, hueDist);
    float saturationWeight = smoothstep(0.05, 0.20, baseHSV.y);
    float purpleWeight = hueWeight * saturationWeight;
    vec3 modulatedInfluence = mix(vec3(1.0), influence, purpleWeight);
    vec3 finalFinalColor = modulatedInfluence * baseImage;

    out_color = vec4(finalFinalColor, 1.0);
}
