use std::collections::VecDeque;

const MAX_EVENTS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Status,
    Warning,
    Error,
}

pub struct Event {
    pub time: u64,
    pub source: &'static str,
    pub level: Level,
    pub message: String,
    pub occurrences: u32,
}

#[derive(Default)]
pub struct Feed {
    pub events: VecDeque<Event>,
    pub hidden: u64,
    started: Option<u64>,
}

pub struct SessionLine<'a> {
    pub time: u64,
    pub source: &'a str,
    pub message: &'a str,
}

impl<'a> SessionLine<'a> {
    pub fn parse(line: &'a str) -> Option<Self> {
        let (time, rest) = line.strip_prefix('[')?.split_once("] [")?;
        let (source, message) = rest.split_once("] ")?;
        Some(Self {
            time: time.parse().ok()?,
            source,
            message,
        })
    }
}

impl Feed {
    pub fn push(&mut self, line: &SessionLine<'_>) {
        let started = *self.started.get_or_insert(line.time);
        let Some((source, level, message)) = classify(line.source, line.message) else {
            self.hidden += 1;
            return;
        };
        let time = line.time.saturating_sub(started);
        if let Some(last) = self.events.back_mut() {
            if last.source == source && last.level == level && last.message == message {
                last.occurrences = last.occurrences.saturating_add(1);
                last.time = time;
                return;
            }
        }
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            time,
            source,
            level,
            message,
            occurrences: 1,
        });
    }
}

fn classify(source: &str, message: &str) -> Option<(&'static str, Level, String)> {
    if source == "collector" {
        return collector(message).map(|(level, message)| ("Collector", level, message));
    }
    let name = source.rsplit(['/', '\\']).next().unwrap_or(source);
    if name == "LogOutput.log" {
        return bepinex(message).map(|(level, message)| ("BepInEx", level, message));
    }
    let label = if name.starts_with("Player") && name.ends_with(".log") {
        "Unity"
    } else if name.ends_with(".txt") {
        "Armello"
    } else if source == "stderr" {
        "Game stderr"
    } else {
        "Game stdout"
    };
    game(message).map(|(level, message)| (label, level, message))
}

fn collector(message: &str) -> Option<(Level, String)> {
    if message.starts_with("Diagnostics window started")
        || message.starts_with("Collector active;")
        || message.starts_with("Close the diagnostics window")
    {
        return None;
    }
    let label = if message.starts_with("Game found:") {
        "Armello executable found".to_string()
    } else if message.starts_with("Capture directory:") {
        "Capture saved".to_string()
    } else if message.starts_with("Existing BepInEx installation detected") {
        "Existing BepInEx files found (version not checked)".to_string()
    } else {
        message.to_string()
    };
    let level = if message.starts_with("Error:")
        || message.starts_with("BepInEx setup failed:")
        || message.starts_with("Game launch or monitoring failed:")
    {
        Level::Error
    } else if message.starts_with("Live log read failed:")
        || message.starts_with("Cannot read ")
        || message.starts_with("Could not preserve ")
        || message.starts_with("Could not open diagnostics window:")
    {
        Level::Warning
    } else {
        Level::Status
    };
    Some((level, label))
}

fn bepinex(message: &str) -> Option<(Level, String)> {
    let (header, detail) = message.strip_prefix('[')?.split_once("] ")?;
    let (level, _) = header.split_once(':')?;
    match level.trim() {
        "Fatal" | "Error" => Some((Level::Error, detail.to_string())),
        "Warning" => Some((Level::Warning, detail.to_string())),
        "Message"
            if matches!(
                detail,
                "Preloader started"
                    | "Preloader finished"
                    | "Chainloader started"
                    | "Chainloader startup complete"
            ) =>
        {
            Some((Level::Status, detail.to_string()))
        }
        _ => None,
    }
}

