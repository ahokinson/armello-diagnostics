use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::collect::{self, Capture};
use crate::Result;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const BUFFER_SIZE: usize = 4096;
const OUTPUT_QUEUE_CAPACITY: usize = 64;
const MAX_LOG_BYTES_PER_POLL: usize = 64 * 1024;

pub struct Live {
    run: PathBuf,
    session: File,
    launcher: File,
    viewer: Option<Child>,
}

pub struct GameExit {
    pub code: i32,
    pub signal: Option<i32>,
}

impl Live {
    pub fn new(capture: &Capture) -> Result<Self> {
        let mut live = Self {
            run: capture.run.clone(),
            session: capture.log_file("session.log")?,
            launcher: capture.launcher_log()?,
            viewer: None,
        };
        live.emit("collector", "Diagnostics started")?;
        let viewer = Command::new(env::current_exe()?)
            .arg("--view")
            .arg(&capture.run)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match viewer {
            Ok(child) => {
                live.emit(
                    "collector",
                    &format!("Diagnostics window started (PID {})", child.id()),
                )?;
                live.viewer = Some(child);
            }
            Err(error) => live.emit(
                "collector",
                &format!("Could not open diagnostics window: {error}"),
            )?,
        }
        Ok(live)
    }

    pub fn wait_for_close(&mut self) -> io::Result<()> {
        if let Some(mut viewer) = self.viewer.take() {
            viewer.wait()?;
        }
        Ok(())
    }

