use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Capture {
    pub run: PathBuf,
    game: Option<PathBuf>,
    started: SystemTime,
}

impl Capture {
    pub fn start(collector: &Path) -> Result<Self> {
        let captures = collector.join("captures");
        fs::create_dir_all(&captures)?;
        private_directory(&captures)?;

        let started = SystemTime::now();
        let timestamp = started.duration_since(UNIX_EPOCH)?.as_secs();
        let run = captures.join(format!("{timestamp}-{}", std::process::id()));
        fs::create_dir(&run)?;
        private_directory(&run)?;
        for name in ["bepinex", "game", "unity", "crash"] {
            let directory = run.join(name);
            fs::create_dir(&directory)?;
            private_directory(&directory)?;
        }
        write_private(&run.join("start_epoch"), &timestamp.to_string())?;
        write_private(
            &run.join("system.txt"),
            &format!(
                "OS: {}\nCollector architecture: {}\nStarted (Unix seconds): {timestamp}\n",
                env::consts::OS,
                env::consts::ARCH,
            ),
        )?;
        write_private(
            &captures.join("current"),
            &format!("{timestamp}-{}", std::process::id()),
        )?;
        Ok(Self {
            run,
            game: None,
            started,
        })
    }

    pub fn set_game(&mut self, game: &Path) -> Result<()> {
        write_private(&self.run.join("game-root"), &game.display().to_string())?;
        let mut system = OpenOptions::new()
            .append(true)
            .open(self.run.join("system.txt"))?;
        writeln!(system, "Game root: {}", game.display())?;
        self.game = Some(game.to_path_buf());
        Ok(())
    }

    pub fn latest(collector: &Path) -> Result<Self> {
        let captures = collector.join("captures");
        let name = fs::read_to_string(captures.join("current"))?;
        let name = name.trim();
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == "."
            || name == ".."
        {
            return Err("Invalid capture identifier".into());
        }
        let run = captures.join(name);
        let started = fs::read_to_string(run.join("start_epoch"))?
            .trim()
            .parse::<u64>()?;
        let game = match fs::read_to_string(run.join("game-root")) {
            Ok(value) => Some(PathBuf::from(value.trim())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            run,
            game,
            started: UNIX_EPOCH + Duration::from_secs(started),
        })
    }

    pub fn write(&self, name: &str, value: &str) -> Result<()> {
        write_private(&self.run.join(name), value)
    }

    pub fn launcher_log(&self) -> Result<File> {
        self.log_file("launcher.log")
    }