fn game(message: &str) -> Option<(Level, String)> {
    let message = strip_game_time(message);
    for (prefix, level) in [
        ("Warning:", Level::Warning),
        ("Error:", Level::Error),
        ("Fatal:", Level::Error),
        ("Exception:", Level::Error),
    ] {
        if let Some(detail) = message.strip_prefix(prefix) {
            return Some((level, detail.trim().to_string()));
        }
    }
    if message
        .split_whitespace()
        .next()
        .is_some_and(|word| word.ends_with("Exception:"))
    {
        return Some((Level::Error, message.to_string()));
    }
    if let Some(scene) = message.strip_prefix("OnSceneLoaded: scene=") {
        return Some((
            Level::Status,
            format!("Scene: {}", scene.split(',').next()?.trim()),
        ));
    }
    None
}

fn strip_game_time(message: &str) -> &str {
    let Some(time) = message.get(..8) else {
        return message;
    };
    let bytes = time.as_bytes();
    if bytes[2] == b':'
        && bytes[5] == b':'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 2 || index == 5 || byte.is_ascii_digit())
    {
        message
            .strip_prefix(time)
            .and_then(|rest| rest.strip_prefix(": "))
            .unwrap_or(message)
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::{Feed, Level, SessionLine};

    #[test]
    fn filters_verbose_output_and_groups_repeated_warnings() {
        let mut feed = Feed::default();
        for line in [
            "[100] [collector] Diagnostics started",
            "[100] [stderr] MEMORY MAP(UnityPlayer.dylib)",
            "[101] [stderr]  0000000102da3000-0000000102da4000 r-x",
            "[102] [/game/logs/Armello.txt] 13:20:15: SessionId: private-id",
            "[103] [/game/logs/Armello.txt] 13:20:15: Warning: Missing font glyph",
            "[104] [/game/logs/Armello.txt] 13:20:15: Warning: Missing font glyph",
            "[105] [/game/BepInEx/LogOutput.log] [Message:   BepInEx] Chainloader startup complete",
            "[106] [/game/logs/Armello.txt] 13:20:16: OnSceneLoaded: scene=Main Menu, mode=Single",
            "[107] [stderr] NullReferenceException: object reference missing",
        ] {
            feed.push(&SessionLine::parse(line).unwrap());
        }
        assert_eq!(feed.hidden, 3);
        assert_eq!(feed.events.len(), 5);
        assert_eq!(feed.events[1].occurrences, 2);
        assert_eq!(feed.events[1].time, 4);
        assert_eq!(feed.events[2].source, "BepInEx");
        assert_eq!(feed.events[3].message, "Scene: Main Menu");
        assert_eq!(feed.events[4].level, Level::Error);
        assert!(!feed
            .events
            .iter()
            .any(|event| event.message.contains("private-id")));
    }

    #[test]
    fn does_not_treat_stderr_or_info_as_an_error() {
        let mut feed = Feed::default();
        for line in [
            "[100] [/game/BepInEx/LogOutput.log] [Info   :   BepInEx] Unity version found",
            "[101] [stderr] something happened",
            "[102] [collector] Collector active; game PID 42 running",
            "[103] [/game/logs/Armello.txt] 13:20:15: Error: Asset load failed",
        ] {
            feed.push(&SessionLine::parse(line).unwrap());
        }
        assert_eq!(feed.hidden, 3);
        assert_eq!(feed.events.len(), 1);
        assert_eq!(feed.events[0].source, "Armello");
        assert_eq!(feed.events[0].level, Level::Error);
        assert_eq!(feed.events[0].message, "Asset load failed");
    }

    #[test]
    fn reads_windows_paths_and_bepinex_log_levels() {
        let mut feed = Feed::default();
        for line in [
            "[100] [C:\\Games\\Armello\\BepInEx\\LogOutput.log] [Warning:   BepInEx] Loader warning",
            "[101] [C:\\Games\\Armello\\BepInEx\\LogOutput.log] [Error  :   BepInEx] Loader error",
            "[102] [C:\\Users\\Player\\AppData\\LocalLow\\Armello\\Player.log] NullReferenceException: failed",
        ] {
            feed.push(&SessionLine::parse(line).unwrap());
        }
        assert_eq!(feed.events[0].source, "BepInEx");
        assert_eq!(feed.events[0].level, Level::Warning);
        assert_eq!(feed.events[1].level, Level::Error);
        assert_eq!(feed.events[2].source, "Unity");
    }
}
