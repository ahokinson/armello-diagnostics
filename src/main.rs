#![cfg_attr(windows, windows_subsystem = "windows")]

mod assets;
mod collect;
mod display;
mod install;
mod live;
mod window;

use std::env;
use std::error::Error;
#[cfg(unix)]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use collect::Capture;
use install::Runtime;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let result = if args.len() == 2 && args[0] == "--view" {
        let run = verify_capture(Path::new(&args[1]));
        run.and_then(|run| {
            window::show(&run)
                .map(|()| 0)
                .map_err(|error| -> Box<dyn Error> { error.to_string().into() })
        })
    } else {
        run(&args)
    };
    let code = match &result {
        Ok(code) => *code,
        Err(error) => {
            eprintln!("Armello diagnostics: {error}");
            #[cfg(windows)]
            live::show_message("Armello diagnostics error", &format!("{error}"));
            #[cfg(unix)]
            let _ = window::show_failure(&error.to_string());
            1
        }
    };
    process::exit(code);
}

fn verify_capture(run: &Path) -> Result<PathBuf> {
    let collector = env::current_exe()?
        .parent()
        .ok_or("Collector directory missing")?
        .to_path_buf();
    let captures = collector.join("captures").canonicalize()?;
    let run = run.canonicalize()?;
    if run.parent() != Some(captures.as_path()) || !run.is_dir() {
        return Err("The diagnostics window path is outside captures".into());
    }
    Ok(run)
}

fn run(args: &[OsString]) -> Result<i32> {
    let collector = env::current_exe()?
        .parent()
        .ok_or("Collector directory missing")?
        .to_path_buf();
    if args.len() == 1 && args[0] == "--collect" {
        let capture = Capture::latest(&collector)?;
        capture.collect()?;
        println!("Capture directory: {}", capture.run.display());
        #[cfg(windows)]
        live::show_message(
            "Armello diagnostics",
            &format!("Logs collected in {}", capture.run.display()),
        );
        return Ok(0);
    }

    let mut capture = Capture::start(&collector)?;
    let mut live = live::Live::new(&capture)?;
    let result = run_capture(&mut capture, args, &live);
    if let Err(error) = &result {
        capture.write("startup-error.txt", &format!("{error}\n"))?;
        live.emit("collector", &format!("Error: {error}"))?;
    }
    live.emit(
        "collector",
        &format!("Capture directory: {}", capture.run.display()),
    )?;
    live.emit("collector", "Close the diagnostics window when finished")?;
    live.wait_for_close()?;
    Ok(result.unwrap_or(1))
}

fn run_capture(capture: &mut Capture, args: &[OsString], live: &live::Live) -> Result<i32> {
    let setup_only = args.first().is_some_and(|value| value == "--setup");
    let args = if setup_only { &args[1..] } else { args };
    let first = args.first().ok_or(
        "Usage: armello-diagnostics [--setup] <Steam game command> [arguments] | --collect",
    )?;
    let (game, executable, runtime) = locate_game(Path::new(first))?;
    capture.set_game(&game)?;
    capture.write("runtime.txt", &format!("{}\n", runtime.label()))?;

    live.emit(
        "collector",
        &format!("Game found: {}", executable.display()),
    )?;

    live.emit("collector", "Checking BepInEx setup")?;
    let setup = install::ensure(&game, &executable, runtime);
    let message = match setup {
        Ok(message) => message,
        Err(error) => {
            capture.write("setup.txt", &format!("{error}\n"))?;
            live.emit("collector", &format!("BepInEx setup failed: {error}"))?;
            return Ok(1);
        }
    };
    capture.write("setup.txt", &format!("{message}\n"))?;
    live.emit("collector", &message)?;
    if setup_only {
        return Ok(0);
    }
    let mut command = launch_command(&game, &executable, runtime);
    command.args(&args[1..]);
    command.current_dir(&game);
    let result = live.run_game(&mut command, &game);
    let code = match result {
        Ok(exit) => {
            capture.write("exit-status.txt", &format!("{}\n", exit.code))?;
            if let Some(signal) = exit.signal {
                capture.write("signal.txt", &format!("{signal}\n"))?;
                live.emit("collector", &format!("Game terminated by signal {signal}"))?;
            } else {
                live.emit(
                    "collector",
                    &format!("Game exited with status {}", exit.code),
                )?;
            }
            exit.code
        }
        Err(error) => {
            capture.write("launcher-error.txt", &format!("{error}\n"))?;
            live.emit(
                "collector",
                &format!("Game launch or monitoring failed: {error}"),
            )?;
            1
        }
    };
    live.emit("collector", "Collecting logs")?;
    capture.collect()?;
    Ok(code)
}

