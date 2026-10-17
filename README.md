# OpenLinkHub for Windows

Control Corsair **iCUE LINK** fans, AIOs and lighting on Windows 10/11 without iCUE.

A small, open-source Windows app that runs in the system tray and gives you a local web dashboard for fan curves, RGB effects and live temperatures. It runs **entirely offline**: no account, no telemetry, no update checks, and nothing leaves your PC.

This is a Windows rewrite, in Rust, of the iCUE LINK support from **OpenLinkHub**, the Linux project by Nikola Jurkovic. The device protocol, fan-curve logic and lighting effects are ported from it.

---

## Install (build it yourself)

OpenLinkHub isn't distributed as a download. You build it from this source code, which takes about 10 minutes the first time, and you get a normal Windows installer.

### 1. One-time setup

Open **PowerShell** and install the build tools with `winget` (it's built into Windows 10/11):

```powershell
winget install Rustlang.Rustup
winget install JRSoftware.InnoSetup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Git.Git
```

- **Rust** is the language OpenLinkHub is written in.
- **Inno Setup** builds the installer.
- **Visual Studio Build Tools** (C++ workload) provide the Windows linker and resource compiler that Rust uses. This is a large download and asks for administrator approval.
- **Git** downloads the source code. You can skip it and use *Code → Download ZIP* on the repository page instead.

**Close and reopen PowerShell** afterwards so the new tools are on your `PATH`.

### 2. Get the source and build

```powershell
git clone https://github.com/CodeCat04/OpenLinkHubWindows.git
cd OpenLinkHubWindows
powershell -ExecutionPolicy Bypass -File .\build.ps1
```

The script runs the tests, builds the app and creates the installer in the `dist` folder:

| File | What it is |
|---|---|
| `dist\OpenLinkHub-Setup-<version>.exe` | **The installer.** Use this. |
| `dist\OpenLinkHub.exe` | The app on its own, if you'd rather run it portable without installing. |

### 3. Install

1. **Quit or uninstall iCUE** (and any other RGB software, such as SignalRGB). Only one program can control the hub.
2. Run **`dist\OpenLinkHub-Setup-<version>.exe`** and click through it like any other installer.

The installer:
- installs for your user account only, so **no administrator prompt**, the same way VS Code and Discord install
- adds a Start menu shortcut, and optionally a desktop shortcut
- offers to start OpenLinkHub automatically when you sign in (recommended)
- registers in **Settings → Apps** for a normal uninstall

> Installers you build yourself don't trigger the "Windows protected your PC" SmartScreen warning. If you give a copy to someone else over the internet, they'll see it, because the app isn't code-signed. They can click **More info → Run anyway**.

## Using it

- **Open the dashboard:** double-click the tray icon, use the Start menu, or go to `http://127.0.0.1:27003` in your browser.
- **Quit:** right-click the tray icon → *Quit*. The hub returns to its built-in (hardware) lighting and fan behaviour.

The dashboard has four tabs:

| Tab | What you can do |
|---|---|
| **Devices** | Live RPM, temperatures and duty for every device on each hub. Pick a fan curve and a lighting effect per device or for all at once, set brightness, turn lights off, give devices labels, and set custom colors per device. |
| **Fan curves** | Built-in curves (Quiet, Normal, Performance, Liquid Temperature) plus your own. Drag points on the graph; a marker shows the live temperature. Curves can follow CPU, GPU, AIO coolant, a temperature probe, a PSU, or any LibreHardwareMonitor sensor. |
| **Lighting** | 27 effects (Static, Rainbow, Spiral Rainbow, Color Pulse, Color Shift, Wave, Rain, Nebula, Marquee, temperature-reactive, and more). Adjust speed and colors. |
| **Settings** | Start with Windows, the LibreHardwareMonitor address, the log viewer, and Quit. |

### CPU temperature (optional, recommended)

Windows doesn't let normal programs read CPU temperature. OpenLinkHub reads it from **LibreHardwareMonitor**, a free open-source monitor, if it's running on the same PC:

1. Download LibreHardwareMonitor from its official GitHub releases page and unzip it anywhere.
2. Run `LibreHardwareMonitor.exe` **as administrator**.
3. In its menu, enable **Options → Remote Web Server → Run** (the default port 8085 is what OpenLinkHub expects).
4. Also tick **Options → Start Minimized**, **Minimize To Tray** and **Run On Windows Startup** so it's always available.

Without it:
- **GPU temperature** still works on NVIDIA cards (via the driver's `nvidia-smi`).
- **AIO pumps** default to the *Liquid Temperature* curve, which uses the AIO's own coolant sensor.
- **Fans on a CPU-based curve** behave as if the CPU were at 50 °C. The dashboard shows a reminder. You can also switch fans to *Liquid Temperature*, which is a good choice for radiator fans anyway.

## Supported hardware

Everything connected to an **iCUE LINK System Hub**:

- **Fans:** QX RGB, LX RGB, RX, RX RGB, RX MAX, RX RGB MAX
- **AIOs:** H100i, H115i, H150i, H170i, TITAN 240/280/360/420 (pump speed, coolant temperature and pump lighting)
- **Custom loop:** XC7 ELITE, XD5 ELITE, XD6 ELITE, XG3 HYBRID, XG7 RGB, generic AIO pump
- **Other:** iCUE COMMANDER DUO, VRM Cooler Module, HX1000i/HX1200i/HX1500i on LINK (fan, temperature and power draw), QX temperature probes

Several hubs are supported at once. Hubs are detected automatically, including after unplugging them or waking from sleep.

**Not supported yet** (the Linux project supports these, but they haven't been ported):
- the LCD screens on pump caps and XD5/XC7 blocks (the screens stay on their built-in display)
- LED strips attached through the iCUE LINK Adapter
- Corsair devices that aren't on a LINK hub (keyboards, mice, headsets, Commander Core/Pro, older AIOs, RAM)
- OpenRGB integration

## Built-in safety

Ported from the Linux project:

- **Coolant protection:** if AIO coolant goes above 57 °C, all fans and the pump run at 100% and the lights pulse red until it drops below 52 °C.
- **Minimum speeds:** the pump never goes below 50% on AIOs (31% on TITAN), and fans never go below 20% unless you enable *zero RPM* on a curve.
- **Port power protection:** lighting brightness is capped automatically when a hub port powers too many LEDs.
- **Clean hand-back:** on quit, update or uninstall, the hub returns to hardware mode so fans keep running on the hub's own control.

## Privacy and security

- **Offline:** OpenLinkHub talks only to your hubs over USB, and optionally to LibreHardwareMonitor on `127.0.0.1`. It refuses to connect to any address that isn't on your own PC, so it can't be pointed elsewhere even by mistake. There is no update check and no telemetry.
- **Local-only dashboard:** the dashboard listens on `127.0.0.1` only, and blocks requests from other websites. A page you visit in your browser can't change your fan speeds.
- **Your data:** settings live in `%APPDATA%\OpenLinkHub` (`config.json`, `hubs\*.json`, your curves and lighting). The log is `%APPDATA%\OpenLinkHub\openlinkhub.log`. It shows only the last 4 characters of hub serial numbers, so it's safe to share when asking for help.

## Updating and uninstalling

- **Update:** in the `OpenLinkHubWindows` folder run `git pull` (or download the ZIP again), then `.\build.ps1`, and run the new installer. It stops the running copy, upgrades it in place, and keeps your settings.
- **Uninstall:** *Settings → Apps → Installed apps → OpenLinkHub → Uninstall*. This removes the program, shortcuts and the autostart entry, and asks whether to also delete your settings.

## Troubleshooting

| Problem | Fix |
|---|---|
| `build.ps1` says Rust is not installed | Run `winget install Rustlang.Rustup`, then **open a new PowerShell window**. |
| Build error mentioning `link.exe`, `rc.exe` or `embed-resource` | The Visual Studio Build Tools C++ workload is missing. Run the Build Tools command from step 1. |
| Only `OpenLinkHub.exe` was built, with no installer | Inno Setup isn't installed. Run `winget install JRSoftware.InnoSetup` and build again. |
| "running scripts is disabled on this system" | Use the full command shown above: `powershell -ExecutionPolicy Bypass -File .\build.ps1`. |
| "No iCUE LINK System Hub found" | Check the hub's USB cable to the motherboard. Make sure iCUE and other RGB software are fully closed (also in the tray). The hub is picked up within a few seconds once it's free. |
| Fans run at a fixed speed / "CPU temperature unavailable" | Set up LibreHardwareMonitor (above), or switch those fans to the *Liquid Temperature* curve. |
| Lighting or fans stopped after sleep | Give it a few seconds: the hub is re-detected and set up again automatically. If not, quit and start OpenLinkHub from the Start menu, and check the log. |
| Dashboard doesn't open | Make sure OpenLinkHub is running (tray icon), then go to `http://127.0.0.1:27003`. If another program uses port 27003, change `listenPort` in `%APPDATA%\OpenLinkHub\config.json`. |
| Something else | Open *Settings → Log* in the dashboard, or the log file, and open an issue on this repository with the relevant lines. |

## For developers

```powershell
cargo test                 # unit tests
cargo run                  # debug build with a console window for logs
.\build.ps1 -NoInstaller   # release OpenLinkHub.exe only
```

The `.exe` is fully self-contained: the dashboard and device data are embedded, and the C runtime is linked statically. The GitHub Actions *Windows* workflow runs clippy, the tests and a full build (exe and installer) on every push and pull request. It doesn't publish anything.

Command-line options:
- `--background`: start without opening the dashboard (used by autostart)
- `--stop`: quit a running OpenLinkHub and wait for it (used by the installer)

| Path | Contents |
|---|---|
| `src/hub/` | iCUE LINK System Hub protocol driver, device discovery and hot-plug |
| `src/rgb/` | Lighting engine and the 27 effects |
| `src/temperatures.rs`, `src/sensors.rs` | Fan curves; CPU/GPU temperatures |
| `src/server.rs`, `web/index.html` | Local API and dashboard |
| `src/app.rs`, `src/main.rs` | Tray icon, autostart, startup |
| `data/` | Device definitions and default lighting settings (from the Linux project), embedded at build time |
| `installer/OpenLinkHub.iss` | Inno Setup installer script |
| `build.ps1` | One-step build of the app and installer |

The original Linux (Go) application this was ported from remains in this repository's git history. Look at commits before the Windows rewrite if you want to port more devices.

## License

GPL-3.0-or-later, the same as the original OpenLinkHub project. See [LICENSE](LICENSE).

Corsair and iCUE are trademarks of Corsair Gaming, Inc. This project isn't affiliated with or endorsed by Corsair.
