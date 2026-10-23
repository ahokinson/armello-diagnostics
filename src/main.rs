#![cfg_attr(windows, windows_subsystem = "windows")]

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
    let (game, executable) = locate_game(Path::new(first))?;
    capture.set_game(&game)?;

    live.emit(
        "collector",
        &format!("Game found: {}", executable.display()),
    )?;

    if setup_only {
        let message = install::ensure(&game, &executable)?;
        capture.write("setup.txt", &format!("{message}\n"))?;
        live.emit("collector", &message)?;
        return Ok(0);
    }

    live.emit("collector", "Checking BepInEx setup")?;
    let setup = install::ensure(&game, &executable);
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
    let mut command = launch_command(&game, &executable);
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

fn launch_command(game: &Path, executable: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(unix)]
    set_doorstop(&mut command, game);
    command
}

#[cfg(unix)]
fn set_doorstop(command: &mut Command, game: &Path) {
    command.env("DOORSTOP_ENABLED", "1");
    command.env(
        "DOORSTOP_TARGET_ASSEMBLY",
        game.join("BepInEx/core/BepInEx.Preloader.dll").as_os_str(),
    );
    command.env("DOORSTOP_BOOT_CONFIG_OVERRIDE", "");
    command.env("DOORSTOP_IGNORE_DISABLED_ENV", "0");
    command.env("DOORSTOP_MONO_DLL_SEARCH_PATH_OVERRIDE", "");
    command.env("DOORSTOP_MONO_DEBUG_ENABLED", "0");
    command.env("DOORSTOP_MONO_DEBUG_ADDRESS", "127.0.0.1:10000");
    command.env("DOORSTOP_MONO_DEBUG_SUSPEND", "0");
    command.env("DOORSTOP_CLR_CORLIB_DIR", "");

    #[cfg(target_os = "macos")]
    {
        command.env("DOORSTOP_CLR_RUNTIME_CORECLR_PATH", ".dylib");
        prepend_path(
            command,
            "DYLD_INSERT_LIBRARIES",
            game.join("libdoorstop.dylib").as_os_str(),
        );
        prepend_path(command, "DYLD_LIBRARY_PATH", game.as_os_str());
    }

    #[cfg(target_os = "linux")]
    {
        command.env("DOORSTOP_CLR_RUNTIME_CORECLR_PATH", ".so");
        prepend_path(
            command,
            "LD_PRELOAD",
            game.join("libdoorstop.so").as_os_str(),
        );
        prepend_path(command, "LD_LIBRARY_PATH", game.as_os_str());
    }
}

#[cfg(unix)]
fn prepend_path(command: &mut Command, key: &str, first: &OsStr) {
    let mut value = first.to_os_string();
    if let Some(inherited) = env::var_os(key) {
        if !inherited.is_empty() {
            value.push(":");
            value.push(inherited);
        }
    }
    command.env(key, value);
}

fn locate_game(command: &Path) -> Result<(PathBuf, PathBuf)> {
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
        if !game
            .join("armello.app/Contents/Resources/Data/Managed/Assembly-CSharp.dll")
            .is_file()
        {
            return Err(
                "This Armello installation does not contain the expected Mono assemblies".into(),
            );
        }
        Ok((game, executable))
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
        let data = game.join("Armello_Data/Managed/Assembly-CSharp.dll");
        if !data.is_file() {
            return Err(
                "This Armello installation does not contain the expected Mono assemblies".into(),
            );
        }
        Ok((game, executable))
    }
}