fn launch_command(game: &Path, executable: &Path, runtime: Runtime) -> Command {
    let mut command = Command::new(executable);
    #[cfg(unix)]
    set_doorstop(&mut command, game, runtime);
    #[cfg(windows)]
    let _ = (game, runtime);
    command
}

#[cfg(unix)]
fn set_doorstop(command: &mut Command, game: &Path, runtime: Runtime) {
    command.env("DOORSTOP_ENABLED", "1");
    command.env(
        "DOORSTOP_TARGET_ASSEMBLY",
        game.join(runtime.preloader()).as_os_str(),
    );
    command.env("DOORSTOP_BOOT_CONFIG_OVERRIDE", "");
    command.env("DOORSTOP_IGNORE_DISABLED_ENV", "0");
    command.env("DOORSTOP_MONO_DLL_SEARCH_PATH_OVERRIDE", "");
    command.env("DOORSTOP_MONO_DEBUG_ENABLED", "0");
    command.env("DOORSTOP_MONO_DEBUG_ADDRESS", "127.0.0.1:10000");
    command.env("DOORSTOP_MONO_DEBUG_SUSPEND", "0");
    if runtime == Runtime::Il2Cpp {
        command.env("DOORSTOP_CLR_CORLIB_DIR", game.join("dotnet").as_os_str());
    } else {
        command.env("DOORSTOP_CLR_CORLIB_DIR", "");
    }

    #[cfg(target_os = "macos")]
    {
        command.env(
            "DOORSTOP_CLR_RUNTIME_CORECLR_PATH",
            if runtime == Runtime::Il2Cpp {
                game.join("dotnet/libcoreclr.dylib").into_os_string()
            } else {
                ".dylib".into()
            },
        );
        prepend_paths(
            command,
            "DYLD_INSERT_LIBRARIES",
            &[game.join("libdoorstop.dylib").as_os_str()],
        );
        if runtime == Runtime::Il2Cpp {
            prepend_paths(
                command,
                "DYLD_LIBRARY_PATH",
                &[game.as_os_str(), game.join("dotnet").as_os_str()],
            );
        } else {
            prepend_paths(command, "DYLD_LIBRARY_PATH", &[game.as_os_str()]);
        }
    }

    #[cfg(target_os = "linux")]
    {
        command.env(
            "DOORSTOP_CLR_RUNTIME_CORECLR_PATH",
            if runtime == Runtime::Il2Cpp {
                game.join("dotnet/libcoreclr.so").into_os_string()
            } else {
                ".so".into()
            },
        );
        prepend_paths(
            command,
            "LD_PRELOAD",
            &[game.join("libdoorstop.so").as_os_str()],
        );
        if runtime == Runtime::Il2Cpp {
            prepend_paths(
                command,
                "LD_LIBRARY_PATH",
                &[game.as_os_str(), game.join("dotnet").as_os_str()],
            );
        } else {
            prepend_paths(command, "LD_LIBRARY_PATH", &[game.as_os_str()]);
        }
    }
}

#[cfg(unix)]
fn prepend_paths(command: &mut Command, key: &str, paths: &[&OsStr]) {
    let mut value = OsString::new();
    for path in paths {
        if !value.is_empty() {
            value.push(":");
        }
        value.push(path);
    }
    if let Some(inherited) = env::var_os(key) {
        if !inherited.is_empty() {
            value.push(":");
            value.push(inherited);
        }
    }
    command.env(key, value);
}

