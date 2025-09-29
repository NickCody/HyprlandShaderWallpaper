const float PI = 3.1415926535897932384626433832795;
const float PI_2 = 1.57079632679489661923;
const float PI_4 = 0.785398163397448309616;

// Change these parameters for different effects
//
float u_rotated_scale = 0.009;
float u_primary_scale = 0.003;
float u_rot_left_divisor = 150.0;
float u_rot_right_divisor = 50.0;
float u_gradient_speed = 0.00; // Set to 0.0 to disable color cycling.
float g_dark_strength = 0.75;  // 1.0 drives the dark stop to pure black.
float g_light_strength = 0.75; // 1.0 pushes the light stop to pure white.
float g_tail_strength = 0.20;  // Multiplier for the trailing low-intensity stop (0.2 ≈ color/5).
float u_vibrance = 0.0;        // Negative values desaturate, positive boosts muted colors.
float u_contrast = 1.0;        // 1.0 leaves contrast unchanged.

const int GRADIENT_SIZE = 4;

// Key color presets (uncomment one to activate).
// const vec3 KEY_COLOR = vec3(0.0, 0.0, 0.0);        // black
// const vec3 KEY_COLOR = vec3(1.0, 1.0, 1.0);        // white
// const vec3 KEY_COLOR = vec3(0.078, 0.031, 0.008);  // sepia shadow
// const vec3 KEY_COLOR = vec3(0.349, 0.176, 0.102);  // sepia midtone
// const vec3 KEY_COLOR = vec3(0.604, 0.376, 0.212);  // sepia highlight
// const vec3 KEY_COLOR = vec3(0.957, 0.905, 0.678);  // sepia glow
// const vec3 KEY_COLOR = vec3(1.0, 0.0, 0.0);        // red
// const vec3 KEY_COLOR = vec3(1.0, 0.5, 0.0);        // orange
// const vec3 KEY_COLOR = vec3(1.0, 1.0, 0.0);        // yellow
// const vec3 KEY_COLOR = vec3(0.0, 1.0, 0.0);        // green
// const vec3 KEY_COLOR = vec3(0.0, 0.0, 1.0);        // blue
// const vec3 KEY_COLOR = vec3(0.29, 0.0, 0.51);      // indigo
// const vec3 KEY_COLOR = vec3(0.56, 0.0, 1.0);       // violet
// const vec3 KEY_COLOR = vec3(0.992, 0.733, 0.769);  // rose
// const vec3 KEY_COLOR = vec3(0.996, 0.894, 0.710);  // apricot
// const vec3 KEY_COLOR = vec3(0.996, 0.980, 0.749);  // lemon chiffon
// const vec3 KEY_COLOR = vec3(0.749, 0.937, 0.780);  // mint
// const vec3 KEY_COLOR = vec3(0.753, 0.843, 0.996);  // periwinkle
// const vec3 KEY_COLOR = vec3(0.886, 0.760, 0.996);  // mauve
// const vec3 KEY_COLOR = vec3(0.054, 0.176, 0.254);  // deep teal
// const vec3 KEY_COLOR = vec3(0.082, 0.360, 0.419);  // sea green
// const vec3 KEY_COLOR = vec3(0.290, 0.631, 0.592);  // aqua foam
// const vec3 KEY_COLOR = vec3(0.560, 0.835, 0.756);  // sea glass
// const vec3 KEY_COLOR = vec3(0.839, 0.949, 0.894);  // driftwood mist
// const vec3 KEY_COLOR = vec3(0.937, 0.780, 0.784);  // blush
// const vec3 KEY_COLOR = vec3(0.988, 0.902, 0.792);  // peach
// const vec3 KEY_COLOR = vec3(0.980, 0.925, 0.898);  // linen
// const vec3 KEY_COLOR = vec3(0.858, 0.792, 0.890);  // lavender
// const vec3 KEY_COLOR = vec3(0.776, 0.725, 0.858);  // wisteria
const vec3 KEY_COLOR = vec3(0.054, 0.176, 0.254);     // deep teal (active)

/*
 * Rec. 709 luma coefficients weight RGB by the human eye's sensitivity curve:
 * green dominates perceived brightness, red contributes less, and blue the least.
 * Using these values keeps vibrance/contrast adjustments aligned with perceived luminance.
 */
const vec3 LUMA_WEIGHTS = vec3(0.2126, 0.7152, 0.0722);

vec3 applyDarkShade(vec3 color) {
    float strength = clamp(g_dark_strength, 0.0, 1.0);
    return mix(color, vec3(0.0), strength);
}

vec3 applyLightShade(vec3 color) {
    float strength = clamp(g_light_strength, 0.0, 1.0);
    return mix(color, vec3(1.0), strength);
}

vec3 getGradientStop(int idx) {
    vec3 base = KEY_COLOR;
    float tail = clamp(g_tail_strength, 0.0, 1.0);

    if (idx == 0) {
        return applyDarkShade(base);
    }
    if (idx == 1) {
        return base;
    }
    if (idx == 2) {
        return applyLightShade(base);
    }
    if (idx == 3) {
        return base * tail;
    }

    return base;
}

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

vec3 sampleGradient(float t) {
    // Interpolate between gradient stops using the provided position.
    if (GRADIENT_SIZE <= 1) {
        return getGradientStop(0);
    }

    t = clamp(t, 0.0, 1.0);
    float scaled = t * float(GRADIENT_SIZE - 1);
    int idx = int(floor(scaled));
    int nextIdx = min(idx + 1, GRADIENT_SIZE - 1);
    float mixAmount = scaled - float(idx);
    vec3 current = getGradientStop(idx);
    vec3 next = getGradientStop(nextIdx);
    return mix(current, next, mixAmount);
}


vec3 applyVibrance(vec3 color, float vibrance) {
    if (abs(vibrance) < 1e-5) {
        return clamp(color, 0.0, 1.0);
    }

    float luma = dot(color, LUMA_WEIGHTS);
    vec3 chroma = color - vec3(luma);
    float saturation = length(chroma);
    float influence = 1.0 - clamp(saturation, 0.0, 1.0);
    float scale = max(0.0, 1.0 + vibrance * influence);
    vec3 adjusted = vec3(luma) + chroma * scale;
    return clamp(adjusted, 0.0, 1.0);
}


vec3 applyContrast(vec3 color, float contrast) {
    float safeContrast = max(contrast, 0.0);
    if (abs(safeContrast - 1.0) < 1e-5) {
        return clamp(color, 0.0, 1.0);
    }

    vec3 pivot = vec3(0.5);
    vec3 adjusted = (color - pivot) * safeContrast + pivot;
    return clamp(adjusted, 0.0, 1.0);
}


void mainImage(out vec4 out_color, vec2 fragCoord) {
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

    const float GRADIENT_WRAP_EPS = 1e-5;
    float basePosition = clamp(n, 0.0, 1.0 - GRADIENT_WRAP_EPS);
    float gradientShift = fract(iTime * max(u_gradient_speed, 0.0));
    float gradientPosition = basePosition;
    if (u_gradient_speed > 0.0) {
        gradientPosition = fract(basePosition + gradientShift);
    }
    vec3 final_color = sampleGradient(gradientPosition);
    final_color = applyVibrance(final_color, u_vibrance);
    final_color = applyContrast(final_color, u_contrast);

    out_color = vec4(final_color, 1.0);
}
