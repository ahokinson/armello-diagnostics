# Armello diagnostics

In [2022, League of Geeks described v2.2 as Armello's final content and feature update](https://steamcommunity.com/app/290340/discussions/0/3467225783136531505/). Some issues remain unresolved. This project collects crash evidence so players can investigate failures without guessing at a fix.

Steam launch prefix for Armello on macOS, Windows, and native Linux. It installs a verified BepInEx 5.4.23.5 sidecar when needed and saves logs after the game exits.

## Install

1. Download the ZIP for your operating system and processor from the [latest release](https://github.com/ahokinson/armello-diagnostics/releases/latest). Extract the binary into a user-owned directory outside the game installation. Windows uses `x86_64-pc-windows-msvc`, macOS Apple silicon uses `aarch64-apple-darwin`, and native Linux x64 uses `x86_64-unknown-linux-gnu`. On macOS or Linux, run `chmod u+x "<absolute path to collector binary>"`.
2. Open **Steam → Armello → Properties → General → Launch Options**. Save the current value. Set the option below, replacing the placeholder with the binary's absolute path. The Windows binary is named `armello-diagnostics.exe`.

   ```text
   "<absolute path to collector binary>" %command%
   ```

3. Launch Armello from Steam. The first run downloads the matching [BepInEx 5.4.23.5 release](https://github.com/BepInEx/BepInEx/releases/tag/v5.4.23.5), checks its published SHA-256, and installs sidecar files beside the game. Setup requires internet access. Existing BepInEx installations are left in place; their version is not checked.
4. Keep the Armello diagnostics window open during play. **Diagnostics** shows setup results, process status, log activity, and saved crash reports. **Activity** shows collector events, BepInEx startup milestones, scene changes, warnings, and errors. It hides routine lines and groups repeated messages. The window stays open after the game exits; close it when finished. Steam may show the collector as running until you close the window.
5. Open `captures/` beside the binary. `captures/current` names the latest run. Check `session.log` for the live feed and `startup-error.txt` if the game never launched. Check `setup.txt`, `exit-status.txt`, `launcher.log`, and `bepinex/LogOutput.log` when present. Record the action and time of a crash.

If Armello freezes, note the time. The collector saves new BepInEx, game, and Unity log bytes under `live/` while the game is running. Force-quit Armello, then let the collector finish. If the collector also stops, `session.log`, `launcher.log`, and `live/` still contain the data written before it stopped. A running game process and a quiet log do not prove the game is responsive.

The Diagnostics section reports only observed results. An exit code, a missing log, or an error line does not establish why Armello failed. On Unix, `signal.txt` records an OS termination signal when available. Crash reports can arrive late; run `--collect` to check again.

If a crash report arrives after the game exits, run the collector binary with `--collect`. On Windows, a dialog confirms the capture directory.

If the diagnostics window cannot open, check `window-error.txt` in the latest capture. The collector still writes `session.log` and collects crash evidence.

**Keep captures private.** Game logs can contain account IDs, session tickets, and service credentials. Redact them before sharing. The collector does not upload files.

The window formats the display only. `session.log`, `launcher.log`, and copied logs retain their original contents. Open `session.log` to see lines hidden from Activity. Warnings and errors are log labels, not proven causes. The window can still show account details in log messages; check it before taking screenshots or streaming.

## Scope and rollback

- Setup adds BepInEx sidecar files beside `armello.app` or the game executable. It refuses to overwrite existing files. It does not edit the game's executable or managed assemblies.
- The game must include Unity Mono's `Assembly-CSharp.dll`. Native Linux is supported; Proton and IL2CPP are not.
- If setup fails, inspect `setup.txt` in the newest capture. Extraction errors may leave new sidecar files behind.
- Restore the saved Steam Launch Options value to disable the prefix. Check ownership of any BepInEx sidecar files before removing them.

## Build

Run `cargo test --locked && cargo build --release --locked` on the target operating system. The binary is at `target/release/armello-diagnostics` (`.exe` on Windows). Run the [build workflow](.github/workflows/build.yml) on the default `develop` branch to test all three platforms and publish their ZIPs as a public GitHub Release. The release also includes `SHA256SUMS`. Actions artifacts remain available for debugging the workflow, but public downloads use the release page.
