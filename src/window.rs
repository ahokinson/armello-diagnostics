use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::display::{Feed, Level, SessionLine};

const REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const READ_CHUNK_BYTES: usize = 8192;
const MAX_READ_BYTES: usize = 64 * 1024;

pub fn show(run: &Path) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Armello diagnostics")
            .with_inner_size([920.0, 600.0]),
        ..Default::default()
    };
    let directory = run.to_path_buf();
    let result = eframe::run_native(
        "Armello diagnostics",
        options,
        Box::new(move |_context| Ok(Box::new(Diagnostics::new(directory)))),
    );
    if let Err(error) = &result {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(run.join("window-error.txt")) {
            let _ = writeln!(file, "{error}");
        }
    }
    result
}

pub fn show_failure(message: &str) -> eframe::Result<()> {
    let error = message.to_string();
    eframe::run_native(
        "Armello diagnostics error",
        eframe::NativeOptions::default(),
        Box::new(move |_context| Ok(Box::new(Failure { error }))),
    )
}

struct Failure {
    error: String,
}

impl eframe::App for Failure {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Armello diagnostics could not start");
            ui.label(&self.error);
        });
    }
}

struct Diagnostics {
    run: PathBuf,
    file: Option<File>,
    offset: u64,
    pending: Vec<u8>,
    feed: Feed,
    status: String,
    last_update: Option<Instant>,
    last_game_output: Option<Instant>,
    game_started: bool,
    game_stopped: bool,
    finished: bool,
}

impl Diagnostics {
    fn new(run: PathBuf) -> Self {
        Self {
            run,
            file: None,
            offset: 0,
            pending: Vec::new(),
            feed: Feed::default(),
            status: "Waiting for collector".to_string(),
            last_update: None,
            last_game_output: None,
            game_started: false,
            game_stopped: false,
            finished: false,
        }
    }

    fn poll(&mut self) -> std::io::Result<()> {
        if self.file.is_none() {
            self.file = Some(File::open(self.run.join("session.log"))?);
        }
        let mut file = self.file.take().expect("session file opened");
        let result = self.read_new(&mut file);
        self.file = Some(file);
        result
    }

