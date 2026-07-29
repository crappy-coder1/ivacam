use ivac_cps::engine::{Engine, PRELUDE};
fn main() {
    let mut text = String::new();
    for (_, src) in PRELUDE {
        text.push_str(src);
        text.push('\n');
    }
    text.push_str(ivac_cps::library::bundled("grbl").unwrap().source);
    let mut e = Engine::new();
    e.eval_named("bundle.js", &text).unwrap();
    for probe in [
        "gFormat === properties",
        "typeof gFormat.getResultingValue",
        "typeof xyzFormat.getResultingValue",
    ] {
        let v = e.eval_named("p.js", probe).unwrap();
        println!(
            "{probe} -> {}",
            v.to_string(e.context_mut())
                .unwrap()
                .to_std_string_escaped()
        );
    }
}
