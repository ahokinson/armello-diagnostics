# Armello diagnostics

In [2022, League of Geeks described v2.2 as Armello's final content and feature update](https://steamcommunity.com/app/290340/discussions/0/3467225783136531505/). Some issues remain unresolved. This project collects crash evidence so players can investigate failures without guessing at a fix.

Steam launch prefix for Armello on macOS, Windows, and native Linux. It detects the Unity runtime, installs a SHA-256-checked BepInEx sidecar when needed, and saves logs after the game exits. Mono uses BepInEx 5.4.23.5; IL2CPP uses BepInEx 6.0.0-pre.2.

## Install

1. Download the ZIP for your operating system and processor from the [latest release](https://github.com/ahokinson/armello-diagnostics/releases/latest). Extract the binary into a user-owned directory outside the game installation. Windows uses `x86_64-pc-windows-msvc`, macOS Apple silicon uses `aarch64-apple-darwin`, and native Linux x64 uses `x86_64-unknown-linux-gnu`. On macOS or Linux, if your extraction tool did not preserve the executable bit, run `chmod u+x "<absolute path to collector binary>"`. On macOS, approve the downloaded binary in system settings if Gatekeeper blocks it; the executable bit and download quarantine are separate.
2. Open **Steam → Armello → Properties → General → Launch Options**. Save the current value. Set the option below, replacing the placeholder with the binary's absolute path. The Windows binary is named `armello-diagnostics.exe`.

   ```text
   "<absolute path to collector binary>" %command%
   ```

3. Launch Armello from Steam. The first run downloads the matching [BepInEx 5.4.23.5 Mono release](https://github.com/BepInEx/BepInEx/releases/tag/v5.4.23.5) or [BepInEx 6.0.0-pre.2 IL2CPP release](https://github.com/BepInEx/BepInEx/releases/tag/v6.0.0-pre.2), checks its pinned SHA-256, and installs sidecar files beside the game. Setup requires internet access. On later runs, the collector checks the existing loader, preloader, and core file against the pinned release offline. If one differs or is missing, setup stops without replacing it. On IL2CPP, the first game launch can take longer while BepInEx downloads Unity libraries and generates interop files.
4. Keep the Armello diagnostics window open during play. **Diagnostics** shows setup results, process status, log activity, and saved crash reports. **Activity** shows collector events, BepInEx startup milestones, scene changes, warnings, and errors. It hides routine lines and groups repeated messages. The window stays open after the game exits; close it when finished. Steam may show the collector as running until you close the window.
5. Open `captures/` beside the binary. `captures/current` names the latest run. Check `session.log` for the live feed and `startup-error.txt` if the game never launched. Check `runtime.txt`, `setup.txt`, `exit-status.txt`, `launcher.log`, and files under `bepinex/` when present. Record the action and time of a crash.

If Armello freezes, note the time. The collector saves new BepInEx, game, and Unity log bytes under `live/` while the game is running. It also watches for BepInEx preloader failure logs beside the game. Force-quit Armello, then let the collector finish. If the collector also stops, `session.log`, `launcher.log`, and `live/` still contain the data written before it stopped. A running game process and a quiet log do not prove the game is responsive.

The Diagnostics section reports only observed results. An exit code, a missing log, or an error line does not establish why Armello failed. On Unix, `signal.txt` records an OS termination signal when available. Crash reports can arrive late; run `--collect` to check again.

If a crash report arrives after the game exits, run the collector binary with `--collect`. On Windows, a dialog confirms the capture directory.

If the diagnostics window cannot open, check `window-error.txt` in the latest capture. The collector still writes `session.log` and collects crash evidence.

**Keep captures private.** Game logs can contain account IDs, session tickets, and service credentials. Redact them before sharing. The collector does not upload files.

The window formats the display only. `session.log`, `launcher.log`, and copied logs retain their original contents. Open `session.log` to see lines hidden from Activity. Warnings and errors are log labels, not proven causes. The window can still show account details in log messages; check it before taking screenshots or streaming.

## Scope and rollback

- Setup adds BepInEx sidecar files beside `armello.app` or the game executable. It refuses to overwrite existing files. It does not edit the game's executable or managed assemblies.
- The game must include Mono's `Assembly-CSharp.dll` or IL2CPP's `il2cpp_data/Metadata/global-metadata.dat`. Native Linux is supported; Proton is not. IL2CPP packages are available for Windows x86/x64, native Linux x64, and macOS x64. BepInEx 6.0.0-pre.2 does not provide a macOS arm64 IL2CPP package. On Apple silicon, the macOS x64 build runs through Rosetta.
- IL2CPP setup adds a `dotnet/` runtime beside the game. BepInEx 6 plugins are not compatible with BepInEx 5 plugins. Existing installations must match the pinned release; the collector does not overwrite other versions or custom builds.
- If setup fails, inspect `setup.txt` in the newest capture. Extraction errors may leave new sidecar files behind.
- Restore the saved Steam Launch Options value to disable the prefix. Check ownership of any BepInEx sidecar files before removing them.

## Build

Run `cargo test --locked && cargo build --release --locked` on the target operating system. The binary is at `target/release/armello-diagnostics` (`.exe` on Windows). Run the [build workflow](.github/workflows/build.yml) on the default `develop` branch to test all three platforms and publish their ZIPs as a public GitHub Release. The release also includes `SHA256SUMS`. Actions artifacts remain available for debugging the workflow, but public downloads use the release page.