    fn read_new(&mut self, file: &mut File) -> std::io::Result<()> {
        if file.metadata()?.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut remaining = MAX_READ_BYTES;
        let mut chunk = [0_u8; READ_CHUNK_BYTES];
        while remaining > 0 {
            let size = file.read(&mut chunk)?;
            if size == 0 {
                break;
            }
            self.offset += size as u64;
            remaining -= size;
            self.pending.extend_from_slice(&chunk[..size]);
            while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line = String::from_utf8_lossy(&self.pending[..end]).to_string();
                self.push(line);
                self.pending.drain(..=end);
            }
            if self.pending.len() > MAX_READ_BYTES {
                let line = String::from_utf8_lossy(&self.pending).to_string();
                self.push(line);
                self.pending.clear();
            }
        }
        Ok(())
    }

    fn push(&mut self, line: String) {
        if let Some(entry) = SessionLine::parse(&line) {
            if entry.source == "collector" {
                self.status = entry.message.to_string();
                if entry.message.starts_with("Game running (PID ") {
                    self.game_started = true;
                }
                if entry.message.starts_with("Game exited with status ")
                    || entry.message.starts_with("Game terminated by signal ")
                    || entry
                        .message
                        .starts_with("Game launch or monitoring failed: ")
                {
                    self.game_stopped = true;
                }
                if entry.message.starts_with("Capture directory:") {
                    self.finished = true;
                }
            } else {
                self.last_game_output = Some(Instant::now());
            }
            self.feed.push(&entry);
        }
        self.last_update = Some(Instant::now());
    }

    fn indications(&self) -> io::Result<Vec<(&'static str, String)>> {
        let mut indications = Vec::new();
        let game = read_capture_file(&self.run, "game-root")?;
        let startup_error = read_capture_file(&self.run, "startup-error.txt")?;
        let setup = read_capture_file(&self.run, "setup.txt")?;
        let launcher_error = read_capture_file(&self.run, "launcher-error.txt")?;
        let exit = read_capture_file(&self.run, "exit-status.txt")?;
        let signal = read_capture_file(&self.run, "signal.txt")?;

        if let Some(error) = startup_error {
            indications.push(("Collector error", format!("{error} (startup-error.txt)")));
        }
        if game.is_none() {
            let message = if self.finished {
                "Armello was not found. Check the Steam launch command and game path."
            } else {
                "Checking the Steam game command."
            };
            indications.push(("Game path", message.to_string()));
        } else {
            indications.push((
                "Game path",
                "Armello and its Mono assemblies were found.".to_string(),
            ));
        }

        let setup_message = match setup.as_deref() {
            Some(message) if message.starts_with("Installed verified ") => {
                "Verified BepInEx files were installed.".to_string()
            }
            Some(message) if message.starts_with("Existing BepInEx installation detected") => {
                "Existing BepInEx found; its version was not checked.".to_string()
            }
            Some(message) => format!("Setup failed: {message} (setup.txt)"),
            None if self.finished && game.is_some() => {
                "No setup result recorded. Check the collector error above.".to_string()
            }
            None => "Not checked yet.".to_string(),
        };
        indications.push(("BepInEx", setup_message));

        if let Some(error) = launcher_error {
            indications.push((
                "Game process",
                format!("Launch or monitoring error: {error} (launcher-error.txt)"),
            ));
        } else if let Some(signal) = signal {
            indications.push((
                "Game process",
                format!("Terminated by OS signal {signal}. Check the logs and crash reports."),
            ));
        } else if let Some(exit) = exit {
            let code = exit.trim().parse::<i32>().map_err(io::Error::other)?;
            let detail = if code == 0 {
                "Exit status 0. This does not rule out errors in the logs.".to_string()
            } else if cfg!(windows) {
                format!("Exit status {code} (0x{:08X}, exit-status.txt). Check the logs and crash reports; the code alone does not establish a cause.", code as u32)
            } else {
                format!("Exit status {code} (exit-status.txt). Check the logs and crash reports; the code alone does not establish a cause.")
            };
            indications.push(("Game process", detail));
        } else if self.game_started && !self.game_stopped {
            indications.push((
                "Game process",
                "Started; no exit recorded. A running process may still be frozen.".to_string(),
            ));
        } else if self.finished {
            indications.push((
                "Game process",
                "Game did not start; no exit status recorded.".to_string(),
            ));
        } else {
            indications.push(("Game process", "Not started yet.".to_string()));
        }

        let (files, bytes) = count_files(&self.run.join("live"))?;
        let output_bytes = fs::metadata(self.run.join("launcher.log"))?.len();
        let activity = match self.last_game_output {
            Some(last) if !self.finished => format!(
                "Last new log output {} seconds ago.",
                last.elapsed().as_secs()
            ),
            Some(_) => "Log output was captured during this run.".to_string(),
            None => {
                "No new game log output observed; silence is not proof of a freeze.".to_string()
            }
        };
        indications.push(("Logs", format!("{files} files in live/ ({bytes} bytes), {output_bytes} bytes in launcher.log. {activity}")));

        let (_, bepinex_live) = count_files(&self.run.join("live/bepinex"))?;
        let (_, bepinex_collected) = count_files(&self.run.join("bepinex"))?;
        indications.push((
            "BepInEx output",
            if bepinex_live + bepinex_collected == 0 {
                "No new BepInEx log captured; installed files alone do not confirm it loaded."
                    .to_string()
            } else {
                "BepInEx log captured.".to_string()
            },
        ));

        let (game_live, _) = count_files(&self.run.join("live/game"))?;
        let (game_alt, _) = count_files(&self.run.join("live/game-alt"))?;
        let (game_collected, _) = count_files(&self.run.join("game"))?;
        let (unity_live, _) = count_files(&self.run.join("live/unity"))?;
        let (unity_collected, _) = count_files(&self.run.join("unity"))?;
        indications.push((
            "Game and Unity logs",
            format!(
                "{} game log copies, {} Unity log copies saved.",
                game_live + game_alt + game_collected,
                unity_live + unity_collected
            ),
        ));

        let (reports, _) = count_files(&self.run.join("crash"))?;
        if self.finished || reports > 0 {
            let message = if reports == 0 {
                "No OS crash report saved. Reports can arrive later; run --collect to check again."
                    .to_string()
            } else {
                format!("{reports} OS crash report files saved under crash/.")
            };
            indications.push(("OS reports", message));
        }
        Ok(indications)
    }

    fn draw(
        &self,
        ui: &mut egui::Ui,
        indications: &io::Result<Vec<(&'static str, String)>>,
    ) -> egui::Rect {
        let panel = egui::Panel::top("diagnostics")
            .default_size(300.0)
            .min_size(180.0)
            .max_size(420.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("Armello diagnostics");
                ui.label(format!("Status: {}", self.status));
                ui.label(format!("Capture: {}", self.run.display()));
                if let Some(last_update) = self.last_update {
                    ui.label(format!(
                        "Last update: {} seconds ago",
                        last_update.elapsed().as_secs()
                    ));
                }
                if self.finished {
                    ui.label("The collector finished. Close this window when you are done.");
                }
                ui.separator();
                ui.heading("Diagnostics");
                egui::ScrollArea::vertical()
                    .id_salt("diagnostic-checks")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match indications {
                        Ok(indications) => {
                            for (name, detail) in indications {
                                ui.label(format!("{name}: {detail}"));
                            }
                        }
                        Err(error) => {
                            ui.label(format!("Cannot inspect capture: {error}"));
                        }
                    });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Activity");
            ui.label(format!(
                "{} routine lines hidden from this view. Full output: session.log",
                self.feed.hidden
            ));
            egui::ScrollArea::vertical()
                .id_salt("live-output")
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for event in &self.feed.events {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(format!("+{:02}:{:02}", event.time / 60, event.time % 60));
                            let (level, color) = match event.level {
                                Level::Status => ("Event", ui.visuals().strong_text_color()),
                                Level::Warning => ("Warning", ui.visuals().warn_fg_color),
                                Level::Error => ("Error", ui.visuals().error_fg_color),
                            };
                            ui.label(egui::RichText::new(level).color(color));
                            ui.label(format!("{}: {}", event.source, event.message));
                            if event.occurrences > 1 {
                                ui.label(format!("(repeated {} times)", event.occurrences));
                            }
                        });
                    }
                });
        });
        panel.response.rect
    }
}

