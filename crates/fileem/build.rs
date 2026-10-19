fn main() {
    // Fluent is the closest std-widgets style to the Java app's default (AtlantaFX Primer); the
    // light/dark mode follows the system. Debug builds (and so tests) keep element info, which
    // the UI tests use to find elements by their accessible labels; release builds stay lean.
    let debug = std::env::var("PROFILE").is_ok_and(|p| p == "debug");
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into()).with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("ui/app.slint compiles");
}
