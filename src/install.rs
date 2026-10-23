use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path};
use std::time::Duration;

use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::assets::{self, Asset};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MONO_MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
const MONO_MAX_EXTRACTED_BYTES: u64 = 32 * 1024 * 1024;
const IL2CPP_MAX_ARCHIVE_BYTES: usize = 48 * 1024 * 1024;
const IL2CPP_MAX_EXTRACTED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FINGERPRINT_BYTES: u64 = 16 * 1024 * 1024;
const FINGERPRINT_BUFFER_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    Mono,
    Il2Cpp,
}

impl Runtime {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Il2Cpp => "IL2CPP",
        }
    }

    pub(crate) fn preloader(self) -> &'static str {
        match self {
            Self::Mono => "BepInEx/core/BepInEx.Preloader.dll",
            Self::Il2Cpp => "BepInEx/core/BepInEx.Unity.IL2CPP.dll",
        }
    }

    fn core(self) -> &'static str {
        match self {
            Self::Mono => "BepInEx/core/BepInEx.dll",
            Self::Il2Cpp => "BepInEx/core/BepInEx.Core.dll",
        }
    }

    fn limits(self) -> (usize, u64) {
        match self {
            Self::Mono => (MONO_MAX_ARCHIVE_BYTES, MONO_MAX_EXTRACTED_BYTES),
            Self::Il2Cpp => (IL2CPP_MAX_ARCHIVE_BYTES, IL2CPP_MAX_EXTRACTED_BYTES),
        }
    }
}