fn read_capture_file(run: &Path, name: &str) -> io::Result<Option<String>> {
    match fs::read_to_string(run.join(name)) {
        Ok(value) => Ok(Some(value.trim().to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn count_files(directory: &Path) -> io::Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    let mut directories = vec![directory.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                count += 1;
                bytes += entry.metadata()?.len();
            }
        }
    }
    Ok((count, bytes))
}

impl eframe::App for Diagnostics {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Err(error) = self.poll() {
            self.status = format!("Cannot read session log: {error}");
        }
        let indications = self.indications();
        self.draw(ui, &indications);
        ui.ctx().request_repaint_after(REFRESH_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::{egui, Diagnostics};
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reads_partial_live_lines_and_completion() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run =
            std::env::temp_dir().join(format!("armello-window-{}-{nonce}", std::process::id()));
        fs::create_dir(&run).unwrap();
        let log = run.join("session.log");
        fs::write(&log, b"[1] [collector] Game running\n[2] [stdout] parti").unwrap();
        let original = fs::read(&log).unwrap();

        let mut viewer = Diagnostics::new(run.clone());
        viewer.poll().unwrap();
        assert_eq!(viewer.status, "Game running");
        assert_eq!(viewer.feed.events.len(), 1);
        assert_eq!(fs::read(&log).unwrap(), original);
        fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"al\n[3] [collector] Capture directory: /tmp/capture\n")
            .unwrap();
        viewer.poll().unwrap();
        assert_eq!(viewer.feed.hidden, 1);
        assert!(viewer.finished);

        fs::remove_dir_all(run).unwrap();
    }

    #[test]
    fn distinguishes_process_failure_from_missing_logs() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run =
            std::env::temp_dir().join(format!("armello-checks-{}-{nonce}", std::process::id()));
        fs::create_dir(&run).unwrap();
        fs::write(run.join("launcher.log"), "").unwrap();
        let mut viewer = Diagnostics::new(run.clone());
        viewer.push("[1] [collector] Error: game executable not found".to_string());
        viewer.push(format!(
            "[2] [collector] Capture directory: {}",
            run.display()
        ));
        let checks = viewer.indications().unwrap();
        assert!(checks
            .iter()
            .any(|(name, detail)| *name == "Game path" && detail.contains("not found")));
        assert!(checks
            .iter()
            .any(|(name, detail)| *name == "BepInEx output" && detail.contains("No new")));

        fs::write(run.join("game-root"), "Armello").unwrap();
        fs::write(
            run.join("setup.txt"),
            "Existing BepInEx installation detected",
        )
        .unwrap();
        fs::write(run.join("exit-status.txt"), "1\n").unwrap();
        fs::create_dir(run.join("crash")).unwrap();
        fs::write(run.join("crash/armello.crash"), "report").unwrap();
        let checks = viewer.indications().unwrap();
        assert!(checks
            .iter()
            .any(|(name, detail)| *name == "Game process" && detail.contains("Exit status 1")));
        assert!(checks
            .iter()
            .any(|(name, detail)| *name == "OS reports" && detail.contains("1 OS crash report")));

        fs::remove_dir_all(run).unwrap();
    }

    #[test]
    fn diagnostics_stays_visible_with_a_full_log() {
        const BUSY_LOG_LINES: usize = 2000;
        let mut viewer = Diagnostics::new("capture".into());
        for index in 0..BUSY_LOG_LINES {
            viewer.push(format!("[{index}] [stderr] Warning: message {index}"));
        }
        let indications = Ok(vec![(
            "Game process",
            "Started; no exit recorded".to_string(),
        )]);
        egui::__run_test_ui(|ui| {
            let diagnostics = viewer.draw(ui, &indications);
            assert!(diagnostics.height() >= 180.0);
        });
    }
}