    pub fn log_file(&self, name: &str) -> Result<File> {
        let path = self.run.join(name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        private_file(&path)?;
        Ok(file)
    }

    pub fn collect(&self) -> Result<()> {
        let game = self
            .game
            .as_ref()
            .ok_or("The latest attempt did not locate the game")?;
        write_private(&self.run.join("copied.txt"), "")?;
        self.scan(&game.join("BepInEx"), "bepinex", is_bepinex_log)?;
        self.scan(game, "bepinex", is_preloader_log)?;
        let home = home()?;

        #[cfg(target_os = "macos")]
        {
            let support =
                home.join("Library/Application Support/unity.League of Geeks.Armello/logs");
            self.scan(&support, "game", |path| has_extension(path, &["txt"]))?;
            let unity = home.join("Library/Logs/League of Geeks/Armello");
            self.scan(&unity, "unity", is_player_log)?;
            let reports = home.join("Library/Logs/DiagnosticReports");
            self.scan(&reports, "crash", |path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .to_ascii_lowercase()
                        .starts_with("armello")
                }) && has_extension(path, &["ips", "crash"])
            })?;
        }

        #[cfg(target_os = "linux")]
        {
            let unity = home.join(".config/unity3d/League of Geeks/Armello");
            self.scan(&unity.join("logs"), "game", |path| {
                has_extension(path, &["txt"])
            })?;
            self.scan(
                &home.join(".config/unity3d/unity.League of Geeks.Armello/logs"),
                "game",
                |path| has_extension(path, &["txt"]),
            )?;
            self.scan(&unity, "unity", is_player_log)?;
            let crashes = unity.join("Crashes");
            self.scan(&crashes, "crash", |path| {
                has_extension(path, &["log", "dmp"])
            })?;
            if crashes.is_dir() {
                for entry in fs::read_dir(crashes)? {
                    let entry = entry?;
                    if entry.path().is_dir() {
                        self.scan(
                            &entry.path(),
                            &format!("crash/{}", entry.file_name().to_string_lossy()),
                            |path| has_extension(path, &["log", "dmp"]),
                        )?;
                    }
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let local = home.join("AppData/LocalLow/League of Geeks/Armello");
            self.scan(&local.join("logs"), "game", |path| {
                has_extension(path, &["txt"])
            })?;
            self.scan(&local, "unity", is_player_log)?;
            if let Some(temporary) = env::var_os("TEMP") {
                let crashes = PathBuf::from(temporary).join("League of Geeks/Armello/Crashes");
                self.scan(&crashes, "crash", |path| {
                    has_extension(path, &["log", "dmp", "txt"])
                })?;
                if crashes.is_dir() {
                    for entry in fs::read_dir(crashes)? {
                        let entry = entry?;
                        if entry.path().is_dir() {
                            self.scan(
                                &entry.path(),
                                &format!("crash/{}", entry.file_name().to_string_lossy()),
                                |path| has_extension(path, &["log", "dmp", "txt"]),
                            )?;
                        }
                    }
                }
            }
            for bucket in ["ReportArchive", "ReportQueue"] {
                let reports = home
                    .join("AppData/Local/Microsoft/Windows/WER")
                    .join(bucket);
                if reports.is_dir() {
                    for entry in fs::read_dir(reports)? {
                        let entry = entry?;
                        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                        if name.starts_with("appcrash_armello") && entry.path().is_dir() {
                            self.scan(
                                &entry.path(),
                                &format!("crash/{bucket}/{}", entry.file_name().to_string_lossy()),
                                |path| has_extension(path, &["wer", "dmp"]),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn scan(&self, directory: &Path, group: &str, select: impl Fn(&Path) -> bool) -> Result<()> {
        if !directory.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if select(&path) {
                self.copy(&path, group, None)?;
            }
        }
        Ok(())
    }

    fn copy(&self, source: &Path, group: &str, name: Option<&str>) -> Result<()> {
        let metadata = match fs::symlink_metadata(source) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            _ => return Ok(()),
        };
        if metadata.modified()?
            < self
                .started
                .checked_sub(Duration::from_secs(1))
                .unwrap_or(UNIX_EPOCH)
        {
            return Ok(());
        }
        let file_name = match name {
            Some(name) => std::ffi::OsStr::new(name),
            None => source.file_name().ok_or("Log filename missing")?,
        };
        let directory = self.run.join(group);
        fs::create_dir_all(&directory)?;
        private_directory(&directory)?;
        let destination = directory.join(file_name);
        fs::copy(source, &destination)?;
        private_file(&destination)?;
        let mut copied = OpenOptions::new()
            .append(true)
            .open(self.run.join("copied.txt"))?;
        writeln!(copied, "{}", source.display())?;
        Ok(())
    }
}

pub(crate) fn append_live(run: &Path, group: &str, name: &OsStr, bytes: &[u8]) -> Result<()> {
    let live = run.join("live");
    let directory = live.join(group);
    fs::create_dir_all(&directory)?;
    private_directory(&live)?;
    private_directory(&directory)?;
    let destination = directory.join(name);
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&destination)?;
    private_file(&destination)?;
    file.write_all(bytes)?;
    Ok(())
}

fn home() -> Result<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "User home directory not set".into())
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension().is_some_and(|extension| {
        extensions
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

fn is_player_log(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("Player"))
        && has_extension(path, &["log"])
}

pub(crate) fn is_bepinex_log(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("LogOutput"))
        && has_extension(path, &["log", "txt"])
}

pub(crate) fn is_preloader_log(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("preloader_"))
        && has_extension(path, &["log"])
}

fn write_private(path: &Path, value: &str) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    private_file(path)?;
    file.write_all(value.as_bytes())?;
    Ok(())
}

#[cfg(unix)]
fn private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn private_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn private_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Capture;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn captures_attempts_before_the_game_is_known() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let collector =
            std::env::temp_dir().join(format!("armello-capture-{}-{nonce}", std::process::id()));
        fs::create_dir(&collector).unwrap();

        let mut capture = Capture::start(&collector).unwrap();
        assert!(capture.run.join("system.txt").is_file());
        assert!(!capture.run.join("game-root").exists());
        assert!(Capture::latest(&collector).unwrap().collect().is_err());

        let game = collector.join("Armello");
        capture.set_game(&game).unwrap();
        assert_eq!(
            Capture::latest(&collector).unwrap().game.as_deref(),
            Some(game.as_path())
        );

        fs::remove_dir_all(collector).unwrap();
    }

    #[test]
    fn collects_rotated_and_preloader_logs() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let collector =
            std::env::temp_dir().join(format!("armello-bepinex-{}-{nonce}", std::process::id()));
        let game = collector.join("game");
        fs::create_dir_all(game.join("BepInEx")).unwrap();
        let mut capture = Capture::start(&collector).unwrap();
        capture.set_game(&game).unwrap();
        fs::write(game.join("BepInEx/LogOutput.1.log"), "rotated").unwrap();
        fs::write(game.join("preloader_20261024.log"), "failure").unwrap();

        capture.collect().unwrap();
        assert_eq!(
            fs::read_to_string(capture.run.join("bepinex/LogOutput.1.log")).unwrap(),
            "rotated"
        );
        assert_eq!(
            fs::read_to_string(capture.run.join("bepinex/preloader_20261024.log")).unwrap(),
            "failure"
        );
        fs::remove_dir_all(collector).unwrap();
    }
}
