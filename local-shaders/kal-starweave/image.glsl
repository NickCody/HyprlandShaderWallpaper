
// --- tweakables ------------------------------------------------------------
// Uniform scale applied before folding into the 45-45-90 triangle
const float TRI_SCALE = 1.45;

// Global rotation rate for the square tessellation (revolutions per second)
const float LATTICE_ROT_SPEED = -0.05;

// Controls the density of the reflected square lattice
const float GRID_SCALE = 4.5;

// Sharpness of the diamond centre glow
const float DIAMOND_SHARPNESS = 3.2;

// Sharpness of the diagonal rhombus bands
const float RHOMBUS_SHARPNESS = 2.7;

// Number of polar spokes threading through the pattern
const float SPOKE_COUNT = 16.0;

// Speed of the spoke oscillation
const float SPOKE_SPEED = 0.85;

// Hue cycling speed for the entire tessellation
const float SPECTRUM_SPEED = 0.24;

// Hue offset contributed by the lattice geometry
const float GEOMETRY_CHROMA_SWAY = 0.22;

// Base saturation level
const float BASE_SATURATION = 0.58;

// Additional saturation derived from highlights
const float SATURATION_GAIN = 0.4;

// Darkest brightness level in the pattern
const float BRIGHTNESS_FLOOR = 0.15;

// Additional brightness from combined highlights
const float BRIGHTNESS_GAIN = 1.05;

// Strength of animated grain
const float NOISE_AMOUNT = 0.007;
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
    p = fract(p * vec2(227.41, 389.17));
    p += dot(p, p.yx + 11.11);
    return fract(p.x * p.y);
}

vec2 foldIntoRightTriangle(vec2 p)
{
    vec2 v0 = vec2(-0.5, -0.5);
    vec2 v1 = vec2(0.5, -0.5);
    vec2 v2 = vec2(-0.5, 0.5);

    vec3 edges[3];
    edges[0] = makeEdge(v0, v1);
    edges[1] = makeEdge(v1, v2);
    edges[2] = makeEdge(v2, v0);

    for (int iter = 0; iter < 12; ++iter)
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

    vec2 spun = rotate(time * TAU * LATTICE_ROT_SPEED) * centred * TRI_SCALE;
    vec2 fundamental = foldIntoRightTriangle(spun);

    // Lattice geometry derived from reflected coordinates
    vec2 tile = fundamental * GRID_SCALE;
    vec2 cell = abs(fract(tile) - 0.5);
    float diamond = pow(clamp(0.5 - max(cell.x, cell.y), 0.0, 1.0), DIAMOND_SHARPNESS);

    float diagonal = abs(fract((tile.x + tile.y) * 0.5) - 0.5);
    float rhombus = pow(clamp(0.5 - diagonal, 0.0, 1.0), RHOMBUS_SHARPNESS);

    float geometry = clamp(diamond + rhombus * 0.9, 0.0, 1.0);

    float angle = atan(centred.y, centred.x);
    float spokes = pow(clamp(0.5 + 0.5 * cos(angle * SPOKE_COUNT + time * TAU * SPOKE_SPEED), 0.0, 1.0), 2.2);

    float highlight = clamp(geometry + spokes * 0.6, 0.0, 1.0);

    float hue = fract(time * SPECTRUM_SPEED + geometry * GEOMETRY_CHROMA_SWAY + spokes * 0.12);
    float saturation = BASE_SATURATION + highlight * SATURATION_GAIN;
    float brightness = BRIGHTNESS_FLOOR + highlight * BRIGHTNESS_GAIN;

    vec3 rgb = hsv2rgb(vec3(hue, saturation, brightness));
    float grain = hash21(fundamental * 9.7 + time) - 0.5;
    rgb += grain * NOISE_AMOUNT;
    rgb = clamp(rgb, 0.0, 1.0);

    fragColor = vec4(rgb, 1.0);
}
