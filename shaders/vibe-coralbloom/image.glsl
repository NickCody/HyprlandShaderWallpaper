
// --- tweakables ------------------------------------------------------------
// Primary deep tone used for the rotating bands
const vec3 COLOR_SHADOW = vec3(0.059, 0.196, 0.235);

// Secondary tint mixed into the mid band gradient
const vec3 COLOR_MID = vec3(0.255, 0.706, 0.667);

// Highlight accent applied during the pulse
const vec3 COLOR_HILITE = vec3(1, 0.784, 0.529);

// Blend ratio between base and highlight colours
const float BLOOM_BLEND = 0.48;

// Angular band count around the centre
const float STRIPE_FREQUENCY = 6.2;

// Controls how sharp the angular bands appear
const float STRIPE_CURVE = 1.1;

// Animation speed for the angular motion
const float TIME_SPEED = 0.6;

// Temporal speed of the radial pulse
const float PULSE_SPEED = 1.5;

// Spatial frequency for the radial pulse
const float PULSE_DEPTH = 10;

// How tightly the pulse hugs the centre
const float RADIAL_FOCUS = 2.5;

// Exponent shaping pulse falloff
const float PULSE_CURVE = 2.8;

// Saturation mix against luminance
const float SATURATION_WEIGHT = 1.1;

// Contrast multiplier around mid tones
const float CONTRAST_GAIN = 1.05;

// Post-contrast gamma curve
const float GAMMA_CURVE = 1;

// Vignette radius factor
const float VIGNETTE_RADIUS = 1.8;

// Controls softness of the vignette
const float VIGNETTE_POWER = 2.6;

// Blend amount for vignette application
const float VIGNETTE_MIX = 0.45;

// Strength of the animated grain
const float GRAIN_AMOUNT = 0.012;
// ---------------------------------------------------------------------------

float hash13(vec3 p)
{
    p = fract(p * 0.1031 + vec3(0.3, 0.7, 0.9));
    p += dot(p, p.yzx + 19.19);
    return fract((p.x + p.y) * p.z);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    vec2 uv = fragCoord / iResolution.xy;
    vec2 centered = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);

    float angle = atan(centered.y, centered.x);
    float radius = length(centered);

    float stripe = 0.5 + 0.5 * sin(angle * STRIPE_FREQUENCY + iTime * TIME_SPEED);
    stripe = pow(clamp(stripe, 0.0, 1.0), STRIPE_CURVE);

    float radial = exp(-radius * RADIAL_FOCUS);
    float pulse = 0.5 + 0.5 * sin(iTime * PULSE_SPEED + radius * PULSE_DEPTH);
    float pulseWeight = pow(clamp(pulse * radial, 0.0, 1.0), PULSE_CURVE);

    vec3 baseColor = mix(COLOR_SHADOW, COLOR_MID, stripe);
    vec3 highlightColor = mix(baseColor, COLOR_HILITE, pulseWeight);
    vec3 color = mix(baseColor, highlightColor, BLOOM_BLEND);

    float luma = dot(color, vec3(0.299, 0.587, 0.114));
    color = mix(vec3(luma), color, SATURATION_WEIGHT);

    color = (color - 0.5) * CONTRAST_GAIN + 0.5;
    color = pow(clamp(color, 0.0, 1.0), vec3(GAMMA_CURVE));

    float vignette = pow(clamp(1.0 - radius * VIGNETTE_RADIUS, 0.0, 1.0), VIGNETTE_POWER);
    color = mix(color, color * vignette, VIGNETTE_MIX);

    float grain = hash13(vec3(fragCoord.xy, iTime * 37.0)) - 0.5;
    color += grain * GRAIN_AMOUNT;

    fragColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
