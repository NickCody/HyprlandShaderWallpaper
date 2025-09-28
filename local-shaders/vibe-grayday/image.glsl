// --- tweakables ------------------------------------------------------------

// Brightness pivot for contrast remap (0..1 range)
const float BASE_BRIGHTNESS = 0.1;

// Additive offset after contrast shaping
const float BRIGHTNESS_BIAS = -0.1;

// >1 increases contrast, <1 flattens
const float CONTRAST_GAIN   = 0.9;

// Gamma curve applied after remap; <1 lifts midtones
const float GAMMA_CURVE     = 0.8;

// Blend ratio between grayscale (0) and original color (1)
const float COLOR_MIX       = 0.0;

// 0 disables dithering, 1 ≈ ±2/255 amplitude jitter
const float DITHER_STRENGTH = 0.5;

const float FINAL_SCALE = 0.5;

// ---------------------------------------------------------------------------

// Cheap hash: decorrelated pseudo-random in [0,1)
float hash13(vec3 p)
{
    p = fract(p * 0.3183099 + vec3(0.1, 0.2, 0.3));
    p += dot(p, p.yzx + 19.19);
    return fract((p.x + p.y) * p.z);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    vec2 uv = fragCoord / iResolution.xy;

    // Original shader content
    vec3 col = 0.5 + 0.5 * cos(iTime + uv.xyx + vec3(0.0, 2.0, 4.0));

    // Perceptual grayscale
    float gray = dot(col, vec3(0.2126, 0.7152, 0.0722));

    // Contrast / brightness shaping around the pivot
    gray = (gray - BASE_BRIGHTNESS) * CONTRAST_GAIN + BASE_BRIGHTNESS;
    gray += BRIGHTNESS_BIAS;

    // Nonlinear curve, clamp to displayable range
    gray = pow(clamp(gray, 0.0, 1.0), GAMMA_CURVE);

    // Blend back some original color if desired
    vec3 penultimateColor = mix(vec3(gray), col, COLOR_MIX);

    // Spatial/temporal dithering to combat banding
    float noise = hash13(vec3(fragCoord, iTime));
    float amplitude = DITHER_STRENGTH * (2.0 / 255.0); // maps 1.0 → ~±2/255
    penultimateColor += (noise - 0.5) * amplitude;

    vec3 finalColor = FINAL_SCALE * penultimateColor;

    fragColor = vec4(clamp(finalColor, 0.0, 1.0), 1.0);
}