pub fn ensure(game: &Path, executable: &Path, runtime: Runtime) -> Result<String> {
    let asset = asset_for(executable, runtime)?;
    let loader = game.join(asset.loader);
    let mut required = vec![
        loader.clone(),
        game.join(runtime.preloader()),
        game.join(runtime.core()),
    ];
    if let Some(coreclr) = asset.coreclr {
        required.push(game.join(coreclr));
        required.push(game.join("dotnet/System.Private.CoreLib.dll"));
    }
    if runtime == Runtime::Il2Cpp && cfg!(windows) {
        required.push(game.join("doorstop_config.ini"));
    }
    if required.iter().all(|path| path.is_file()) {
        verify_existing(game, runtime, asset)?;
        return Ok(format!(
            "Existing verified BepInEx {} {} installation detected: {}",
            asset.release,
            runtime.label(),
            game.display()
        ));
    }
    if game.join("BepInEx").exists()
        || loader.exists()
        || (runtime == Runtime::Il2Cpp && game.join("dotnet").exists())
    {
        return Err(
            "An incomplete or different BepInEx installation exists; refusing to overwrite it"
                .into(),
        );
    }

    let url = format!(
        "https://github.com/BepInEx/BepInEx/releases/download/{}/{}",
        asset.release, asset.name
    );
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(90)))
        .build()
        .new_agent();
    let mut response = agent.get(&url).call()?;
    let mut bytes = Vec::new();
    let (max_archive_bytes, max_extracted_bytes) = runtime.limits();
    response
        .body_mut()
        .as_reader()
        .take(max_archive_bytes as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_archive_bytes {
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
        let relative = safe_entry(name, runtime)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("Archive contains a symbolic link: {name}").into());
        }
        total_size = total_size
            .checked_add(entry.size())
            .ok_or("Invalid archive size")?;
        if total_size > max_extracted_bytes {
            return Err("BepInEx archive exceeds the extracted size limit".into());
        }
        let destination = game.join(relative);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(format!("Refusing to overwrite {}", destination.display()).into());
        }
        paths.push(destination);
    }
    if !required.iter().all(|path| paths.contains(path)) {
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

fn verify_existing(game: &Path, runtime: Runtime, asset: &Asset) -> Result<()> {
    for (name, expected) in [
        (asset.loader, asset.loader_sha256),
        (runtime.preloader(), asset.preloader_sha256),
        (runtime.core(), asset.core_sha256),
    ] {
        let path = game.join(name);
        if fingerprint(&path)? != expected {
            return Err(format!(
                "Existing BepInEx installation does not match {} ({}); refusing to replace it",
                asset.release,
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

fn fingerprint(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_FINGERPRINT_BYTES {
        return Err(format!(
            "BepInEx file exceeds fingerprint size limit: {}",
            path.display()
        )
        .into());
    }
    let mut file = file.take(MAX_FINGERPRINT_BYTES + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; FINGERPRINT_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        total += size as u64;
        if total > MAX_FINGERPRINT_BYTES {
            return Err(format!(
                "BepInEx file exceeds fingerprint size limit: {}",
                path.display()
            )
            .into());
        }
        hash.update(&buffer[..size]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn safe_entry(name: &str, runtime: Runtime) -> Result<&Path> {
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
        Some("BepInEx") => match components.next().and_then(|part| part.as_os_str().to_str()) {
            Some("core") => {
                let entry = components.next();
                (runtime == Runtime::Il2Cpp && entry.is_none())
                    || (entry.is_some() && components.next().is_none())
            }
            Some("plugins" | "patchers") => {
                runtime == Runtime::Il2Cpp && components.next().is_none()
            }
            None => runtime == Runtime::Il2Cpp,
            _ => false,
        },
        Some("dotnet") => {
            let entry = components.next();
            runtime == Runtime::Il2Cpp && (entry.is_none() || components.next().is_none())
        }
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

fn asset_for(executable: &Path, runtime: Runtime) -> Result<&'static Asset> {
    #[cfg(target_os = "macos")]
    {
        if !executable.is_file() {
            return Err("Armello executable not found".into());
        }
        match runtime {
            Runtime::Mono => Ok(&assets::MONO_MACOS),
            Runtime::Il2Cpp => {
                macos_il2cpp_arch(executable)?;
                Ok(&assets::IL2CPP_MACOS_X64)
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut file = File::open(executable)?;
        let mut header = [0_u8; 64];
        file.read_exact(&mut header)?;
        #[cfg(target_os = "linux")]
        {
            let bits = elf_bits(&header)?;
            return match (runtime, bits) {
                (Runtime::Mono, 64) => Ok(&assets::MONO_LINUX_X64),
                (Runtime::Mono, 32) => Ok(&assets::MONO_LINUX_X86),
                (Runtime::Il2Cpp, 64) => Ok(&assets::IL2CPP_LINUX_X64),
                (Runtime::Il2Cpp, 32) => {
                    Err("BepInEx 6 IL2CPP has no native Linux x86 package".into())
                }
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
            return match (runtime, u16::from_le_bytes(pe[4..6].try_into()?)) {
                (Runtime::Mono, 0x8664) => Ok(&assets::MONO_WIN_X64),
                (Runtime::Mono, 0x14c) => Ok(&assets::MONO_WIN_X86),
                (Runtime::Il2Cpp, 0x8664) => Ok(&assets::IL2CPP_WIN_X64),
                (Runtime::Il2Cpp, 0x14c) => Ok(&assets::IL2CPP_WIN_X86),
                _ => Err("Unsupported Windows game architecture".into()),
            };
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_il2cpp_arch(executable: &Path) -> Result<()> {
    let mut file = std::fs::File::open(executable)?;
    let mut header = [0_u8; 8];
    file.read_exact(&mut header)?;
    const X86_64: u32 = 0x0100_0007;
    if &header[..4] == b"\xcf\xfa\xed\xfe" && u32::from_le_bytes(header[4..8].try_into()?) == X86_64
    {
        return Ok(());
    }
    let endian = match &header[..4] {
        b"\xca\xfe\xba\xbe" => Some(false),
        b"\xbe\xba\xfe\xca" => Some(true),
        _ => None,
    };
    if let Some(little) = endian {
        let number = if little {
            u32::from_le_bytes(header[4..8].try_into()?)
        } else {
            u32::from_be_bytes(header[4..8].try_into()?)
        };
        if number > 0 && number <= 16 {
            let mut x64 = false;
            for _ in 0..number {
                let mut arch = [0_u8; 20];
                file.read_exact(&mut arch)?;
                let cpu = if little {
                    u32::from_le_bytes(arch[..4].try_into()?)
                } else {
                    u32::from_be_bytes(arch[..4].try_into()?)
                };
                if cpu == X86_64 {
                    x64 = true;
                } else {
                    return Err(
                        "BepInEx 6 IL2CPP requires an x86_64-only macOS game executable".into(),
                    );
                }
            }
            if x64 {
                return Ok(());
            }
        }
    }
    Err("BepInEx 6 IL2CPP requires an x86_64-only macOS game executable".into())
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
    use super::{asset_for, ensure, safe_entry, verify_existing, Runtime};
    use crate::assets::Asset;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rejects_archive_paths_outside_the_sidecar() {
        for name in [
            "../Armello.exe",
            "/tmp/file",
            "armello.app/Contents/MacOS/Armello",
            "BepInEx/../Armello.exe",
            "BepInEx\\core\\evil.dll",
        ] {
            assert!(safe_entry(name, Runtime::Mono).is_err(), "accepted {name}");
            assert!(
                safe_entry(name, Runtime::Il2Cpp).is_err(),
                "accepted {name}"
            );
        }
        assert!(safe_entry("BepInEx/core/BepInEx.dll", Runtime::Mono).is_ok());
        assert!(safe_entry("dotnet/libcoreclr.so", Runtime::Mono).is_err());
        assert!(safe_entry("dotnet/libcoreclr.so", Runtime::Il2Cpp).is_ok());
        assert!(safe_entry("dotnet/", Runtime::Il2Cpp).is_ok());
        assert!(safe_entry("BepInEx/plugins/evil.dll", Runtime::Il2Cpp).is_err());
        assert!(safe_entry("dotnet/subdir/evil.dll", Runtime::Il2Cpp).is_err());
        assert!(safe_entry("BepInEx/plugins/", Runtime::Il2Cpp).is_ok());
        assert!(safe_entry("BepInEx/core/", Runtime::Il2Cpp).is_ok());
    }

    #[test]
    fn chooses_a_package_for_the_game_architecture() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let executable =
            std::env::temp_dir().join(format!("armello-arch-{}-{nonce}", std::process::id()));
        #[cfg(target_os = "macos")]
        {
            fs::write(&executable, b"\xcf\xfa\xed\xfe\x07\x00\x00\x01").unwrap();
            assert!(asset_for(&executable, Runtime::Mono)
                .unwrap()
                .name
                .contains("universal"));
            assert!(asset_for(&executable, Runtime::Il2Cpp)
                .unwrap()
                .name
                .contains("macos-x64"));
            fs::write(&executable, b"\xcf\xfa\xed\xfe\x0c\x00\x00\x01").unwrap();
            assert!(asset_for(&executable, Runtime::Il2Cpp).is_err());
        }
        #[cfg(target_os = "linux")]
        {
            let mut header = [0_u8; 64];
            header[..4].copy_from_slice(b"\x7fELF");
            header[4] = 2;
            header[5] = 1;
            header[18] = 62;
            fs::write(&executable, header).unwrap();
            assert!(asset_for(&executable, Runtime::Mono)
                .unwrap()
                .name
                .contains("linux_x64"));
            assert!(asset_for(&executable, Runtime::Il2Cpp)
                .unwrap()
                .name
                .contains("linux-x64"));
            header[4] = 1;
            header[18] = 3;
            fs::write(&executable, header).unwrap();
            assert!(asset_for(&executable, Runtime::Il2Cpp).is_err());
        }
        #[cfg(target_os = "windows")]
        {
            let mut header = [0_u8; 70];
            header[..2].copy_from_slice(b"MZ");
            header[60] = 64;
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
            fs::write(&executable, header).unwrap();
            assert!(asset_for(&executable, Runtime::Mono)
                .unwrap()
                .name
                .contains("win_x64"));
            assert!(asset_for(&executable, Runtime::Il2Cpp)
                .unwrap()
                .name
                .contains("win-x64"));
            header[68..70].copy_from_slice(&0x14c_u16.to_le_bytes());
            fs::write(&executable, header).unwrap();
            assert!(asset_for(&executable, Runtime::Il2Cpp)
                .unwrap()
                .name
                .contains("win-x86"));
        }
        fs::remove_file(executable).unwrap();
    }

    #[test]
    fn validates_existing_sidecars_without_replacing_them() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let game =
            std::env::temp_dir().join(format!("armello-sidecar-{}-{nonce}", std::process::id()));
        fs::create_dir_all(game.join("BepInEx/core")).unwrap();
        let executable = game.join("Armello");
        #[cfg(target_os = "macos")]
        fs::write(&executable, b"\xcf\xfa\xed\xfe\x07\x00\x00\x01").unwrap();
        #[cfg(target_os = "linux")]
        {
            let mut header = [0_u8; 64];
            header[..4].copy_from_slice(b"\x7fELF");
            header[4] = 2;
            header[5] = 1;
            header[18] = 62;
            fs::write(&executable, header).unwrap();
        }
        #[cfg(target_os = "windows")]
        {
            let mut header = [0_u8; 70];
            header[..2].copy_from_slice(b"MZ");
            header[60] = 64;
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
            fs::write(&executable, header).unwrap();
        }
        let loader = asset_for(&executable, Runtime::Mono).unwrap().loader;
        fs::write(game.join(loader), "loader").unwrap();
        fs::write(game.join("BepInEx/core/BepInEx.Preloader.dll"), "mono").unwrap();
        fs::write(game.join("BepInEx/core/BepInEx.dll"), "core").unwrap();
        let fixture = Asset {
            name: "fixture",
            sha256: "",
            loader,
            loader_sha256: "d47712cceb4c780603026e6325221c1bcff90679ebc076baa51c71ebe796717c",
            preloader_sha256: "d7de34b17b4691aa77ab8c17afdd440b004bc87d5d9ab991a8296d032fadb867",
            core_sha256: "0d45f5fd462b8c70bffb10021ac1bcff3f58f29b1faf7568595095427d42812c",
            release: "v5.4.23.5",
            coreclr: None,
        };
        verify_existing(&game, Runtime::Mono, &fixture).unwrap();
        let error = ensure(&game, &executable, Runtime::Mono)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not match v5.4.23.5"), "{error}");
        assert!(ensure(&game, &executable, Runtime::Il2Cpp).is_err());
        fs::write(game.join("BepInEx/core/BepInEx.Preloader.dll"), "newer").unwrap();
        assert!(verify_existing(&game, Runtime::Mono, &fixture).is_err());
        assert_eq!(fs::read_to_string(game.join(loader)).unwrap(), "loader");

        fs::remove_file(game.join("BepInEx/core/BepInEx.Preloader.dll")).unwrap();
        fs::remove_file(game.join("BepInEx/core/BepInEx.dll")).unwrap();
        fs::write(game.join("BepInEx/core/BepInEx.Unity.IL2CPP.dll"), "il2cpp").unwrap();
        fs::write(game.join("BepInEx/core/BepInEx.Core.dll"), "core").unwrap();
        fs::create_dir(game.join("dotnet")).unwrap();
        let coreclr = asset_for(&executable, Runtime::Il2Cpp)
            .unwrap()
            .coreclr
            .unwrap();
        fs::write(game.join(coreclr), "coreclr").unwrap();
        fs::write(game.join("dotnet/System.Private.CoreLib.dll"), "corlib").unwrap();
        #[cfg(windows)]
        fs::write(game.join("doorstop_config.ini"), "config").unwrap();
        assert!(ensure(&game, &executable, Runtime::Il2Cpp)
            .unwrap_err()
            .to_string()
            .contains("does not match v6.0.0-pre.2"));
        assert!(ensure(&game, &executable, Runtime::Mono).is_err());
        fs::remove_file(game.join("BepInEx/core/BepInEx.Core.dll")).unwrap();
        assert!(ensure(&game, &executable, Runtime::Il2Cpp)
            .unwrap_err()
            .to_string()
            .contains("incomplete or different"));
        fs::remove_dir_all(game).unwrap();
    }
}
