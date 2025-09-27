float u_rotated_scale        = 0.05;
float u_primary_scale        = 0.01;
float u_rot_left_timescale   = 0.1;
float u_rot_right_timescale  = 0.2;
float u_timescale            = 0.1;
int   u_showComponents       = 0;

// Voronoise
// https://iquilezles.org/articles/voronoise
// by inigo quilez
//
vec3 hash3( vec2 p ){
    vec3 q = vec3( dot(p,vec2(127.1,311.7)),
    dot(p,vec2(269.5,183.3)),
    dot(p,vec2(419.2,371.9)) );
    return fract(sin(q)*43758.5453);
}

float iqnoise( in vec2 x, float u, float v ){
    vec2 p = floor(x);
    vec2 f = fract(x);

    float k = 1.0+63.0*pow(1.0-v,4.0);

    float va = 0.0;
    float wt = 0.0;
    for( int j=-2; j<=2; j++ )
    for( int i=-2; i<=2; i++ )
    {
        vec2 g = vec2( float(i),float(j) );
        vec3 o = hash3( p + g )*vec3(u,u,1.0);
        vec2 r = g - f + o.xy;
        float d = dot(r,r);
        float ww = pow( 1.0-smoothstep(0.0,1.414,sqrt(d)), k );
        va += o.z*ww;
        wt += ww;
    }

    return va/wt;
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

void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    vec3 white = vec3(1.0, 1.0, 1.0);
    vec3 black = vec3(0.0, 0.0, 0.0);

    vec2 rotated_resolution = iResolution.xy * u_rotated_scale;
    vec2 primary_resolution = iResolution.xy * u_primary_scale;

    vec2 rotated_fragCoord = fragCoord.xy * u_rotated_scale;
    vec2 primary_fragCoord = fragCoord.xy * u_primary_scale;

    vec2 left_rotated_center = rotated_resolution.xy/4.0;
    vec2 right_rotated_center = 3.0 * rotated_resolution.xy/4.0;
    vec2 primary_center = primary_resolution.xy/2.0;

    float time3d     = iTime * u_timescale;
    float timeLeft   = iTime * u_rot_left_timescale;
    float timeRight  = iTime * u_rot_right_timescale;

    vec2 coord0 = vec2( rotateOrigin(primary_fragCoord, primary_center,time3d));
    vec2 coord1 = vec2( rotateOrigin(rotated_fragCoord, left_rotated_center, timeLeft));
    vec2 coord2 = vec2( rotateOrigin(rotated_fragCoord, right_rotated_center, timeRight));

    vec2 uv = iMouse.xy/iResolution.xy;
    float n0 = iqnoise(coord0, uv.x, uv.y);
    float n1 = iqnoise(coord1, uv.x, uv.y);
    float n2 = iqnoise(coord2, uv.x, uv.y);

    vec3 color;

    if (u_showComponents == 0) {
        float brighten = 1.5;
        float c = (n1+n2)/2.0;
        float n = iqnoise(coord0 * c, 0.0, 1.0);

        vec3 col = 0.5 + 0.5*cos(iTime+coord0.xyx+vec3(0,2,4));

        color = mix(vec3(n, n, n), col, c);
    } else {
        color = vec3(n0, n1, n2);
    }

    fragColor = vec4(color, 1.0);
}