    pub fn emit(&self, source: &str, message: &str) -> io::Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut session = &self.session;
        for line in message.lines() {
            let formatted = format!("[{timestamp}] [{source}] {line}\n");
            session.write_all(formatted.as_bytes())?;
            let _ = io::stdout().lock().write_all(formatted.as_bytes());
        }
        Ok(())
    }

    pub fn run_game(&self, command: &mut Command, game: &Path) -> io::Result<GameExit> {
        let mut follow = Follow::new(game)?;
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        self.emit("collector", &format!("Game running (PID {})", child.id()))?;

        let (sender, receiver) = mpsc::sync_channel(OUTPUT_QUEUE_CAPACITY);
        for (source, pipe) in [
            (
                "stdout",
                child
                    .stdout
                    .take()
                    .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
            ),
            (
                "stderr",
                child
                    .stderr
                    .take()
                    .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
            ),
        ] {
            if let Some(pipe) = pipe {
                let sender = sender.clone();
                thread::spawn(move || {
                    let mut pipe = pipe;
                    let mut buffer = [0_u8; BUFFER_SIZE];
                    loop {
                        match pipe.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(size) => {
                                if sender.send((source, buffer[..size].to_vec())).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
        }
        drop(sender);

        let mut heartbeat = Instant::now();
        let status = loop {
            self.drain(&receiver)?;
            if let Err(error) = follow.poll(self) {
                self.emit("collector", &format!("Live log read failed: {error}"))?;
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
                self.emit(
                    "collector",
                    &format!("Collector active; game PID {} running", child.id()),
                )?;
                heartbeat = Instant::now();
            }
            thread::sleep(POLL_INTERVAL);
        };

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        while Instant::now() < deadline {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((source, bytes)) => self.output(source, &bytes)?,
                Err(_) => break,
            }
        }
        self.drain(&receiver)?;
        if let Err(error) = follow.poll(self) {
            self.emit("collector", &format!("Live log read failed: {error}"))?;
        }
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(windows)]
        let signal = None;
        Ok(GameExit {
            code: status.code().unwrap_or(1),
            signal,
        })
    }

    fn drain(&self, receiver: &Receiver<(&str, Vec<u8>)>) -> io::Result<()> {
        for _ in 0..OUTPUT_QUEUE_CAPACITY {
            let Ok((source, bytes)) = receiver.try_recv() else {
                break;
            };
            self.output(source, &bytes)?;
        }
        Ok(())
    }

    fn output(&self, source: &str, bytes: &[u8]) -> io::Result<()> {
        let mut launcher = &self.launcher;
        launcher.write_all(bytes)?;
        self.emit_bytes(source, bytes)
    }

    fn emit_bytes(&self, source: &str, bytes: &[u8]) -> io::Result<()> {
        for line in String::from_utf8_lossy(bytes).split_terminator('\n') {
            self.emit(
                source,
                &line.trim_end_matches('\r').escape_debug().to_string(),
            )?;
        }
        Ok(())
    }
}

struct Follow {
    game: PathBuf,
    player_directory: PathBuf,
    log_directories: Vec<PathBuf>,
    offsets: HashMap<PathBuf, FileState>,
}

struct FileState {
    offset: u64,
    generation: u32,
}

struct WatchedLog {
    path: PathBuf,
    group: &'static str,
}

impl Follow {
    fn new(game: &Path) -> io::Result<Self> {
        let home = env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "User home directory not set")
            })?;
        let home = PathBuf::from(home);
        #[cfg(windows)]
        let player_directory = home.join("AppData/LocalLow/League of Geeks/Armello");
        #[cfg(windows)]
        let log_directories = vec![player_directory.join("logs")];

        #[cfg(target_os = "macos")]
        let player_directory = home.join("Library/Logs/League of Geeks/Armello");
        #[cfg(target_os = "macos")]
        let log_directories =
            vec![home.join("Library/Application Support/unity.League of Geeks.Armello/logs")];

        #[cfg(target_os = "linux")]
        let player_directory = home.join(".config/unity3d/League of Geeks/Armello");
        #[cfg(target_os = "linux")]
        let log_directories = vec![
            player_directory.join("logs"),
            home.join(".config/unity3d/unity.League of Geeks.Armello/logs"),
        ];

        let mut follow = Self {
            game: game.to_path_buf(),
            player_directory,
            log_directories,
            offsets: HashMap::new(),
        };
        for watched in follow.paths()? {
            if let Ok(metadata) = fs::symlink_metadata(&watched.path) {
                if metadata.file_type().is_file() {
                    follow.offsets.insert(
                        watched.path,
                        FileState {
                            offset: metadata.len(),
                            generation: 0,
                        },
                    );
                }
            }
        }
        Ok(follow)
    }

    fn paths(&self) -> io::Result<Vec<WatchedLog>> {
        let mut paths = vec![WatchedLog {
            path: self.game.join("BepInEx/LogOutput.log"),
            group: "bepinex",
        }];
        if self.player_directory.is_dir() {
            for entry in fs::read_dir(&self.player_directory)? {
                let path = entry?.path();
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("Player"))
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
                {
                    paths.push(WatchedLog {
                        path,
                        group: "unity",
                    });
                }
            }
        }
        for (index, directory) in self.log_directories.iter().enumerate() {
            if directory.is_dir() {
                for entry in fs::read_dir(directory)? {
                    let path = entry?.path();
                    if path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
                    {
                        paths.push(WatchedLog {
                            path,
                            group: if index == 0 { "game" } else { "game-alt" },
                        });
                    }
                }
            }
        }
        Ok(paths)
    }

    fn poll(&mut self, live: &Live) -> io::Result<()> {
        for watched in self.paths()? {
            let path = watched.path;
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_file() => metadata,
                _ => continue,
            };
            let state = self.offsets.entry(path.clone()).or_insert(FileState {
                offset: 0,
                generation: 0,
            });
            if metadata.len() < state.offset {
                state.offset = 0;
                state.generation += 1;
                live.emit("collector", &format!("Log restarted: {}", path.display()))?;
            }
            if metadata.len() == state.offset {
                continue;
            }
            let mut file = match OpenOptions::new().read(true).open(&path) {
                Ok(file) => file,
                Err(error) => {
                    live.emit(
                        "collector",
                        &format!("Cannot read {}: {error}", path.display()),
                    )?;
                    continue;
                }
            };
            file.seek(SeekFrom::Start(state.offset))?;
            let mut remaining = MAX_LOG_BYTES_PER_POLL;
            let mut buffer = [0_u8; BUFFER_SIZE];
            while remaining > 0 {
                let size = file.read(&mut buffer)?;
                if size == 0 {
                    break;
                }
                state.offset += size as u64;
                remaining -= size;
                live.emit_bytes(&path.display().to_string(), &buffer[..size])?;
                let mut name = path
                    .file_name()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "Log filename missing")
                    })?
                    .to_os_string();
                if state.generation > 0 {
                    name.push(format!(".{}", state.generation));
                }
                if let Err(error) =
                    collect::append_live(&live.run, watched.group, &name, &buffer[..size])
                {
                    live.emit(
                        "collector",
                        &format!("Could not preserve {}: {error}", path.display()),
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub fn show_message(title: &str, text: &str) {
    let wide = |value: &str| {
        value
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text).as_ptr(),
            wide(title).as_ptr(),
            0x0001_0000,
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::Live;
    use crate::collect::Capture;
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn streams_game_output_and_bepinex_log_to_session() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let collector =
            std::env::temp_dir().join(format!("armello-live-{}-{nonce}", std::process::id()));
        let game = collector.join("game");
        fs::create_dir_all(game.join("BepInEx")).unwrap();
        let capture = Capture::start(&collector).unwrap();
        let live = Live {
            run: capture.run.clone(),
            session: capture.log_file("session.log").unwrap(),
            launcher: capture.launcher_log().unwrap(),
            viewer: None,
        };
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "printf 'game output\\n'; printf 'game error\\n' >&2; printf 'BepInEx output\\n' > \"$TEST_LOG_PATH\"",
            ])
            .env("TEST_LOG_PATH", game.join("BepInEx/LogOutput.log"));

        assert_eq!(live.run_game(&mut command, &game).unwrap().code, 0);
        let launcher = fs::read_to_string(capture.run.join("launcher.log")).unwrap();
        let session = fs::read_to_string(capture.run.join("session.log")).unwrap();
        assert!(launcher.contains("game output"));
        assert!(launcher.contains("game error"));
        assert!(session.contains("[stdout] game output"));
        assert!(session.contains("[stderr] game error"));
        assert!(session.contains("BepInEx output"));
        assert_eq!(
            fs::read_to_string(capture.run.join("live/bepinex/LogOutput.log")).unwrap(),
            "BepInEx output\n"
        );

        fs::remove_dir_all(collector).unwrap();
    }
}
