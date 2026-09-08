use std::path::{Path, PathBuf};
use std::process::Command;

fn in_path(name: &str) -> Option<PathBuf> {
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&candidate).ok()?.permissions().mode();
                if mode & 0o111 != 0 {
                    return Some(candidate);
                }
            }
            #[cfg(not(unix))]
            return Some(candidate);
        }
    }
    None
}

/// Cross-compiler selection, most explicit first:
/// 1. `UMANAGER_MINGW_CC` (any wrapper/compiler the developer names),
/// 2. system `x86_64-w64-mingw32-gcc` (CI installs gcc-mingw-w64-x86-64),
/// 3. the repo-local zig shim `.tools/mingw-cc` (works out of the box),
/// 4. a bare `zig` on PATH.
/// Zig is invoked with `-target x86_64-windows-gnu` and without
/// `--no-insert-timestamp` (unsupported there); gcc-style tools keep both.
enum Compiler {
    Gcc(PathBuf),
    Zig(PathBuf),
}

fn find_compiler() -> Option<Compiler> {
    if let Ok(explicit) = std::env::var("UMANAGER_MINGW_CC") {
        if !explicit.trim().is_empty() {
            return Some(Compiler::Gcc(PathBuf::from(explicit)));
        }
    }
    if let Some(mingw) = in_path("x86_64-w64-mingw32-gcc") {
        return Some(Compiler::Gcc(mingw));
    }
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tools/mingw-cc");
    if shim.is_file() {
        return Some(Compiler::Gcc(shim));
    }
    if let Some(zig) = in_path("zig") {
        return Some(Compiler::Zig(zig));
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=resources/windows/wecom-titlebar.c");
    println!("cargo:rerun-if-env-changed=UMANAGER_MINGW_CC");
    let Some(compiler) = find_compiler() else {
        panic!(
            "Building the Wine titlebar helper needs a Windows cross-compiler.\n\
             Install gcc-mingw-w64-x86-64 (CI does), put zig on PATH, or set UMANAGER_MINGW_CC."
        );
    };
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("wecom-titlebar.exe");
    let (program, args): (&Path, Vec<String>) = match &compiler {
        Compiler::Gcc(program) => (
            program,
            vec![
                "-Os".into(),
                "-s".into(),
                "-static".into(),
                "-Wl,--no-insert-timestamp".into(),
                "resources/windows/wecom-titlebar.c".into(),
                "-o".into(),
                output.display().to_string(),
                "-luser32".into(),
            ],
        ),
        Compiler::Zig(program) => (
            program,
            vec![
                "cc".into(),
                "-target".into(),
                "x86_64-windows-gnu".into(),
                "-Os".into(),
                "-s".into(),
                "-static".into(),
                "resources/windows/wecom-titlebar.c".into(),
                "-o".into(),
                output.display().to_string(),
                "-luser32".into(),
            ],
        ),
    };
    let status = match &compiler {
        Compiler::Gcc(_) => Command::new(program).args(&args).status(),
        Compiler::Zig(_) => {
            // zig wants writable cache dirs; ~/.cache may be read-only, so pin
            // them inside this build script's own output directory.
            let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
            Command::new(program)
                .args(&args)
                .env("ZIG_GLOBAL_CACHE_DIR", out.join("zig-cache"))
                .env("ZIG_LOCAL_CACHE_DIR", out.join("zig-local"))
                .status()
        }
    }
    .unwrap_or_else(|e| panic!("failed to run cross-compiler {}: {e}", program.display()));
    assert!(status.success(), "Windows titlebar helper compilation failed");
    tauri_build::build()
}
