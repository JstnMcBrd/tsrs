//! `tsrs/tsdk/tsgo.exe`: runs the platform package's `tsrs` binary with this process's arguments and stdio.
//!
//! The package layout is `<tsrs>/tsdk/tsgo.exe` next to `<tsrs>/package.json`. The binary is found the way
//! npm/tsrs/lib/getExePath.js finds it: `TSRS_BINARY` when set, else `@tsrs/<platform>/tsrs[.exe]` in the
//! `node_modules` directories that Node's resolution searches from the `tsrs` package.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const EXE_NAME: &str = if cfg!(windows) { "tsrs.exe" } else { "tsrs" };

fn main() -> ExitCode {
    let exe = match find_binary() {
        Ok(exe) => exe,
        Err(message) => {
            eprintln!("tsgo: {message}");
            return ExitCode::FAILURE;
        }
    };
    // The child inherits stdin, stdout and stderr, so the language server talks to the editor directly.
    match Command::new(&exe).args(env::args_os().skip(1)).status() {
        // ExitCode only carries a u8, and Windows exit codes are 32 bits.
        Ok(status) => status.code().map_or(ExitCode::FAILURE, |code| std::process::exit(code)),
        Err(error) => {
            eprintln!("tsgo: failed to run {}: {error}", exe.display());
            ExitCode::FAILURE
        }
    }
}

fn find_binary() -> Result<PathBuf, String> {
    if let Some(binary) = env::var_os("TSRS_BINARY").filter(|value| !value.is_empty()) {
        let binary = PathBuf::from(binary);
        if !binary.exists() {
            return Err(format!(
                "TSRS_BINARY is set to {}, which does not exist.",
                binary.display()
            ));
        }
        return Ok(binary);
    }

    let launcher =
        env::current_exe().map_err(|error| format!("cannot locate this launcher: {error}"))?;
    // pnpm links `node_modules/tsrs` to its store, where the platform package is a sibling of the real directory, so
    // search from the real path first (Node resolves symlinks the same way), then from the path as launched.
    let mut package_dirs = Vec::new();
    if let Ok(real) = fs::canonicalize(&launcher) {
        package_dirs.extend(package_dir(&simplify(real)));
    }
    package_dirs.extend(package_dir(&launcher));

    let candidates = platform_packages();
    for name in &candidates {
        for package_dir in &package_dirs {
            for modules in node_modules_dirs(package_dir) {
                let exe = modules.join("@tsrs").join(name).join(EXE_NAME);
                if exe.is_file() {
                    return Ok(exe);
                }
            }
        }
    }

    let names =
        candidates.iter().map(|name| format!("@tsrs/{name}")).collect::<Vec<_>>().join(" or ");
    let from = package_dirs
        .first()
        .map_or_else(|| launcher.display().to_string(), |dir| dir.display().to_string());
    Err(format!(
        "Cannot find the optional dependency {names} (searched from {from}), which contains the tsrs binary. \
         Common causes: installing with --no-optional / --omit=optional, or a lockfile or node_modules created on \
         another platform. Reinstall to fix. To use a locally built binary instead, set TSRS_BINARY=/path/to/{EXE_NAME}."
    ))
}

/// `<tsrs>` for a launcher at `<tsrs>/tsdk/tsgo[.exe]`.
fn package_dir(launcher: &Path) -> Option<PathBuf> {
    launcher.parent()?.parent().map(Path::to_path_buf)
}

/// The directories Node's module resolution searches from a file in `dir`: `<ancestor>/node_modules` for each ancestor
/// of `dir` (itself included) that is not itself named `node_modules`.
fn node_modules_dirs(dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    dir.ancestors()
        .filter(|ancestor| ancestor.file_name().is_none_or(|name| name != "node_modules"))
        .map(|ancestor| ancestor.join("node_modules"))
}

/// The platform package names (without the `@tsrs/` scope) to try, best first. The package manager installs the one
/// matching the Node that ran the install. An x64 launcher on Windows ARM64 runs under emulation and still finds the
/// arm64 package when that is the one installed.
fn platform_packages() -> Vec<String> {
    let os = match env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let suffix = match (os, cfg!(target_env = "musl")) {
        ("linux", true) => "-musl",
        ("linux", false) => "-gnu",
        _ => "",
    };
    let own = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let other = if own == "arm64" { "x64" } else { "arm64" };
    [own, other].into_iter().map(|cpu| format!("{os}-{cpu}{suffix}")).collect()
}

/// Drops the `\\?\` prefix `fs::canonicalize` adds on Windows when the path does not need it, as getExePath.js only
/// adds it to long paths. The binary finds its default libraries beside its own path, and diagnostics name them.
fn simplify(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let Some(text) = path.to_str() else { return path };
    let plain: Option<String> = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        Some(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
            .then(|| rest.to_owned())
    } else {
        None
    };
    match plain {
        Some(plain) if plain.len() < 248 => PathBuf::from(plain),
        _ => path,
    }
}
