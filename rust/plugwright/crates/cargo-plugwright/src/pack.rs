//! `lib`, `wasm` and `plugin`: build a crate of the current cargo workspace,
//! named on the command line, and copy the result into an output directory.
//! No `plugwright.toml` is involved; the cache lives in the workspace's
//! target directory.

use std::path::{Path, PathBuf};

use plugwright::error::io_err;
use plugwright::package::find_package;
use plugwright::{BuildOptions, BuildRequest, Builder, Cache, CrateType, Loader, PluginError, Target};

/// What to make of the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// A native library of this kind, whatever the crate declares.
    Library(CrateType),
    /// A `cdylib` for `wasm32-unknown-unknown`.
    Wasm,
    /// The crate as it declares itself, checked by loading it as a plugin.
    Plugin { wasm: bool },
}

#[derive(Debug, PartialEq, Eq)]
pub struct PackArgs {
    pub krate: String,
    pub output: Output,
    pub out_dir: PathBuf,
}

/// Arguments of `cmd` (`lib`, `wasm` or `plugin`) after the command name.
pub fn parse(cmd: &str, args: &[String]) -> Result<PackArgs, String> {
    let mut output = match cmd {
        "lib" => Output::Library(CrateType::Cdylib),
        "wasm" => Output::Wasm,
        _ => Output::Plugin { wasm: false },
    };
    let (mut krate, mut out_dir) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match (a.as_str(), &mut output) {
            ("--dynamic", Output::Library(t)) => *t = CrateType::Cdylib,
            ("--static", Output::Library(t)) => *t = CrateType::Staticlib,
            ("--wasm", Output::Plugin { wasm }) => *wasm = true,
            ("--out-dir", _) => match it.next() {
                Some(d) => out_dir = Some(PathBuf::from(d)),
                None => return Err("--out-dir needs a path".into()),
            },
            (flag, _) if flag.starts_with('-') => return Err(format!("unknown option `{flag}` for `{cmd}`")),
            (name, _) if krate.is_none() => krate = Some(name.to_owned()),
            _ => return Err(format!("`{cmd}` takes one crate name")),
        }
    }
    let krate = krate.ok_or_else(|| format!("`{cmd}` needs a crate name"))?;
    Ok(PackArgs { krate, output, out_dir: out_dir.unwrap_or_else(|| "dist".into()) })
}

/// The platform's file name for library `lib` with extension `ext`: Unix
/// libraries carry a `lib` prefix, Windows ones and Wasm modules do not.
pub fn file_name(lib: &str, ext: &str) -> String {
    match ext {
        "dll" | "lib" | "wasm" => format!("{lib}.{ext}"),
        _ => format!("lib{lib}.{ext}"),
    }
}

pub fn pack(a: &PackArgs) -> Result<bool, PluginError> {
    let opts = BuildOptions::default();
    let cwd = std::env::current_dir().map_err(|e| PluginError::Manifest(format!("current directory: {e}")))?;
    let pkg = find_package(&opts.cargo, &cwd, &a.krate)?;
    let cache_dir = pkg.target_dir.join("plugwright");
    let (artifact, info) = match a.output {
        Output::Plugin { wasm } => {
            let loader = Loader::with_options(&cache_dir, opts)?;
            let artifact = if wasm { loader.build_wasm(&pkg.dir)? } else { loader.build(&pkg.dir)? };
            // Runs the plugin's `new` in this process: the crate is the user's
            // own, whose build script cargo has just run unconfined anyway.
            let info = loader.open(&artifact.path)?.info().clone();
            (artifact, Some(info))
        }
        Output::Library(_) | Output::Wasm => {
            let (target, crate_type) = match a.output {
                Output::Library(t) => (Target::Native, t),
                _ => (Target::Wasm, CrateType::Cdylib),
            };
            let builder = Builder::new(Cache::new(&cache_dir)?, opts);
            let artifact = builder.build(&BuildRequest {
                id: pkg.name.clone(),
                source_label: format!("path+{}", pkg.dir.display()),
                crate_dir: pkg.dir.clone(),
                target,
                features: Vec::new(),
                no_default_features: false,
                // A library may end up in any process; Cranelift code aborts on panic.
                in_process: target == Target::Native,
                crate_type: Some(crate_type),
            })?;
            (artifact, None)
        }
    };
    let ext = artifact.path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let dest = copy_out(&artifact.path, &a.out_dir, &file_name(&pkg.lib_name, ext))?;
    let state = if artifact.cached { "cached" } else { "built" };
    match info {
        Some(i) => println!("  ✓ {:<20} {} ({state}; plugin {} {})", pkg.name, dest.display(), i.name, i.version),
        None => println!("  ✓ {:<20} {} ({state})", pkg.name, dest.display()),
    }
    Ok(true)
}

