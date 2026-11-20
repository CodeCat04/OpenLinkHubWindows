//! Finding a noisy fan: per-device speed overrides, a "spotlight" that lights chosen devices
//! white and darkens everything else, and an automatic test that walks through fans one by one.
//!
//! During the test every fan and pump runs at its quietest and goes dark. Each selected device
//! in turn lights up and runs Normal, then Max, then Quiet, for one phase each. Nothing here is
//! saved: speeds and lighting go back to how they were when the test ends.

use crate::hub::{manager, ManualSpeed};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::{Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(250);
/// Spin-down time at the start so every fan is already quiet when the first one is tested.
const SETTLE_SECS: u32 = 10;
pub const MIN_PHASE_SECS: u32 = 5;
pub const MAX_PHASE_SECS: u32 = 300;

// ---------------------------------------------------------------- spotlight

/// `None`: normal lighting. `Some(set)`: only these (hub serial, channel) are lit, in white.
static SPOTLIGHT: RwLock<Option<BTreeSet<(String, u8)>>> = RwLock::new(None);

/// Channels to light on this hub, or `None` when no spotlight is active.
pub fn spotlight(serial: &str) -> Option<BTreeSet<u8>> {
    let spot = SPOTLIGHT.read().unwrap();
    spot.as_ref().map(|set| set.iter().filter(|(s, _)| s == serial).map(|&(_, ch)| ch).collect())
}

fn spotlight_all() -> Option<BTreeSet<(String, u8)>> {
    SPOTLIGHT.read().unwrap().clone()
}

fn set_spotlight(v: Option<BTreeSet<(String, u8)>>) {
    *SPOTLIGHT.write().unwrap() = v;
}

/// Light one device up (everything else goes dark) or turn it back off. Not available during a test.
pub fn toggle_spotlight(serial: &str, channel: u8, on: bool) -> Result<(), String> {
    if running() {
        return Err("The fan test is running".into());
    }
    let mut spot = SPOTLIGHT.write().unwrap();
    let mut set = spot.take().unwrap_or_default();
    if on {
        set.insert((serial.to_string(), channel));
    } else {
        set.remove(&(serial.to_string(), channel));
    }
    *spot = (!set.is_empty()).then_some(set);
    Ok(())
}

/// For the dashboard: every lit device, or `None`.
pub fn spotlight_view() -> Option<Vec<Target>> {
    spotlight_all().map(|set| set.into_iter().map(|(serial, channel)| Target { serial, channel }).collect())
}

// ---------------------------------------------------------------- manual speeds

/// Put every fan and pump back on its curve.
pub fn clear_manual_speeds() -> Result<(), String> {
    if running() {
        return Err("The fan test is running".into());
    }
    for hub in manager::all() {
        hub.apply_manual_speeds(|_| None);
    }
    Ok(())
}

// ---------------------------------------------------------------- automatic test

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub serial: String,
    pub channel: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Everything quiet and dark, before the first device.
    Settle,
    Normal,
    Max,
    Quiet,
}

