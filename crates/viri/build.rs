use std::process::Command;

fn main() {
    let version = std::env::var("VIRI_VERSION").unwrap_or_else(|_| {
        let date = cmd("date", &["-u", "+%Y-%m-%d-%H%M"]);
        let change = cmd(
            "jj",
            &["log", "-r", "@", "--no-graph", "-T", "change_id.short()"],
        );
        format!("{date}+{change}-dev")
    });
    println!("cargo:rustc-env=VIRI_VERSION={version}");
    // Declaring any rerun-if replaces cargo's default of watching every
    // package file, so the sources need listing alongside the env var.
    println!("cargo:rerun-if-env-changed=VIRI_VERSION");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
}

fn cmd(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}
