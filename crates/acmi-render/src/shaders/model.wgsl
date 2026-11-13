// Instanced object models in one colour each, shaded to show their shape.

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct InstanceIn {
    // Model matrix columns; the translation is already eye-relative.
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    // sRGB colour.
    @location(6) color: vec4<f32>,
};

struct ModelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    // From the eye to the fragment.
    @location(2) view: vec3<f32>,
};

@vertex
fn vs_model(vertex: VertexIn, instance: InstanceIn) -> ModelOut {
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    var out: ModelOut;
    let relative = model * vec4<f32>(vertex.position, 1.0);
    out.clip = globals.view_proj * relative;
    out.view = relative.xyz;
    out.normal = (model * vec4<f32>(vertex.normal, 0.0)).xyz;
    out.color = vec4<f32>(srgb_to_linear(instance.color.rgb), instance.color.a);
    return out;
}

@fragment
fn fs_model(in: ModelOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let to_eye = normalize(-in.view);
    // Two-sided lighting: many models are open or single-sided shells.
    let diffuse = abs(dot(normal, globals.sun.xyz));
    // Objects stay readable on the night side.
    let ambient = max(globals.sun.w, 0.45);
    let sun = mix(1.0, ambient + (1.0 - ambient) * diffuse, globals.params.y);
    // A light at the eye: surfaces turned away from the viewer darken, which
    // is what separates wings, fuselage and tail in a single-coloured model.
    let facing = abs(dot(normal, to_eye));
    let shape = 0.35 + 0.65 * facing;
    // A soft highlight where the sun reflects towards the viewer.
    let gloss = pow(max(dot(normal, normalize(globals.sun.xyz + to_eye)), 0.0), 24.0) * 0.25 * globals.params.y;
    return vec4<f32>(in.color.rgb * sun * shape + vec3<f32>(gloss), in.color.a);
}