impl Phase {
    fn speed(self) -> ManualSpeed {
        match self {
            Phase::Normal => ManualSpeed::Normal,
            Phase::Max => ManualSpeed::Max,
            Phase::Settle | Phase::Quiet => ManualSpeed::Quiet,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: Phase,
    /// The device being tested (absent while settling).
    pub current: Option<Target>,
    /// 1-based position of `current` in `queue`.
    pub index: usize,
    pub queue: Vec<Target>,
    pub phase_secs: u32,
    /// Seconds left in this phase.
    pub remaining: u32,
}

struct Control {
    status: Option<Status>,
    stop: bool,
    skip: bool,
}

static CONTROL: Mutex<Control> = Mutex::new(Control { status: None, stop: false, skip: false });

pub fn running() -> bool {
    CONTROL.lock().unwrap().status.is_some()
}

pub fn status() -> Option<Status> {
    CONTROL.lock().unwrap().status.clone()
}

pub fn stop() {
    let mut c = CONTROL.lock().unwrap();
    if c.status.is_some() {
        c.stop = true;
    }
}

/// Move on to the next device.
pub fn skip() {
    let mut c = CONTROL.lock().unwrap();
    if c.status.is_some() {
        c.skip = true;
    }
}

pub fn start(queue: Vec<Target>, phase_secs: u32) -> Result<(), String> {
    if queue.is_empty() {
        return Err("Pick at least one fan to test".into());
    }
    if !(MIN_PHASE_SECS..=MAX_PHASE_SECS).contains(&phase_secs) {
        return Err(format!("Each step must be {MIN_PHASE_SECS}-{MAX_PHASE_SECS} seconds"));
    }
    if manager::all().iter().any(|h| h.is_critical()) {
        return Err("AIO coolant is too hot to run the fans slowly; wait for it to cool down".into());
    }
    for t in &queue {
        let hub = manager::get(&t.serial).ok_or("Hub not found (disconnected?)")?;
        if !hub.view().devices.iter().any(|d| d.channel_id == t.channel && d.speed_controllable()) {
            return Err("One of the chosen devices has no speed control".into());
        }
    }
    {
        let mut c = CONTROL.lock().unwrap();
        if c.status.is_some() {
            return Err("The fan test is already running".into());
        }
        *c = Control {
            status: Some(Status { phase: Phase::Settle, current: None, index: 0, queue: queue.clone(), phase_secs, remaining: SETTLE_SECS }),
            stop: false,
            skip: false,
        };
    }
    thread::Builder::new()
        .name("fan-test".into())
        .spawn(move || run(queue, phase_secs))
        .map(|_| ())
        .map_err(|e| {
            CONTROL.lock().unwrap().status = None;
            format!("Unable to start the fan test: {e}")
        })
}

/// What the test asks for: the device under test runs at `phase`, everything else quiet.
fn apply(current: Option<&Target>, phase: Phase) {
    for hub in manager::all() {
        let serial = hub.serial.clone();
        hub.apply_manual_speeds(|d| {
            let tested = current.is_some_and(|t| t.serial == serial && t.channel == d.channel_id);
            Some(if tested { phase.speed() } else { ManualSpeed::Quiet })
        });
    }
    set_spotlight(Some(current.map(|t| (t.serial.clone(), t.channel)).into_iter().collect()));
}

enum Wait {
    Done,
    Skip,
    Stop,
}

/// Hold one phase, re-applying it every tick (a hub that reconnects starts fresh).
fn hold(current: Option<&Target>, index: usize, phase: Phase, secs: u32) -> Wait {
    let until = Instant::now() + Duration::from_secs(secs as u64);
    loop {
        let left = until.saturating_duration_since(Instant::now());
        {
            let mut c = CONTROL.lock().unwrap();
            if c.stop {
                return Wait::Stop;
            }
            if std::mem::take(&mut c.skip) && current.is_some() {
                return Wait::Skip;
            }
            if let Some(s) = c.status.as_mut() {
                s.phase = phase;
                s.current = current.cloned();
                s.index = index;
                s.remaining = left.as_secs_f32().ceil() as u32;
            }
        }
        if left.is_zero() {
            return Wait::Done;
        }
        if manager::all().iter().any(|h| h.is_critical()) {
            log::warn!("Fan test stopped: AIO coolant is critically hot");
            return Wait::Stop;
        }
        apply(current, phase);
        thread::sleep(TICK.min(left));
    }
}

fn run(queue: Vec<Target>, phase_secs: u32) {
    log::info!("Fan test started: {} device(s), {phase_secs} s per step", queue.len());
    // Put back afterwards: speeds set by hand, and any device lit with "Light up".
    let saved: Vec<_> = manager::all().iter().map(|h| (h.serial.clone(), h.manual_speeds())).collect();
    let saved_spot = spotlight_all();

    let mut stopped = matches!(hold(None, 0, Phase::Settle, SETTLE_SECS.min(phase_secs)), Wait::Stop);
    'devices: for (i, target) in queue.iter().enumerate() {
        if stopped {
            break;
        }
        for phase in [Phase::Normal, Phase::Max, Phase::Quiet] {
            match hold(Some(target), i + 1, phase, phase_secs) {
                Wait::Done => {}
                Wait::Skip => continue 'devices,
                Wait::Stop => {
                    stopped = true;
                    break 'devices;
                }
            }
        }
    }

    for hub in manager::all() {
        let previous = saved.iter().find(|(s, _)| *s == hub.serial).map(|(_, v)| v.as_slice()).unwrap_or(&[]);
        hub.apply_manual_speeds(|d| previous.iter().find(|(ch, _)| *ch == d.channel_id).and_then(|(_, m)| *m));
    }
    set_spotlight(saved_spot);
    CONTROL.lock().unwrap().status = None;
    log::info!("Fan test {}", if stopped { "stopped" } else { "finished" });
}
