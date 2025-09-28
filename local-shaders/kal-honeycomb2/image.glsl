// kal-honeycomn — fixed: re-fold after parallax; no pre-clamp of barycentrics

const float TRI_SCALE          = 5.0;
const float TRI_ROT_SPEED      = 0.05;
const float ALIGN_EXTRA_ROT    = 0.0;

const int   CELL_DIVS          = 8;

const float OBJECT_THICKNESS   = 0.0;
const float OBJECT_DRIFT_SPEED = 0.40;

const float MIRROR_WIDTH       = 0.020;
const vec3  MIRROR_ALIGN_BIAS  = vec3(0.0);
const vec3  MIRROR_COLOR       = vec3(0.08);

const float APERTURE_RADIUS    = 0.90;
const float APERTURE_BORDER    = 0.012;
const vec3  APERTURE_BORDER_COL= vec3(0.0);

#define TAU 6.28318530717958647692

mat2 rot2(float a){ float s=sin(a), c=cos(a); return mat2(c,-s,s,c); }
vec2 perp(vec2 v){ return vec2(-v.y, v.x); }

struct Edge{ vec2 n; float o; };
Edge makeEdge(vec2 a, vec2 b){ vec2 e=b-a; vec2 n=normalize(perp(e)); return Edge(n, -dot(n,a)); }
float sdEdge(Edge E, vec2 p){ return dot(E.n,p)+E.o; }
vec2 reflectAcross(Edge E, vec2 p){ float d=sdEdge(E,p); return p-2.0*d*E.n; }

vec3 barycentric(vec2 p, vec2 a, vec2 b, vec2 c){
    vec2 v0=b-a, v1=c-a, v2=p-a;
    float d00=dot(v0,v0), d01=dot(v0,v1), d11=dot(v1,v1);
    float d20=dot(v2,v0), d21=dot(v2,v1);
    float D=d00*d11-d01*d01;
    float v=(d11*d20-d01*d21)/D;
    float w=(d00*d21-d01*d20)/D;
    return vec3(1.0-v-w, v, w);
}

float hash11(float x){ x=fract(x*0.1031); x*=x+33.33; x*=x+x; return fract(x); }

