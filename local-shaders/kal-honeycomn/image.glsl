
// --- tweakables ------------------------------------------------------------
// Uniform scale before folding into the equilateral mirror triangle (0.8 – 2.5)
const float TRI_SCALE = 4;

// Global rotation rate of the mirror system in revolutions per second (-1.0 – 1.0)
const float TRI_ROTATION_SPEED = 0.08;

// Extra angular offset for mirror alignment (radians, -1.0 – 1.0)
const float MIRROR_ALIGNMENT_OFFSET = 0.8;

// Width of the mirror seam accent inside the triangle (0.0 – 0.2)
const float MIRROR_EDGE_WIDTH = 0.14;

// Aperture radius controlling visible field (0.3 – 1.0)
const float APERTURE_RADIUS = 1.0;

// Feather width for the aperture mask (0.0 – 0.1; set very low for crisp edge)
const float APERTURE_FEATHER = 0.00;

// Simulated object-cell thickness driving parallax offset (0.0 – 0.3)
const float OBJECT_CELL_THICKNESS = 0.12;

// Speed applied to the object cell parallax wobble (0.0 – 3.0)
const float OBJECT_CELL_SWAY_SPEED = 1.15;

// Rate for the snap-to-cell color permutation (0.0 – 2.0)
const float PALETTE_ROTATION_SPEED = 0.1;

// Hex lattice scale controlling honeycomb density (2.0 – 8.0)
const float HONEYCOMB_SCALE = 9.2;

// Strength of binary petal mask (0.5 – 3.0)
const float PETAL_SHARPNESS = 0.01;

// Strength of the mirror seam glow (0.0 – 2.0)
const float MIRROR_GLOW = 1.8;

// Strength of animated film grain (0.0 – 0.03)
const float NOISE_AMOUNT = 0.000;
// ---------------------------------------------------------------------------

const float PI = 3.14159265359;
const float TAU = 6.28318530718;
const float SQRT3 = 1.73205080757;

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
    p = fract(p * vec2(417.73, 289.97));
    p += dot(p, p.yx + 17.17);
    return fract(p.x * p.y);
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

vec3 palette(int index)
{
    const vec3 colors[6] = vec3[](
        vec3(0.968, 0.741, 0.231), // golden honey
        vec3(0.956, 0.435, 0.196), // amber flame
        vec3(0.286, 0.722, 0.812), // lagoon teal
        vec3(0.941, 0.282, 0.525), // magenta bloom
        vec3(0.368, 0.874, 0.419), // verdant leaf
        vec3(0.933, 0.933, 0.933)  // porcelain white
    );
    return colors[clamp(index, 0, 5)];
}

int quantise_hex(vec2 p)
{
    vec2 basis_q = vec2(1.0, 0.0);
    vec2 basis_r = vec2(0.5, 0.86602540378);
    vec2 axial;
    axial.x = dot(p, basis_q);
    axial.y = dot(p, basis_r);
    vec2 cell = floor(axial + 0.5);
    float hue_index = mod(cell.x - cell.y, 6.0);
    return int(hue_index);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    vec2 centred = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float time = iTime;

    float radius = length(centred);
    float aperture = smoothstep(
        max(0.0, APERTURE_RADIUS - APERTURE_FEATHER),
        APERTURE_RADIUS,
        radius
    );

    vec2 alignment = rotate(time * TAU * TRI_ROTATION_SPEED + MIRROR_ALIGNMENT_OFFSET) * centred;
    vec2 spun = alignment * TRI_SCALE;

    vec2 parallax = spun;
    float cell_thickness = OBJECT_CELL_THICKNESS * sin(time * OBJECT_CELL_SWAY_SPEED + dot(spun, vec2(1.2, -0.8)));
    parallax += vec2(0.0, cell_thickness);

    vec2 fundamental = foldIntoTriangle(parallax);

    vec2 v0 = vec2(0.0, 0.57735026919);
    vec2 v1 = vec2(-0.5, -0.28867513459);
    vec2 v2 = vec2(0.5, -0.28867513459);

    vec3 bc = barycentric(fundamental, v0, v1, v2);
    float edge_distance = min(min(bc.x, bc.y), bc.z);

    float seam = 1.0 - smoothstep(0.0, MIRROR_EDGE_WIDTH * 0.5, edge_distance);
    seam = pow(seam, 3.0) * MIRROR_GLOW;

    vec2 honey = alignment * HONEYCOMB_SCALE;
    int base_index = quantise_hex(honey);
    int palette_offset = int(floor(time * PALETTE_ROTATION_SPEED)) % 6;
    if (palette_offset < 0) {
        palette_offset += 6;
    }
    int color_index = (base_index + palette_offset) % 6;

    float angle = atan(fundamental.y, fundamental.x);
    float petal = step(0.0, cos(angle * 6.0));
    float petal_mask = pow(petal * step(0.12, edge_distance), PETAL_SHARPNESS);

    vec3 base_color = palette(color_index);
    vec3 seam_color = palette((color_index + 3) % 6);
    vec3 final_color = mix(base_color, seam_color, clamp(seam, 0.0, 1.0));

    final_color = mix(final_color, palette((color_index + 2) % 6), petal_mask);

    final_color *= 1.0 - aperture;

    float grain = hash21(fundamental * 9.1 + time * 1.7) - 0.5;
    final_color += grain * NOISE_AMOUNT;

    fragColor = vec4(clamp(final_color, 0.0, 1.0), 1.0);
}
