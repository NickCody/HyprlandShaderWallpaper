void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    // Normalized pixel coordinates (from 0 to 1)
    vec2 uv = fragCoord/iResolution.xy;

    // Time varying pixel color
    vec3 col = 0.5 + 0.5*cos(iTime+uv.xyx+vec3(0,2,4));
    float gray = dot(col, vec3(0.2126, 0.7152, 0.0722)); // perceptual weighting

    // color
    fragColor = vec4(col,1.0);
}