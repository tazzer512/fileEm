fn main() {
    // Fluent is the closest std-widgets style to the Java app's default (AtlantaFX Primer); the
    // theme (src/theme.rs) sets its light or dark mode. Debug builds (and so tests) keep element
    // info, which the UI tests use to find elements by their accessible labels; release builds
    // stay lean.
    let debug = std::env::var("PROFILE").is_ok_and(|p| p == "debug");
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into()).with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("ui/app.slint compiles");
    about();
}

/// What the About box shows besides the version (src/about.rs): the build date, the compiler
/// and the Slint version. Rebuilt with the sources, so the date is the last build that changed
/// something.
fn about() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // SOURCE_DATE_EPOCH makes reproducible builds
    let secs = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    });
    let (y, m, d) = civil(secs.div_euclid(86_400));
    println!("cargo:rustc-env=FILEEM_BUILT={y:04}-{m:02}-{d:02}");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let version = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|v| v.split_whitespace().nth(1).map(str::to_owned))
        .unwrap_or_default();
    println!("cargo:rustc-env=FILEEM_RUSTC={version}");
    let lock = std::fs::read_to_string("../../Cargo.lock").unwrap_or_default();
    let slint = lock
        .split("[[package]]")
        .find(|p| p.contains("\nname = \"slint\"\n"))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
        .map(|v| v.trim_end_matches('"'))
        .unwrap_or_default();
    println!("cargo:rustc-env=FILEEM_SLINT={slint}");
}

/// The date of a day counted from 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
