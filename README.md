# OpenLinkHub for Windows

Control Corsair **iCUE LINK** fans, AIOs, pump screens and lighting on Windows 10/11 without iCUE.

A small, open-source Windows app that runs in the system tray, with a dashboard window for fan curves, RGB effects, LCD screens and live temperatures. Its own code has **no network access at all**: no account, no telemetry, no update checks, no web server. It talks only to your hardware.

This is a Windows rewrite, in Rust, of the iCUE LINK support from **OpenLinkHub**, the Linux project by Nikola Jurkovic. The device protocol, fan-curve logic, lighting effects and LCD screens are ported from it.

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
| `dist\OpenLinkHubSensors.exe` | The optional CPU temperature service (installed by the installer). |

### 3. Install

1. **Quit or uninstall iCUE** (and any other RGB software, such as SignalRGB). Only one program can control the hub.
2. *Optional, for CPU temperature:* install the PawnIO driver (see [CPU temperature](#cpu-temperature-optional)).
3. Run **`dist\OpenLinkHub-Setup-<version>.exe`** and click through it like any other installer.

The installer:
- installs for your user account only, the same way VS Code and Discord install
- adds a Start menu shortcut, and optionally a desktop shortcut
- offers to start OpenLinkHub automatically when you sign in (recommended)
- offers the **CPU temperature sensor** service. This is the only part that asks for administrator permission, with one Windows prompt.
- registers in **Settings → Apps** for a normal uninstall

> Installers you build yourself don't trigger the "Windows protected your PC" SmartScreen warning. If you give a copy to someone else over the internet, they'll see it, because the app isn't code-signed. They can click **More info → Run anyway**.

## Using it

- **Open the dashboard:** double-click the tray icon, or use the Start menu. Starting OpenLinkHub again while it runs just brings the dashboard to the front.
- **Close the dashboard:** OpenLinkHub keeps running in the tray, and the dashboard's memory is released.
- **Switch profile:** the *Profile* switcher at the top of the dashboard, or right-click the tray icon. A profile puts every fan on its curve (pumps keep their own) and remembers its own lighting: whatever you change while a profile is active is saved with it, and comes back when you switch to it again. A profile you haven't used yet starts with your current lighting.
  - **Quiet, Normal and Performance** are always there. You can edit their curves on the *Fan curves* tab; *Reset to default* puts them back.
  - **Custom profiles:** every curve you create on the *Fan curves* tab is also a profile. Pick it from the *Custom* dropdown next to the switcher, or from the tray menu.
- **Fan speeds:** hover the fan button at the top right to see every fan's and pump's speed. Click it to keep it open; click again (or press Esc) to close it. The bulb next to each fan turns just that fan's lights off so you can spot it in the case; click again to turn them back on (they also come back when OpenLinkHub restarts).
- **Quit:** right-click the tray icon → *Quit*. The hub returns to its built-in (hardware) lighting and fan behaviour.

The dashboard has four tabs:

| Tab | What you can do |
|---|---|
| **Devices** | Live RPM, temperatures and duty for every device on each hub. Pick a fan curve and a lighting effect per device or for all at once, set brightness, turn lights off, give devices labels, set custom colors per device, choose what's connected to a LINK Adapter, and set up pump screens (**LCD** button). |
| **Fan curves** | Built-in curves (Quiet, Normal, Performance, Liquid Temperature, PSU), which you can edit and reset, plus your own. Drag points on the graph; a marker shows the live temperature. Curves can follow CPU, GPU, AIO coolant, a temperature probe, or a PSU. |
| **Lighting** | 27 effects (Static, Rainbow, Spiral Rainbow, Color Pulse, Color Shift, Wave, Rain, Nebula, Marquee, temperature-reactive, and more). Adjust speed and colors. |
| **Settings** | Start with Windows, sensor status, the log viewer, and Quit. |

### Which temperature should fans follow?

- **AIO pumps** default to *Liquid Temperature*, which uses the AIO's own coolant sensor.
- **Radiator fans:** *Liquid Temperature* is usually the best choice too. Coolant temperature changes smoothly, so fans don't surge with every short CPU spike.
- **Case fans** default to *Normal*, which follows the CPU. That needs the CPU temperature sensor below; without it they behave as if the CPU were at 50 °C.

### CPU temperature (optional)

Windows only lets kernel drivers read the CPU's temperature sensor. Every tool that shows CPU temperature (HWiNFO, Ryzen Master, LibreHardwareMonitor, iCUE) installs one. OpenLinkHub uses **PawnIO**, the open-source driver that LibreHardwareMonitor, Fan Control and OpenRGB also use. The driver is signed through Microsoft's hardware program and only accepts requests from Administrators and the system.

1. In an administrator PowerShell: `winget install namazso.PawnIO`
2. Run the OpenLinkHub installer with **CPU temperature sensor** ticked.

The sensor is a tiny separate service, **OpenLinkHub Sensors** (about 0.3 MB, about 5 MB of memory). Every 2 seconds it reads one CPU temperature register through PawnIO and shares the value with OpenLinkHub. It takes no input, writes no files and has no network code. It runs from `C:\Program Files\OpenLinkHub`, so non-admin programs can't modify it. Keeping it separate means OpenLinkHub itself never needs administrator rights. Uninstalling OpenLinkHub removes the service; PawnIO stays installed until you remove it in *Settings → Apps*.

Supported: AMD Ryzen, Threadripper and EPYC (Zen 1 to Zen 5; tested on a Ryzen 7 9800X3D) and Intel Core CPUs (implemented the same way as LibreHardwareMonitor, not yet tested on real hardware).

> Some game anti-cheat systems refuse to run alongside PawnIO; FACEIT is a known example. If that affects you, skip the CPU sensor; everything else works without it.

**GPU temperature** needs nothing extra on NVIDIA cards: it's read from the NVIDIA driver. Other GPUs aren't supported, and GPU-based curves then follow the CPU temperature.

## Supported hardware

Everything connected to an **iCUE LINK System Hub**:

- **Fans:** QX RGB, LX RGB, RX, RX RGB, RX MAX, RX RGB MAX
- **AIOs:** H100i, H115i, H150i, H170i, TITAN 240/280/360/420 (pump speed, coolant temperature and pump lighting)
- **Custom loop:** XC7 ELITE, XD5 ELITE, XD6 ELITE, XG3 HYBRID, XG7 RGB, generic AIO pump
- **LCD screens:**
  - the iCUE LINK LCD pump cap
  - XD5 ELITE LCD
  - Screens: liquid, pump, CPU and GPU temperatures, combined layouts, CPU/GPU load, clock, arc and animated layouts, and your own images and GIFs. Put image files in `%APPDATA%\OpenLinkHub\lcd`.
- **iCUE LINK Adapter:**
  - LS350/LS430 Aurora strips
  - 5000T and 9000D case lighting
  - QX, LL and 8-LED fans
  - RGB LED strips
  - the LC100 kit
  - Each connected part gets its own lighting effect.
- **Other:** iCUE COMMANDER DUO, VRM Cooler Module, HX1000i/HX1200i/HX1500i on LINK (fan, temperature and power draw), QX temperature probes

Several hubs are supported at once. Hubs and screens are detected automatically, including after unplugging them or waking from sleep.

**Not supported:**
- Corsair devices that aren't on a LINK hub (keyboards, mice, headsets, Commander Core/Pro, older AIOs, RAM lighting)
- OpenRGB integration. Its protocol requires a network port, which this app deliberately doesn't have.

LCD screens and the LINK Adapter are ported line-by-line from the Linux project, but haven't been tested on real hardware yet. Please open an issue if something doesn't look right.

## Built-in safety

Ported from the Linux project:

- **Coolant protection:** if AIO coolant goes above 57 °C, all fans and the pump run at 100% and the lights pulse red until it drops below 52 °C.
- **Minimum speeds:** the pump never goes below 50% on AIOs (31% on TITAN), and fans never go below 20% unless you enable *zero RPM* on a curve.
- **Port power protection:** lighting brightness is capped automatically when a hub port powers too many LEDs.
- **LINK Adapter check:** the adapter configuration is refused if the hub's channel table doesn't match the devices found, so the hub can't be misconfigured.
- **Clean hand-back:** on quit, update or uninstall, the hub and screens return to hardware mode so fans keep running on the hub's own control.

## Privacy and security

- **No network:** OpenLinkHub's own code has no network access, opens no ports and makes no connections. It talks to your hubs and screens over USB, and reads sensors from the NVIDIA driver and the optional sensor service.
- **Dashboard window:** the dashboard uses Windows' built-in WebView2 component. Pages are served from inside the app, never over a network.
  - Edge's SmartScreen lookups and background networking are switched off for it, and it can't navigate anywhere else.
  - WebView2's own diagnostic data follows your Windows privacy settings.
  - The only outside link is *Settings → Source code & help*, which opens this repository in your normal browser.
- **No admin needed:** OpenLinkHub itself runs as a normal user. Only the optional sensor service runs with system rights.
- **Your data:** settings live in `%APPDATA%\OpenLinkHub` (`config.json`, `hubs\*.json`, your curves, lighting and LCD images). The log is `%APPDATA%\OpenLinkHub\openlinkhub.log`. It shows only the last 4 characters of hub serial numbers, so it's safe to share when asking for help.

## Updating and uninstalling

- **Update:** in the `OpenLinkHubWindows` folder run `git pull` (or download the ZIP again), then `.\build.ps1`, and run the new installer. It stops the running copy, upgrades it in place, and keeps your settings. Tick *CPU temperature sensor* again to update the service too.
- **Uninstall:** *Settings → Apps → Installed apps → OpenLinkHub → Uninstall*.
  - This removes the program, shortcuts, the autostart entry and the sensor service (with one permission prompt).
  - It asks whether to also delete your settings.
  - PawnIO, if you installed it, is a separate entry in *Settings → Apps*.

## Troubleshooting

| Problem | Fix |
|---|---|
| `build.ps1` says Rust is not installed | Run `winget install Rustlang.Rustup`, then **open a new PowerShell window**. |
| Build error mentioning `link.exe`, `rc.exe` or `embed-resource` | The Visual Studio Build Tools C++ workload is missing. Run the Build Tools command from step 1. |
| Only `OpenLinkHub.exe` was built, with no installer | Inno Setup isn't installed. Run `winget install JRSoftware.InnoSetup` and build again. |
| "running scripts is disabled on this system" | Use the full command shown above: `powershell -ExecutionPolicy Bypass -File .\build.ps1`. |
| Dashboard window stays blank | The Microsoft Edge WebView2 Runtime is missing (rare; it's part of Windows 11). Install it with `winget install Microsoft.EdgeWebView2Runtime`. |
| "No iCUE LINK System Hub found" | Check the hub's USB cable to the motherboard. Make sure iCUE and other RGB software are fully closed (also in the tray). The hub is picked up within a few seconds once it's free. |
| "CPU temperature unavailable" | See *Settings* in the dashboard for the sensor status. Install PawnIO and the sensor service as described above, or switch fans to *Liquid Temperature*. |
| Lighting or fans stopped after sleep | Give it a few seconds: the hub is re-detected and set up again automatically. If not, quit and start OpenLinkHub from the Start menu, and check the log. |
| Something else | Open *Settings → Log* in the dashboard, or the log file, and open an issue on this repository with the relevant lines. |

## For developers

```powershell
cargo test                 # unit tests
cargo run --bin OpenLinkHub   # debug build with a console window for logs
.\build.ps1 -NoInstaller   # release executables only
```

The executables are fully self-contained: the dashboard, device data, fonts and images are embedded, and the C runtime is linked statically. The GitHub Actions *Windows* workflow runs clippy, the tests and a full build (executables and installer) on every push and pull request. It doesn't publish anything.

Command-line options:
- `OpenLinkHub.exe --background` starts without opening the dashboard (used by autostart).
- `OpenLinkHub.exe --stop` quits a running OpenLinkHub and waits for it (used by the installer).
- `OpenLinkHubSensors.exe install | uninstall | console` (from an administrator prompt) registers or removes the service, or prints readings for testing.

| Path | Contents |
|---|---|
| `src/hub/` | iCUE LINK System Hub protocol driver, device discovery, hot-plug, LINK Adapter |
| `src/lcd/` | LCD screens: rendering, images/GIFs, panel protocol |
| `src/rgb/` | Lighting engine and the 27 effects |
| `src/temperatures.rs`, `src/sensors.rs` | Fan curves; CPU, GPU and load readings |
| `src/server.rs`, `web/index.html` | The dashboard and its in-process API |
| `src/app.rs`, `src/main.rs` | Tray, dashboard window, single instance, autostart |
| `sensors/` | The CPU temperature service and the signed PawnIO modules it uses |
| `data/` | Device definitions, LINK Adapter layouts and default lighting (from the Linux project) |
| `assets/` | Icon, LCD font (SIL OFL), LCD background and animation |
| `installer/OpenLinkHub.iss` | Inno Setup installer script |
| `build.ps1` | One-step build of the apps and installer |

The original Linux (Go) application this was ported from remains in this repository's git history. Look at commits before the Windows rewrite if you want to port more devices.

## License

GPL-3.0-or-later, the same as the original OpenLinkHub project. See [LICENSE](LICENSE). Third-party files keep their own licenses: the Teko font is SIL OFL 1.1 (`assets/lcd/OFL.txt`), and the PawnIO modules are LGPL 2.1 (`sensors/pawnio/`).

Corsair and iCUE are trademarks of Corsair Gaming, Inc. This project isn't affiliated with or endorsed by Corsair.
