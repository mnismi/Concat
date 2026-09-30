struct Params { amount: f32 }

// A broken digital signal: the picture cut into horizontal slices, some of
// them torn sideways by their own amount, with red and blue pulled apart
// along the tear. The slices re-deal twelve times a second, so it flickers
// the way a bad feed does; at zero amount the picture comes through whole.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let k = params.amount * 0.01;
    let slice = floor(uv.y * 24.0);
    let tick = floor(frame.time * 12.0);
    let torn = step(0.55, hash(vec2<f32>(slice, tick), 2.0));
    let disp = (hash(vec2<f32>(slice, tick), 1.0) - 0.5) * 0.12 * k * torn;
    let split = 0.012 * k;
    let at = uv + vec2<f32>(disp, 0.0);
    let c = sample(at);
    let r = sample(at + vec2<f32>(split, 0.0)).r;
    let b = sample(at - vec2<f32>(split, 0.0)).b;
    return vec4<f32>(r, c.g, b, c.a);
}
