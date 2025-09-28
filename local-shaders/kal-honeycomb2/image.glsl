// kal-honeycomn — three-mirror (60-60-60) kaleidoscope with honeycomb tessellation
// p6m symmetry via triangle reflection folding
// Author: you + GPT-5 Pro (Shadertoy compatible: uses iTime, iResolution)

// ----------------- TWEAKABLES ----------------------------------------------
const float TRI_SCALE          = 5.0;     // zoom of the tiling (larger => denser repeats)
const float TRI_ROT_SPEED      = 0.05;    // revolutions per second of the tessellation
const float ALIGN_EXTRA_ROT    = 0.0;     // extra rotation (radians) to “align” mirrors

// Subdivision of the fundamental triangle into crisp colored micro-triangles
const int   CELL_DIVS          = 8;       // number of subdivisions per edge (≥ 2)

// Object cell “thickness”: amplitude of parallax-like layer motion (0 = static)
const float OBJECT_THICKNESS   = 0.20;    // ~0.0 – 0.5 is subtle, keep small to stay sharp
const float OBJECT_DRIFT_SPEED = 0.40;    // speed of that micro-motion (Hz)

// Mirror seam control
const float MIRROR_WIDTH       = 0.010;   // seam thickness inside each reflected triangle (scene units)
const vec3  MIRROR_ALIGN_BIAS  = vec3(0.0); // per-edge inward(+)/outward(-) bias of seam placement
const vec3  MIRROR_COLOR       = vec3(0.08); // seam color (dark gray)

// Aperture (hard field stop)
const float APERTURE_RADIUS    = 0.90;    // radius in normalized screen units (relative to min dimension)
const float APERTURE_BORDER    = 0.012;   // ring thickness at the aperture edge (0 = none)
const vec3  APERTURE_BORDER_COL= vec3(0.0); // ring color
// ---------------------------------------------------------------------------

#define PI 3.14159265358979323846
#define TAU 6.28318530717958647692

// ----------------- MATH HELPERS --------------------------------------------
mat2 rot2(float a){
    float s = sin(a), c = cos(a);
    return mat2(c,-s,s,c);
}

vec2 perp(vec2 v){ return vec2(-v.y, v.x); }

// Edge represented as: dot(n, x) + o = 0, with outward unit normal n
struct Edge{ vec2 n; float o; };

Edge makeEdge(vec2 a, vec2 b){ // edge from a->b, outward normal points outside the triangle
    vec2 e = b - a;
    vec2 n = normalize(perp(e));  // rotate -90°
    float o = -dot(n, a);
    return Edge(n, o);
}

// Signed distance to edge: positive outside, negative inside (since outward normal)
float sdEdge(Edge E, vec2 p){ return dot(E.n, p) + E.o; }

// Reflect point p across edge E
vec2 reflectAcross(Edge E, vec2 p){
    float d = sdEdge(E, p);
    return p - 2.0*d*E.n;
}

// Barycentric coordinates of p w.r.t. triangle (a,b,c)
vec3 barycentric(vec2 p, vec2 a, vec2 b, vec2 c){
    vec2 v0 = b - a, v1 = c - a, v2 = p - a;
    float d00 = dot(v0,v0);
    float d01 = dot(v0,v1);
    float d11 = dot(v1,v1);
    float d20 = dot(v2,v0);
    float d21 = dot(v2,v1);
    float denom = d00*d11 - d01*d01;
    float v = (d11*d20 - d01*d21) / denom;
    float w = (d00*d21 - d01*d20) / denom;
    float u = 1.0 - v - w;
    return vec3(u,v,w);
}