/// Copy through a temporary file and rename, so a library some process has
/// mapped from `dir` is replaced rather than rewritten under it.
fn copy_out(src: &Path, dir: &Path, name: &str) -> Result<PathBuf, PluginError> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::copy(src, &tmp).map_err(io_err(&tmp))?;
    std::fs::rename(&tmp, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(&dest)(e)
    })?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(cmd: &str, a: &[&str]) -> Result<PackArgs, String> {
        parse(cmd, &a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    fn ok(krate: &str, output: Output, out_dir: &str) -> Result<PackArgs, String> {
        Ok(PackArgs { krate: krate.into(), output, out_dir: out_dir.into() })
    }

    #[test]
    fn commands_parse_their_own_flags() {
        let dynamic = Output::Library(CrateType::Cdylib);
        let stat = Output::Library(CrateType::Staticlib);
        assert_eq!(args("lib", &["x"]), ok("x", dynamic, "dist"));
        assert_eq!(args("lib", &["--static", "x"]), ok("x", stat, "dist"));
        assert_eq!(args("lib", &["x", "--static", "--dynamic"]), ok("x", dynamic, "dist"));
        assert_eq!(args("lib", &["x", "--out-dir", "out/libs"]), ok("x", dynamic, "out/libs"));
        assert_eq!(args("wasm", &["x"]), ok("x", Output::Wasm, "dist"));
        assert_eq!(args("plugin", &["x"]), ok("x", Output::Plugin { wasm: false }, "dist"));
        assert_eq!(args("plugin", &["--wasm", "x"]), ok("x", Output::Plugin { wasm: true }, "dist"));
    }

    #[test]
    fn bad_arguments_are_refused() {
        for (cmd, a, why) in [
            ("lib", &[][..], "needs a crate name"),
            ("lib", &["a", "b"], "one crate name"),
            ("lib", &["x", "--wasm"], "unknown option `--wasm`"),
            ("wasm", &["x", "--static"], "unknown option `--static`"),
            ("plugin", &["x", "--dynamic"], "unknown option `--dynamic`"),
            ("plugin", &["x", "--out-dir"], "--out-dir needs a path"),
        ] {
            let e = args(cmd, a).unwrap_err();
            assert!(e.contains(why), "{cmd} {a:?}: {e}");
        }
    }

    #[test]
    fn file_names_follow_the_platform_convention() {
        for (ext, name) in [
            ("so", "libmy_lib.so"),
            ("dylib", "libmy_lib.dylib"),
            ("a", "libmy_lib.a"),
            ("dll", "my_lib.dll"),
            ("lib", "my_lib.lib"),
            ("wasm", "my_lib.wasm"),
        ] {
            assert_eq!(file_name("my_lib", ext), name);
        }
    }

    #[test]
    fn copy_out_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        std::fs::write(&src, b"new").unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("lib.a"), b"old").unwrap();
        let dest = copy_out(&src, &out, "lib.a").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1, "no temporary file left behind");
    }
}
