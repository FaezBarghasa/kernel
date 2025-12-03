use std::env;
use std::fs;
use std::path::Path;
use toml::Table;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=config.toml");
    println!("cargo:rerun-if-changed=config.toml.example"); // Add rerun-if-changed for example file
    println!("cargo:rerun-if-changed=linkers");

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);
    let target = env::var("TARGET").unwrap();

    // Fix for "environment variable `TARGET` not defined at compile time"
    println!("cargo:rustc-env=TARGET={}", target);

    // Config parsing
    let config_path = Path::new("config.toml");
    let config_example_path = Path::new("config.toml.example");

    let config_str = if config_path.exists() {
        fs::read_to_string(config_path).unwrap_or_default()
    } else if config_example_path.exists() {
        println!("cargo:warning=config.toml not found, using config.toml.example");
        fs::read_to_string(config_example_path).unwrap_or_default()
    } else {
        println!("cargo:warning=Neither config.toml nor config.toml.example found, using empty config.");
        String::new()
    };

    let root: Table = if !config_str.is_empty() {
        toml::from_str(&config_str).unwrap_or_else(|e| {
            println!("cargo:warning=Failed to parse config: {}", e);
            Table::new()
        })
    } else {
        Table::new()
    };

    if let Some(arch_table) = root.get("arch").and_then(|v| v.as_table()) {
        for (key, value) in arch_table {
            if let Some(choice) = value.as_str() {
                println!("cargo:rustc-cfg={}=\"{}\"", key, choice);
            }
        }
    }

    // Linker selection
    let linker_script = if target.contains("x86_64") {
        "x86_64.ld"
    } else if target.contains("aarch64") {
        "aarch64.ld"
    } else if target.contains("riscv64") {
        "riscv64.ld"
    } else if target.contains("riscv32") {
        "riscv32.ld"
    } else if target.contains("i686") || target.contains("i586") {
        "i586.ld"
    } else {
        panic!("Unsupported target for linker script selection: {}", target);
    };

    let linker_path = Path::new("linkers").join(linker_script);

    if !linker_path.exists() {
        panic!("Linker script not found: {}", linker_path.display());
    }

    let dest_path = out_path.join(linker_script);
    fs::copy(&linker_path, &dest_path).unwrap();

    println!("cargo:rustc-link-search={}", out_dir);
    println!("cargo:rustc-link-arg=-T{}", linker_script);
}