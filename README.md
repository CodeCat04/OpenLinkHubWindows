# OpenLinkHub for Windows

Control Corsair **iCUE LINK** fans, AIOs and lighting on Windows 10/11 without iCUE.

A small, open-source Windows app (a single 2 MB `.exe`) that runs in the system tray and gives you a local web dashboard for fan curves, RGB effects and live temperatures. It runs **entirely offline**: no account, no telemetry, no update checks, and nothing leaves your PC.

This is a Windows rewrite, in Rust, of the iCUE LINK support from **OpenLinkHub**, the Linux project by Nikola Jurkovic. The device protocol, fan-curve logic and lighting effects are ported from it.

---

## Install

1. **Quit or uninstall iCUE** (and any other RGB software, such as SignalRGB). Only one program can control the hub.
2. Download **`OpenLinkHub.exe`** from the [Releases page](https://github.com/CodeCat04/OpenLinkHubWindows/releases/latest).
3. Double-click it and choose **Yes** when asked to install.

That's it. OpenLinkHub copies itself to your user profile, starts automatically when you sign in, adds itself to the Start menu, and opens the dashboard. **No administrator rights are needed.**

> **"Windows protected your PC"?** The app isn't code-signed (certificates cost money), so SmartScreen warns about new downloads. Click **More info → Run anyway**. You can check the download against the `OpenLinkHub.exe.sha256` file on the Releases page.

**Prefer not to install?** Choose **No** at the prompt and it just runs from wherever you saved it. You won't be asked again.

**PowerShell one-liner** (downloads from this repository's Releases, verifies the checksum, installs):

```powershell
irm https://github.com/CodeCat04/OpenLinkHubWindows/releases/latest/download/install.ps1 | iex
```

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
- **Clean hand-back:** on quit, the hub returns to hardware mode so fans keep running on the hub's own control.

## Privacy and security

- **Offline:** OpenLinkHub talks only to your hubs over USB, and optionally to LibreHardwareMonitor on `127.0.0.1`. It refuses to connect to any address that isn't on your own PC, so it can't be pointed elsewhere even by mistake. There is no update check and no telemetry.
- **Local-only dashboard:** the dashboard listens on `127.0.0.1` only, and blocks requests from other websites. A page you visit in your browser can't change your fan speeds.
- **Your data:** settings live in `%APPDATA%\OpenLinkHub` (`config.json`, `hubs\*.json`, your curves and lighting). The log is `%APPDATA%\OpenLinkHub\openlinkhub.log`. It shows only the last 4 characters of hub serial numbers, so it's safe to share when asking for help.

## Updating and uninstalling

- **Update:** download the new `OpenLinkHub.exe` and double-click it. It offers to replace the installed copy and keeps your settings.
- **Uninstall:** *Settings → Apps → Installed apps → OpenLinkHub → Uninstall*. This removes the program, the autostart entry and the shortcut. To also remove your settings, delete `%APPDATA%\OpenLinkHub`.

## Troubleshooting

| Problem | Fix |
|---|---|
| "No iCUE LINK System Hub found" | Check the hub's USB cable to the motherboard. Make sure iCUE and other RGB software are fully closed (also in the tray). The hub is picked up within a few seconds once it's free. |
| Fans run at a fixed speed / "CPU temperature unavailable" | Set up LibreHardwareMonitor (above), or switch those fans to the *Liquid Temperature* curve. |
| Lighting or fans stopped after sleep | Give it a few seconds: the hub is re-detected and set up again automatically. If not, quit and start OpenLinkHub from the Start menu, and check the log. |
| Dashboard doesn't open | Make sure OpenLinkHub is running (tray icon), then go to `http://127.0.0.1:27003`. If another program uses port 27003, change `listenPort` in `%APPDATA%\OpenLinkHub\config.json`. |
| Something else | Open *Settings → Log* in the dashboard, or the log file, and open an issue on this repository with the relevant lines. |

## Building from source

Requirements: Rust (stable, installed via rustup) and Visual Studio Build Tools with the *Desktop development with C++* workload.

```powershell
cd rust
cargo test
cargo build --release      # → rust\target\release\OpenLinkHub.exe
```

The `.exe` is fully self-contained: the dashboard and device data are embedded, and the C runtime is linked statically.

Pushing a tag like `v0.2.0` runs the *Windows* GitHub Actions workflow, which tests, builds, and publishes `OpenLinkHub.exe`, its `.sha256` checksum and `install.ps1` as a release.

### Repository layout

| Path | Contents |
|---|---|
| `rust/` | **The Windows app.** `src/hub/` is the iCUE LINK protocol driver, `src/rgb/` the lighting effects, `src/temperatures.rs` the fan curves, `src/sensors.rs` the CPU/GPU temperatures, `src/app.rs` the tray, install and autostart, and `web/index.html` the dashboard. |
| `database/external/lsh.json`, `database/rgb.json` | Device definitions and default lighting settings, shared with the Linux project and embedded into the `.exe` at build time. |
| everything else (`src/`, `static/`, `web/`, `*.sh`, …) | The original Linux (Go) application, kept as the reference implementation for porting more devices. It isn't part of the Windows build. |

## License

GPL-3.0-or-later, the same as the original OpenLinkHub project. See [LICENSE](LICENSE).

Corsair and iCUE are trademarks of Corsair Gaming, Inc. This project isn't affiliated with or endorsed by Corsair.
