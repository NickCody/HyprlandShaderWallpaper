
// --- tweakables ------------------------------------------------------------
// Uniform scale applied before folding into the equilateral triangle (use ~0.8 – 2.5 for gentle zoom; larger exaggerates repetition)
const float TRI_SCALE = 6.0;

// Global rotation of the triangle tessellation in revolutions per second (-1.0 – 1.0, negative spins clockwise)
const float TRI_ROTATION_SPEED = 0.033;

// Speed of the hue cycle inside the fundamental triangle (0.0 – 1.0 covers static to fast cycling)
const float SPECTRUM_SPEED = 0.01;

// Weighting applied to barycentric coordinates for petal shaping (1.0 – 6.0: higher = sharper petals)
const float PETAL_POWER = 60.0;

// Frequency of concentric wavelets emanating from the triangle centroid (3.0 – 14.0 controls ring density)
const float RADIAL_WAVE_FREQUENCY = 5.0;

// Speed of the concentric wavelets (0.1 – 2.5 adjusts pulse tempo)
const float RADIAL_WAVE_SPEED = 0.9;

// Blend between barycentric petals and radial wavelets (0.0 – 1.0, 0 = pure petals, 1 = pure waves)
const float WAVE_BLEND = 0.85;

// Amount of chroma variation contributed by each barycentric axis (0.0 – 1.0 per component; skews hue per triangle corner)
const vec3 CHROMA_AXIS_SWAY = vec3(0.12, 0.8, 0.24);

// Base saturation used when luma is minimal (0.0 – 1.0 sets minimum colorfulness)
const float BASE_SATURATION = 0.5;

// Additional saturation gained through petal highlights (0.0 – 1.0 boosts saturated accents)
const float SATURATION_GAIN = 0.1;

// Base brightness level inside the triangle (0.0 – 0.4 establishes background luminance)
const float BRIGHTNESS_FLOOR = 0.18;

// Additional brightness contributed by highlights (0.0 – 1.5 scales highlight peaks)
const float BRIGHTNESS_GAIN = 1.2;

// Strength of animated grain (0.0 – 0.05 applies subtle film noise)
const float NOISE_AMOUNT = 0.006;
// ---------------------------------------------------------------------------

const float PI = 3.14159265359;
const float TAU = 6.28318530718;

mat2 rotate(float a)
{
    float s = sin(a);
    float c = cos(a);
    return mat2(c, -s, s, c);
}

vec3 makeEdge(vec2 a, vec2 b)
{
    vec2 edge = b - a;
    vec2 normal = normalize(vec2(edge.y, -edge.x));
    float offset = -dot(normal, a);
    return vec3(normal, offset);
}

float hash21(vec2 p)
{
    p = fract(p * vec2(317.11, 183.97));
    p += dot(p, p.yx + 23.23);
    return fract(p.x * p.y);
}

vec3 barycentric(vec2 p, vec2 a, vec2 b, vec2 c)
{
    vec2 v0 = b - a;
    vec2 v1 = c - a;
    vec2 v2 = p - a;
    float d00 = dot(v0, v0);
    float d01 = dot(v0, v1);
    float d11 = dot(v1, v1);
    float d20 = dot(v2, v0);
    float d21 = dot(v2, v1);
    float denom = d00 * d11 - d01 * d01;
    float v = (d11 * d20 - d01 * d21) / denom;
    float w = (d00 * d21 - d01 * d20) / denom;
    float u = 1.0 - v - w;
    return vec3(u, v, w);
}

vec2 foldIntoTriangle(vec2 p)
{
    vec2 v0 = vec2(0.0, 0.57735026919);
    vec2 v1 = vec2(-0.5, -0.28867513459);
    vec2 v2 = vec2(0.5, -0.28867513459);

    vec3 edges[3];
    edges[0] = makeEdge(v1, v2);
    edges[1] = makeEdge(v2, v0);
    edges[2] = makeEdge(v0, v1);

    for (int iter = 0; iter < 10; ++iter)
    {
        float d0 = dot(edges[0].xy, p) + edges[0].z;
        float d1 = dot(edges[1].xy, p) + edges[1].z;
        float d2 = dot(edges[2].xy, p) + edges[2].z;
        if (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
            break;
        if (d0 > 0.0)
        {
            p -= 2.0 * d0 * edges[0].xy;
            continue;
        }
        if (d1 > 0.0)
        {
            p -= 2.0 * d1 * edges[1].xy;
            continue;
        }
        p -= 2.0 * d2 * edges[2].xy;
    }

    return p;
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

    centred = rotate(time * TAU * TRI_ROTATION_SPEED) * centred * TRI_SCALE;
    vec2 fundamental = foldIntoTriangle(centred);

    vec2 v0 = vec2(0.0, 0.57735026919);
    vec2 v1 = vec2(-0.5, -0.28867513459);
    vec2 v2 = vec2(0.5, -0.28867513459);

    vec3 bc = barycentric(fundamental, v0, v1, v2);
    bc = clamp(bc, 0.0, 1.0);

    float petal = pow(clamp(min(min(bc.x, bc.y), bc.z) * 3.0, 0.0, 1.0), PETAL_POWER);

    vec2 tri_center = (v0 + v1 + v2) / 3.0;
    float radius = length(fundamental - tri_center);
    float radialWave = 0.5 + 0.5 * cos(radius * RADIAL_WAVE_FREQUENCY - time * TAU * RADIAL_WAVE_SPEED);

    float wobble = sin(iTime * TAU * 0.5);       // -1 .. 1
    float highlight = mix(radialWave, petal, WAVE_BLEND * wobble);

    float hue = fract(time * SPECTRUM_SPEED
                      + dot(bc, CHROMA_AXIS_SWAY)
                      + radius * 0.1);
    float saturation = BASE_SATURATION + highlight * SATURATION_GAIN;
    float brightness = BRIGHTNESS_FLOOR + highlight * BRIGHTNESS_GAIN;

    vec3 rgb = hsv2rgb(vec3(hue, saturation, brightness));
    float grain = hash21(fundamental * 7.1 + time * 0.7) - 0.5;
    rgb += grain * NOISE_AMOUNT;
    rgb = clamp(rgb, 0.0, 1.0);

    fragColor = vec4(rgb, 1.0);
}