// Simple integerish hash to choose palette colors deterministically
float hash11(float x){
    x = fract(x * 0.1031);
    x *= x + 33.33;
    x *= x + x;
    return fract(x);
}
float hash21(vec2 p){
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Discrete palette (no gradients). 16 bright-ish colors.
vec3 palette(int idx){
    // sRGB-ish constants
    if(idx==0)  return vec3(0.90,0.18,0.23);
    if(idx==1)  return vec3(0.98,0.64,0.07);
    if(idx==2)  return vec3(0.99,0.88,0.06);
    if(idx==3)  return vec3(0.00,0.62,0.38);
    if(idx==4)  return vec3(0.09,0.48,0.82);
    if(idx==5)  return vec3(0.36,0.17,0.78);
    if(idx==6)  return vec3(0.97,0.32,0.54);
    if(idx==7)  return vec3(0.51,0.77,0.88);
    if(idx==8)  return vec3(0.64,0.83,0.33);
    if(idx==9)  return vec3(0.84,0.45,0.12);
    if(idx==10) return vec3(0.65,0.20,0.18);
    if(idx==11) return vec3(0.20,0.67,0.63);
    if(idx==12) return vec3(0.74,0.55,0.90);
    if(idx==13) return vec3(0.95,0.58,0.77);
    if(idx==14) return vec3(0.84,0.82,0.79);
    return               vec3(0.23,0.23,0.23);
}

// ----------------- EQUILATERAL TRIANGLE SETUP ------------------------------
// Build an equilateral triangle centered at the origin.
// Coordinates are derived from inradius r_in = 1/sqrt(3).
void triVertices(out vec2 A, out vec2 B, out vec2 C, float extraRotation){
    float r_in = inversesqrt(3.0);          // ~0.5773502691896258
    // Unrotated, one vertex up:
    vec2 vA = vec2(0.0,  r_in);
    vec2 vB = vec2(-0.5, -0.5*r_in);
    vec2 vC = vec2( 0.5, -0.5*r_in);
    mat2 R  = rot2(extraRotation);
    A = R * vA; B = R * vB; C = R * vC;
}

void triEdges(in vec2 A, in vec2 B, in vec2 C, out Edge E0, out Edge E1, out Edge E2){
    // Edges opposite A,B,C respectively
    E0 = makeEdge(B, C);
    E1 = makeEdge(C, A);
    E2 = makeEdge(A, B);
}

// ----------------- FOLD INTO TRIANGLE (triangle reflection group) ----------
// Returns point folded into the triangle and the final signed distances to edges.
vec2 foldIntoEquilateral(vec2 p, Edge E0, Edge E1, Edge E2, out vec3 finalSD){
    // Iterate reflections until inside. 10 is plenty for our viewport.
    for(int i=0; i<10; ++i){
        float d0 = sdEdge(E0, p);
        float d1 = sdEdge(E1, p);
        float d2 = sdEdge(E2, p);
        if(d0<=0.0 && d1<=0.0 && d2<=0.0){ finalSD = vec3(d0,d1,d2); break; }
        // Reflect across any offending edge (choose one each loop)
        if(d0>0.0){ p = reflectAcross(E0, p); continue; }
        if(d1>0.0){ p = reflectAcross(E1, p); continue; }
        p = reflectAcross(E2, p);
    }
    // Compute final SD after loop (in case we exited early)
    float d0 = sdEdge(E0, p);
    float d1 = sdEdge(E1, p);
    float d2 = sdEdge(E2, p);
    finalSD = vec3(d0,d1,d2);
    return p;
}

// ----------------- TRIANGULAR SUBDIVISION (CRISP COLOR REGIONS) ------------
// Partition the *object cell* (fundamental triangle) into CELL_DIVS subdivisions
// per edge. We use the standard “square split into two triangles” trick on the
// (bc.x, bc.y) plane: orientation decided by fract sums.
struct CellInfo{ ivec3 ijk; int orient; }; // i+j+k = CELL_DIVS-1

CellInfo cellFromBary(vec3 bc){
    float N = float(CELL_DIVS);
    vec2 uv = bc.xy * N;
    vec2 f  = floor(uv);
    vec2 fr = fract(uv);

    int i = int(f.x);
    int j = int(f.y);

    int orient = 0; // 0 = lower-left small triangle, 1 = upper-right small triangle
    if(fr.x + fr.y > 1.0){ i += 1; j += 1; orient = 1; }

    int k = int(N) - 1 - i - j;
    // clamp for numerical edge cases
    i = max(0, min(i, CELL_DIVS-1));
    j = max(0, min(j, CELL_DIVS-1));
    k = max(0, min(k, CELL_DIVS-1));

    CellInfo info; info.ijk = ivec3(i,j,k); info.orient = orient;
    return info;
}

// Per-cell solid color (no gradients). Deterministic from (i,j,k,orient)
vec3 cellColor(CellInfo ci){
    // Mix indices into a single id and pick from palette.
    int base = (ci.ijk.x*131 + ci.ijk.y*197 + ci.ijk.z*233 + ci.orient*17) & 15;
    return palette(base);
}

// Optional: draw crisp internal lines of the triangular subdivision (0/1 mask)
float cellEdgeMask(vec3 bc, float lineWidth){
    // Distances to sub-division lines: bc scaled by CELL_DIVS
    vec3 s = bc * float(CELL_DIVS);
    vec3 fracp = abs(fract(s) - 0.0);
    // Being near a line means one of the three fractional parts is near 0
    float d = min(fracp.x, min(fracp.y, fracp.z));
    return step(d, lineWidth);
}

// Micro motion of the object cell content to simulate thickness (parallax-like)
vec2 objectParallax(vec2 pObj, vec2 triCentroid, vec3 bc){
    // Depth as a linear combo of barycentric (each corner acts like a different depth)
    float depth = dot(bc, vec3(0.2, 0.5, 0.8)); // asymmetric depth map
    float ang   = TAU * OBJECT_DRIFT_SPEED * iTime * OBJECT_THICKNESS * (depth - 0.5);
    float scale = 1.0 + 0.08 * OBJECT_THICKNESS * (depth - 0.5);
    vec2 d = pObj - triCentroid;
    d = rot2(ang) * d * scale;
    return triCentroid + d;
}

// ----------------- MAIN -----------------------------------------------------
void mainImage(out vec4 fragColor, in vec2 fragCoord){
    // Normalized coordinates with uniform scale (preserve aspect)
    vec2 R = iResolution.xy;
    vec2 uv = (fragCoord - 0.5*R) / R.y; // center at 0, scale by height

    // Aperture mask (hard-edged circle)
    float r = length(uv);
    float inside = step(r, APERTURE_RADIUS);

    // Global rotation / scale of the tessellation
    float t = iTime;
    float spin = t * TAU * TRI_ROT_SPEED + ALIGN_EXTRA_ROT;
    vec2 p = rot2(spin) * (uv * TRI_SCALE);

    // Build equilateral triangle and its edges
    vec2 A,B,C;
    triVertices(A,B,C, 0.0); // we already rotated world by 'spin'
    Edge E0,E1,E2;
    triEdges(A,B,C, E0,E1,E2);

    // Fold world point into the fundamental triangle (p6m symmetry)
    vec3 sd;
    vec2 pf = foldIntoEquilateral(p, E0,E1,E2, sd);

    // Triangle centroid
    vec2 Tcent = (A + B + C) / 3.0;

    // Barycentric coordinates inside the folded triangle
    vec3 bc = barycentric(pf, A,B,C);
    bc = clamp(bc, 0.0, 1.0); // numeric safety

    // Parallax-like object thickness motion (moves *content*, not the mirror seams)
    vec2 pObj = objectParallax(pf, Tcent, bc);
    vec3 bcObj = barycentric(pObj, A,B,C);
    bcObj = clamp(bcObj, 0.0, 1.0);

    // Choose a crisp color region by subdividing the triangle into small triangles
    CellInfo ci = cellFromBary(bcObj);
    vec3 col = cellColor(ci);

    // Optional internal cell grid lines (crisp). Keep tiny.
    float subLines = cellEdgeMask(bcObj, 0.012);
    col = mix(col, vec3(0.0), subLines); // black subdivision lines

    // Mirror seam overlay (distance to actual triangle edges, with width & alignment bias)
    // Inside the folded triangle, sd <= 0. Closeness to edge = -sd (small near edge).
    vec3 edgeIn = -sd + MIRROR_ALIGN_BIAS; // apply bias per-edge
    float seam = step(min(edgeIn.x, min(edgeIn.y, edgeIn.z)), MIRROR_WIDTH);
    col = mix(col, MIRROR_COLOR, seam);

    // Aperture hard stop + border ring
    if(inside < 0.5){
        // Outside aperture -> black
        fragColor = vec4(0.0,0.0,0.0,1.0);
        return;
    }
    // Border ring (hard)
    float ring = step(APERTURE_RADIUS - APERTURE_BORDER, r) * step(r, APERTURE_RADIUS);
    col = mix(col, APERTURE_BORDER_COL, ring);

    fragColor = vec4(col, 1.0);
}