vec3 palette(int idx){
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

void triVertices(out vec2 A, out vec2 B, out vec2 C){
    float r_in = inversesqrt(3.0);
    A = vec2(0.0,  r_in);
    B = vec2(-0.5,-0.5*r_in);
    C = vec2( 0.5,-0.5*r_in);
}
void triEdges(in vec2 A, in vec2 B, in vec2 C, out Edge E0,out Edge E1,out Edge E2){
    E0=makeEdge(B,C); E1=makeEdge(C,A); E2=makeEdge(A,B);
}

vec2 foldIntoEquilateral(vec2 p, Edge E0, Edge E1, Edge E2, out vec3 finalSD){
    for(int i=0;i<12;++i){
        float d0=sdEdge(E0,p), d1=sdEdge(E1,p), d2=sdEdge(E2,p);
        if(d0<=0.0 && d1<=0.0 && d2<=0.0){ finalSD=vec3(d0,d1,d2); break; }
        if(d0>0.0){ p=reflectAcross(E0,p); continue; }
        if(d1>0.0){ p=reflectAcross(E1,p); continue; }
        p=reflectAcross(E2,p);
    }
    finalSD=vec3(sdEdge(E0,p), sdEdge(E1,p), sdEdge(E2,p));
    return p;
}

// Subdivision: map barycentrics to a triangular grid index (no clamp needed)
struct CellInfo{ ivec3 ijk; int orient; };
CellInfo cellFromBary(vec3 bc){
    float N = float(CELL_DIVS);
    // Ensure we stay on the bc.x+bc.y+bc.z=1 plane numerically
    bc /= (bc.x+bc.y+bc.z);
    vec2 uv = bc.xy * N;
    vec2 f  = floor(uv);
    vec2 fr = fract(uv);
    int i = int(f.x), j = int(f.y), orient=0;
    if(fr.x + fr.y > 1.0){ i+=1; j+=1; orient=1; }
    int k = int(N)-1 - i - j;
    i = clamp(i,0,CELL_DIVS-1);
    j = clamp(j,0,CELL_DIVS-1);
    k = clamp(k,0,CELL_DIVS-1);
    CellInfo info; info.ijk=ivec3(i,j,k); info.orient=orient; return info;
}
vec3 cellColor(CellInfo ci){
    int base = (ci.ijk.x*131 + ci.ijk.y*197 + ci.ijk.z*233 + ci.orient*17) & 15;
    return palette(base);
}
float cellEdgeMask(vec3 bc, float lineWidth){
    bc /= (bc.x+bc.y+bc.z);
    vec3 s = bc * float(CELL_DIVS);
    vec3 f = fract(s);
    float d = min(min(min(f.x,1.0-f.x), min(f.y,1.0-f.y)), min(f.z,1.0-f.z));
    return step(d, lineWidth);
}

vec2 objectParallax(vec2 pObj, vec2 triCentroid, vec3 bc){
    bc /= (bc.x+bc.y+bc.z);
    float depth = dot(bc, vec3(0.2,0.5,0.8));
    float ang   = TAU * OBJECT_DRIFT_SPEED * iTime * OBJECT_THICKNESS * (depth - 0.5);
    float scale = 1.0 + 0.08 * OBJECT_THICKNESS * (depth - 0.5);
    vec2 d = pObj - triCentroid;
    d = rot2(ang) * d * scale;
    return triCentroid + d;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord){
    vec2 R = iResolution.xy;
    vec2 uv = (fragCoord - 0.5*R) / R.y;

    float r = length(uv);
    float inside = step(r, APERTURE_RADIUS);

    float t = iTime;
    float spin = t * TAU * TRI_ROT_SPEED + ALIGN_EXTRA_ROT;
    vec2 p = rot2(spin) * (uv * TRI_SCALE);

    vec2 A,B,C; triVertices(A,B,C);
    Edge E0,E1,E2; triEdges(A,B,C,E0,E1,E2);

    vec3 sd;
    vec2 pf = foldIntoEquilateral(p, E0,E1,E2, sd);

    vec2 Tcent = (A+B+C)/3.0;

    // ★ Compute barycentrics BEFORE parallax (for depth), but:
    vec3 bc = barycentric(pf, A,B,C);

    // ★ Apply parallax to OBJECT, then RE-FOLD to keep it inside the triangle
    vec2 pObj  = objectParallax(pf, Tcent, bc);
    vec3 sd2;  // not used for seams
    vec2 pObjF = foldIntoEquilateral(pObj, E0,E1,E2, sd2); // ★ crucial

    // ★ Use barycentrics of the re-folded point (no clamp)
    vec3 bcObj = barycentric(pObjF, A,B,C);

    CellInfo ci = cellFromBary(bcObj);
    vec3 col = cellColor(ci);

    // crisp internal sub-triangle edges
    float subLines = cellEdgeMask(bcObj, 0.006);
    col = mix(col, vec3(0.0), subLines);

    // Mirror seams from the *mirror* fold (sd), not the object fold
    vec3 edgeIn = -sd + MIRROR_ALIGN_BIAS;
    float edgeDist = max(0.0, min(edgeIn.x, min(edgeIn.y, edgeIn.z)));
    float seam = 1.0 - smoothstep(0.0, MIRROR_WIDTH, edgeDist);
    col = mix(col, MIRROR_COLOR, seam);

    if(inside < 0.5){ fragColor = vec4(0.0,0.0,0.0,1.0); return; }

    float ring = step(APERTURE_RADIUS - APERTURE_BORDER, r) * step(r, APERTURE_RADIUS);
    col = mix(col, APERTURE_BORDER_COL, ring);

    fragColor = vec4(col,1.0);
}