use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path};
use std::time::Duration;

#[cfg(not(target_os = "macos"))]
use std::fs::File;

use sha2::{Digest, Sha256};
use zip::ZipArchive;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 32 * 1024 * 1024;

struct Asset {
    name: &'static str,
    sha256: &'static str,
    loader: &'static str,
}

pub fn ensure(game: &Path, executable: &Path) -> Result<String> {
    let asset = asset_for(executable)?;
    let preloader = game.join("BepInEx/core/BepInEx.Preloader.dll");
    let loader = game.join(asset.loader);
    if loader.is_file() && preloader.is_file() {
        return Ok(format!(
            "Existing BepInEx installation detected (version not checked): {}",
            game.display()
        ));
    }
    if game.join("BepInEx").exists() || loader.exists() {
        return Err(
            "An incomplete or different BepInEx installation exists; refusing to overwrite it"
                .into(),
        );
    }

    let url = format!(
        "https://github.com/BepInEx/BepInEx/releases/download/v5.4.23.5/{}",
        asset.name
    );
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(90)))
        .build()
        .new_agent();
    let mut response = agent.get(&url).call()?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("BepInEx download exceeded the archive size limit".into());
    }
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != asset.sha256 {
        return Err(format!("BepInEx SHA-256 mismatch for {}", asset.name).into());
    }

    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut total_size = 0_u64;
    let mut paths = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name();
        let relative = safe_entry(name)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("Archive contains a symbolic link: {name}").into());
        }
        total_size = total_size
            .checked_add(entry.size())
            .ok_or("Invalid archive size")?;
        if total_size > MAX_EXTRACTED_BYTES {
            return Err("BepInEx archive exceeds the extracted size limit".into());
        }
        let destination = game.join(relative);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(format!("Refusing to overwrite {}", destination.display()).into());
        }
        paths.push(destination);
    }
    if !paths.iter().any(|path| path == &loader) || !paths.iter().any(|path| path == &preloader) {
        return Err("BepInEx archive is missing required files".into());
    }

    for (index, destination) in paths.iter().enumerate() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            fs::create_dir_all(destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        io::copy(&mut entry, &mut output)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let launcher = game.join("run_bepinex.sh");
        fs::set_permissions(launcher, fs::Permissions::from_mode(0o700))?;
    }

    Ok(format!("Installed verified {} beside the game", asset.name))
}

fn safe_entry(name: &str) -> Result<&Path> {
    if name.contains('\\') {
        return Err(format!("Invalid archive path: {name}").into());
    }
    let path = Path::new(name);
    if path.components().count() == 0
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!("Invalid archive path: {name}").into());
    }
    let mut components = path.components();
    let first = components.next().ok_or("Empty archive path")?;
    let allowed = match first.as_os_str().to_str() {
        Some("BepInEx") => components
            .next()
            .is_some_and(|part| part.as_os_str() == "core"),
        Some(
            ".doorstop_version"
            | "changelog.txt"
            | "libdoorstop.dylib"
            | "libdoorstop.so"
            | "run_bepinex.sh"
            | "doorstop_config.ini"
            | "winhttp.dll",
        ) => components.next().is_none(),
        _ => false,
    };
    if !allowed {
        return Err(format!("Unexpected archive path: {name}").into());
    }
    Ok(path)
}

fn asset_for(executable: &Path) -> Result<Asset> {
    #[cfg(target_os = "macos")]
    {
        if !executable.is_file() {
            return Err("Armello executable not found".into());
        }
        Ok(Asset {
            name: "BepInEx_macos_universal_5.4.23.5.zip",
            sha256: "01c2ae782eb016dfd6c345a18dbd2dcafffb3d9d318449d6486689f426b4a323",
            loader: "libdoorstop.dylib",
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut file = File::open(executable)?;
        let mut header = [0_u8; 64];
        file.read_exact(&mut header)?;
        #[cfg(target_os = "linux")]
        {
            let bits = elf_bits(&header)?;
            return match bits {
                64 => Ok(Asset {
                    name: "BepInEx_linux_x64_5.4.23.5.zip",
                    sha256: "e538560be65739f562519ab518a75f9c65b3f57f87457403ae7cde683c12dab7",
                    loader: "libdoorstop.so",
                }),
                32 => Ok(Asset {
                    name: "BepInEx_linux_x86_5.4.23.5.zip",
                    sha256: "b4a815b33635921faedeada0df00ba5436b4441d9e16bb13f4bc8c7ad4edde63",
                    loader: "libdoorstop.so",
                }),
                _ => Err("Unsupported Linux game architecture".into()),
            };
        }
        #[cfg(target_os = "windows")]
        {
            let offset = usize::try_from(u32::from_le_bytes(header[60..64].try_into()?))?;
            let mut pe = [0_u8; 6];
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(offset as u64))?;
            file.read_exact(&mut pe)?;
            if &header[..2] != b"MZ" || &pe[..4] != b"PE\0\0" {
                return Err("The game is not a Windows PE executable".into());
            }
            return match u16::from_le_bytes(pe[4..6].try_into()?) {
                0x8664 => Ok(Asset {
                    name: "BepInEx_win_x64_5.4.23.5.zip",
                    sha256: "82f9878551030f54657792c0740d9d51a09500eeae1fba21106b0c441e6732c4",
                    loader: "winhttp.dll",
                }),
                0x14c => Ok(Asset {
                    name: "BepInEx_win_x86_5.4.23.5.zip",
                    sha256: "37651c79e40d6f909572a4f461ac25350bb3ef8fe7fbd29f1aa8791a33b84c82",
                    loader: "winhttp.dll",
                }),
                _ => Err("Unsupported Windows game architecture".into()),
            };
        }
    }
}

#[cfg(target_os = "linux")]
fn elf_bits(header: &[u8; 64]) -> Result<u8> {
    if &header[..4] != b"\x7fELF" || header[5] != 1 {
        return Err("Native Linux ELF game required (not Proton)".into());
    }
    match (header[4], u16::from_le_bytes([header[18], header[19]])) {
        (2, 62) => Ok(64),
        (1, 3) => Ok(32),
        _ => Err("Unsupported Linux game architecture".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::safe_entry;

    #[test]
    fn rejects_archive_paths_outside_the_sidecar() {
        for name in [
            "../Armello.exe",
            "/tmp/file",
            "armello.app/Contents/MacOS/Armello",
            "BepInEx/../Armello.exe",
            "BepInEx\\core\\evil.dll",
        ] {
            assert!(safe_entry(name).is_err(), "accepted {name}");
        }
        assert!(safe_entry("BepInEx/core/BepInEx.dll").is_ok());
    }
}
