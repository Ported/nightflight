//! Print what the macros are doing, bar by bar.
use engine::Engine;

fn main() {
    let mut engine = Engine::new(dsp::SR, 126.0, engine::sets::intro());
    let mut buf = vec![0.0f32; 128 * 2];
    let bar_samples = 4.0 * 60.0 / 126.0 * f64::from(dsp::SR);
    println!(
        "{:>5}  {:>28}  velocity_scale / gate depth / lap",
        "bar", "macros"
    );
    for bar in 0..27 {
        let target = (bar as f64 * bar_samples) as usize;
        while ((engine.state().bar * bar_samples) as usize) < target {
            engine.process(&mut buf);
        }
        let macros: Vec<String> = engine
            .macro_values()
            .iter()
            .map(|(n, v)| format!("{n} {v:.2}"))
            .collect();
        println!(
            "{bar:5}  {:>28}  {}",
            macros.join(" "),
            engine.debug_parts()
        );
    }
}
