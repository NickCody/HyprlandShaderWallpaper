
// --- tweakables ------------------------------------------------------------
// Number of mirrors in the dihedral kaleidoscope (360° / SEGMENT_COUNT)
const float SEGMENT_COUNT = 6.0;

// Global spin applied to the entire kaleidoscope pattern (revolutions per second)
const float ROTATION_SPEED = 0.1;

// Spin applied to the virtual object cell inside the wedge
const float OBJECT_ROT_SPEED = 0.35;

// Controls the sharpness of wedge-symmetric spikes
const float STAR_SHARPNESS = 3.5;

// Number of sub-spokes carved inside each wedge
const float STAR_FACETS = 4.0;

// Tightens or loosens the radial falloff toward the edge of the tube
const float RADIAL_FOCUS = 1.35;

// Exponent that shapes the radial envelope
const float RADIAL_POWER = 2.6;

// Hue displacement contributed by radius
const float RADIUS_CHROMA_SWAY = 0.45;

// Hue displacement contributed by wedge index
const float WEDGE_CHROMA_SWAY = 0.11;

// Speed of the rainbow cycle along the ring
const float SPECTRUM_SPEED = 0.18;

// Base brightness level of the darkest petals
const float BRIGHTNESS_FLOOR = 0.12;

// Additional brightness contributed by spikes and rings
const float BRIGHTNESS_GAIN = 1.15;

// Power applied to the circular vignette (higher = tighter falloff)
const float VIGNETTE_POWER = 2.4;

// Strength of animated grain
const float NOISE_AMOUNT = 0.0065;
// ---------------------------------------------------------------------------

const float PI = 3.14159265359;
const float TAU = 6.28318530718;

mat2 rotate(float a)
{
    float s = sin(a);
    float c = cos(a);
    return mat2(c, -s, s, c);
}

float hash21(vec2 p)
{
    p = fract(p * vec2(115.23, 236.53));
    p += dot(p, p.yx + 19.19);
    return fract(p.x * p.y);
}

float mirrorFold(float angle, float span)
{
    float wrapped = mod(angle, TAU);
    if (wrapped < 0.0)
        wrapped += TAU;
    float k = floor(wrapped / span);
    float u = wrapped - k * span;
    if (mod(k, 2.0) >= 1.0)
        return span - u;
    return u;
}

vec3 hsv2rgb(vec3 c)
{
    vec3 rgb = clamp(abs(mod(c.x * 6.0 + vec3(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0, 0.0, 1.0);
    rgb = rgb * rgb * (3.0 - 2.0 * rgb);
    return c.z * mix(vec3(1.0), rgb, c.y);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    vec2 centred = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float time = iTime;

    // Spin the whole kaleidoscope cylinder
    centred = rotate(time * TAU * ROTATION_SPEED) * centred;

    float radius = length(centred);
    float angle = atan(centred.y, centred.x);
    float span = TAU / max(SEGMENT_COUNT, 1.0);

    float sectorIndex = floor(mod(angle + TAU, TAU) / span);
    float foldedAngle = mirrorFold(angle, span);
    float wedgeU = foldedAngle / span; // [0, 1]

    // Virtual object-cell coordinates (unwrapped wedge space)
    float objectAngle = (wedgeU - 0.5) * TAU * STAR_FACETS;
    objectAngle += time * TAU * OBJECT_ROT_SPEED;
    float angularBloom = pow(clamp(0.5 + 0.5 * cos(objectAngle), 0.0, 1.0), STAR_SHARPNESS);

    float radialEnvelope = pow(clamp(1.0 - radius * RADIAL_FOCUS, 0.0, 1.0), RADIAL_POWER);
    float highlight = clamp(angularBloom * radialEnvelope, 0.0, 1.0);

    float hue = fract(time * SPECTRUM_SPEED
                      + radius * RADIUS_CHROMA_SWAY
                      + sectorIndex * WEDGE_CHROMA_SWAY / max(SEGMENT_COUNT, 1.0));
    float saturation = mix(0.45, 1.0, highlight);
    float brightness = BRIGHTNESS_FLOOR + highlight * BRIGHTNESS_GAIN;

    // Gentle vignette to mimic the circular field stop
    float vignette = pow(clamp(1.0 - radius, 0.0, 1.0), VIGNETTE_POWER);
    brightness *= vignette;

    vec3 rgb = hsv2rgb(vec3(hue, saturation, brightness));

    float grain = hash21(centred * 11.7 + time * 1.3) - 0.5;
    rgb += grain * NOISE_AMOUNT;
    rgb = clamp(rgb, 0.0, 1.0);

    fragColor = vec4(rgb, 1.0);
}