fn locate_game(command: &Path) -> Result<(PathBuf, PathBuf, Runtime)> {
    #[cfg(target_os = "macos")]
    {
        let app = if command
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            command.to_path_buf()
        } else {
            command
                .ancestors()
                .find(|part| {
                    part.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
                })
                .ok_or("Steam must pass the Armello app or executable")?
                .to_path_buf()
        };
        if !app
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("armello.app"))
        {
            return Err("The Steam command does not point to Armello".into());
        }
        let game = app
            .parent()
            .ok_or("Game directory missing")?
            .canonicalize()?;
        let executable = game.join("armello.app/Contents/MacOS/Armello");
        let runtime = detect_runtime(&game.join("armello.app/Contents/Resources/Data"))?;
        Ok((game, executable, runtime))
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let executable = command.canonicalize()?;
        let name = executable
            .file_name()
            .ok_or("Game executable missing")?
            .to_string_lossy()
            .to_ascii_lowercase();
        if !name.starts_with("armello") {
            return Err("The Steam command does not point to Armello".into());
        }
        let game = executable
            .parent()
            .ok_or("Game directory missing")?
            .to_path_buf();
        let runtime = detect_runtime(&game.join("Armello_Data"))?;
        Ok((game, executable, runtime))
    }
}

fn detect_runtime(data: &Path) -> Result<Runtime> {
    let mono = data.join("Managed/Assembly-CSharp.dll").is_file();
    let il2cpp = data
        .join("il2cpp_data/Metadata/global-metadata.dat")
        .is_file();
    match (mono, il2cpp) {
        (true, false) => Ok(Runtime::Mono),
        (false, true) => Ok(Runtime::Il2Cpp),
        (true, true) => Err(
            "Armello contains both Mono and IL2CPP markers; cannot select a BepInEx runtime".into(),
        ),
        (false, false) => {
            Err("Armello contains neither Mono assemblies nor IL2CPP metadata".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_runtime, Runtime};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn selects_runtime_only_when_one_backend_is_present() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data =
            std::env::temp_dir().join(format!("armello-runtime-{}-{nonce}", std::process::id()));
        fs::create_dir_all(data.join("Managed")).unwrap();
        fs::create_dir_all(data.join("il2cpp_data/Metadata")).unwrap();
        assert!(detect_runtime(&data).is_err());

        fs::write(data.join("Managed/Assembly-CSharp.dll"), "").unwrap();
        assert_eq!(detect_runtime(&data).unwrap(), Runtime::Mono);
        fs::write(data.join("il2cpp_data/Metadata/global-metadata.dat"), "").unwrap();
        assert!(detect_runtime(&data).is_err());
        fs::remove_file(data.join("Managed/Assembly-CSharp.dll")).unwrap();
        assert_eq!(detect_runtime(&data).unwrap(), Runtime::Il2Cpp);
        fs::remove_dir_all(data).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn configures_coreclr_only_for_il2cpp() {
        use super::launch_command;
        use std::path::Path;

        let game = Path::new("/game");
        let envs = |runtime| {
            launch_command(game, Path::new("/game/Armello"), runtime)
                .get_envs()
                .map(|(key, value)| {
                    (
                        key.to_str().unwrap().to_string(),
                        value.unwrap().to_os_string(),
                    )
                })
                .collect::<std::collections::HashMap<String, std::ffi::OsString>>()
        };
        let mono = envs(Runtime::Mono);
        let il2cpp = envs(Runtime::Il2Cpp);
        assert_eq!(
            mono["DOORSTOP_TARGET_ASSEMBLY"],
            "/game/BepInEx/core/BepInEx.Preloader.dll"
        );
        assert_eq!(mono["DOORSTOP_CLR_CORLIB_DIR"], "");
        assert_eq!(
            il2cpp["DOORSTOP_TARGET_ASSEMBLY"],
            "/game/BepInEx/core/BepInEx.Unity.IL2CPP.dll"
        );
        assert_eq!(il2cpp["DOORSTOP_CLR_CORLIB_DIR"], "/game/dotnet");
        #[cfg(target_os = "macos")]
        let (coreclr, library_path) = ("/game/dotnet/libcoreclr.dylib", "DYLD_LIBRARY_PATH");
        #[cfg(target_os = "linux")]
        let (coreclr, library_path) = ("/game/dotnet/libcoreclr.so", "LD_LIBRARY_PATH");
        assert_eq!(il2cpp["DOORSTOP_CLR_RUNTIME_CORECLR_PATH"], coreclr);
        assert!(il2cpp[library_path]
            .to_string_lossy()
            .starts_with("/game:/game/dotnet"));
    }
}
