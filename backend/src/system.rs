//! This Mac's CPU temperature, for the apps' headers. Sampled in the background and
//! cached, so requests never touch the sensors.
//!
//! Apple Silicon exposes its die sensors through the IOHID event system (the private API
//! tools like Stats and macmon use; no root needed). One reading of the die sensors costs a
//! few tens of milliseconds, so at one sample per `INTERVAL` it is a small fraction of one
//! core. Anywhere the API is missing (Intel, a future macOS) the temperature is just null.

use std::sync::Mutex;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use serde_json::{Value, json};

use crate::state::{AppState, Event};

pub const INTERVAL: Duration = Duration::from_secs(30);
/// Push an update to clients when the temperature moves at least this much.
const NOTIFY_DELTA: f64 = 2.0;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Reading {
    /// Hottest CPU die sensor, °C. None where it can't be read.
    pub cpu_temp_c: Option<f64>,
    /// nominal | warm | hot, from the temperature (Apple Silicon throttles around 100 °C).
    pub level: Option<&'static str>,
    pub sampled_at: Option<String>,
}

#[derive(Default)]
pub struct Cache(Mutex<Reading>);

impl Cache {
    pub fn get(&self) -> Reading {
        self.0.lock().unwrap().clone()
    }
}

pub fn level(c: f64) -> &'static str {
    match c {
        c if c >= 95.0 => "hot",
        c if c >= 85.0 => "warm",
        _ => "nominal",
    }
}

/// Whether a new reading is worth pushing to connected clients.
pub fn changed(old: &Reading, new: &Reading) -> bool {
    match (old.cpu_temp_c, new.cpu_temp_c) {
        (Some(a), Some(b)) => (a - b).abs() >= NOTIFY_DELTA || old.level != new.level,
        (a, b) => a.is_some() != b.is_some(),
    }
}

pub async fn sampler(state: AppState) {
    let mut tick = tokio::time::interval(INTERVAL);
    loop {
        tick.tick().await;
        let temp = tokio::task::spawn_blocking(sensors::max_die_temp).await.ok().flatten();
        let reading = Reading {
            cpu_temp_c: temp.map(|t| (t * 10.0).round() / 10.0),
            level: temp.map(level),
            sampled_at: Some(crate::db::now()),
        };
        let push = {
            let mut cur = state.system.0.lock().unwrap();
            let push = changed(&cur, &reading);
            *cur = reading.clone();
            push
        };
        if push {
            state.emit(Event::System { system: json!(reading) });
        }
        if temp.is_none() {
            // Nothing to read on this machine; check rarely instead of every 30 s.
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    }
}

/// `GET /api/system`: the last cached reading.
pub async fn get(State(state): State<AppState>) -> Json<Value> {
    Json(json!(state.system.get()))
}

#[cfg(target_os = "macos")]
mod sensors {
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;

    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFAllocatorDefault: CFTypeRef;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
        fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, len: CFIndex, enc: u32) -> bool;
        fn CFNumberCreate(alloc: CFTypeRef, kind: CFIndex, value: *const c_void) -> CFTypeRef;
        fn CFDictionaryCreate(
            alloc: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            n: CFIndex,
            kcb: *const c_void,
            vcb: *const c_void,
        ) -> CFTypeRef;
        fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> CFTypeRef;
        fn CFRelease(t: CFTypeRef);
        fn CFRetain(t: CFTypeRef) -> CFTypeRef;
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOHIDEventSystemClientCreate(alloc: CFTypeRef) -> CFTypeRef;
        fn IOHIDEventSystemClientSetMatching(client: CFTypeRef, matching: CFTypeRef) -> i32;
        fn IOHIDEventSystemClientCopyServices(client: CFTypeRef) -> CFTypeRef;
        fn IOHIDServiceClientCopyProperty(service: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn IOHIDServiceClientCopyEvent(service: CFTypeRef, kind: i64, options: i32, timestamp: i64) -> CFTypeRef;
        fn IOHIDEventGetFloatValue(event: CFTypeRef, field: i32) -> f64;
    }

    const UTF8: u32 = 0x0800_0100;
    const NUMBER_SINT32: CFIndex = 3;
    const EVENT_TEMPERATURE: i64 = 15;
    const FIELD_TEMPERATURE: i32 = (EVENT_TEMPERATURE as i32) << 16;

    /// The die sensors, found once: the service list doesn't change while running.
    struct Sensors(Vec<usize>);
    // Pointers to IOHID service clients, only ever used from one blocking thread at a time.
    unsafe impl Send for Sensors {}
    unsafe impl Sync for Sensors {}

    fn cfstr(s: &str) -> CFTypeRef {
        let c = std::ffi::CString::new(s).unwrap();
        unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), UTF8) }
    }

    fn string(t: CFTypeRef) -> Option<String> {
        if t.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 128];
        let ok = unsafe { CFStringGetCString(t, buf.as_mut_ptr(), buf.len() as CFIndex, UTF8) };
        unsafe { CFRelease(t) };
        ok.then(|| unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
    }

    fn die_sensors() -> &'static Sensors {
        static S: OnceLock<Sensors> = OnceLock::new();
        S.get_or_init(|| unsafe {
            let client = IOHIDEventSystemClientCreate(kCFAllocatorDefault);
            if client.is_null() {
                return Sensors(vec![]);
            }
            // Vendor usage page 0xff00, usage 5: temperature sensors.
            let (page, usage): (i32, i32) = (0xff00, 5);
            let keys = [cfstr("PrimaryUsagePage"), cfstr("PrimaryUsage")];
            let vals = [
                CFNumberCreate(kCFAllocatorDefault, NUMBER_SINT32, &page as *const i32 as *const c_void),
                CFNumberCreate(kCFAllocatorDefault, NUMBER_SINT32, &usage as *const i32 as *const c_void),
            ];
            let matching = CFDictionaryCreate(
                kCFAllocatorDefault,
                keys.as_ptr(),
                vals.as_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            IOHIDEventSystemClientSetMatching(client, matching);
            let services = IOHIDEventSystemClientCopyServices(client);
            let product = cfstr("Product");
            let mut out = vec![];
            if !services.is_null() {
                for i in 0..CFArrayGetCount(services) {
                    let s = CFArrayGetValueAtIndex(services, i);
                    let name = string(IOHIDServiceClientCopyProperty(s, product)).unwrap_or_default();
                    // CPU die sensors are named "PMU tdie<n>"; the client keeps them alive.
                    if name.contains("tdie") {
                        out.push(CFRetain(s) as usize);
                    }
                }
                CFRelease(services);
            }
            // `client` stays alive for the process: the services belong to it.
            Sensors(out)
        })
    }

    pub fn max_die_temp() -> Option<f64> {
        let sensors = die_sensors();
        sensors
            .0
            .iter()
            .filter_map(|&s| unsafe {
                let e = IOHIDServiceClientCopyEvent(s as CFTypeRef, EVENT_TEMPERATURE, 0, 0);
                if e.is_null() {
                    return None;
                }
                let t = IOHIDEventGetFloatValue(e, FIELD_TEMPERATURE);
                CFRelease(e);
                // Sensors that aren't powered report 0 or nonsense.
                (1.0..150.0).contains(&t).then_some(t)
            })
            .fold(None, |m: Option<f64>, t| Some(m.map_or(t, |m| m.max(t))))
    }
}

#[cfg(not(target_os = "macos"))]
mod sensors {
    pub fn max_die_temp() -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(level(60.0), "nominal");
        assert_eq!(level(85.0), "warm");
        assert_eq!(level(99.0), "hot");
    }

    #[test]
    fn pushes_only_meaningful_changes() {
        let r = |t: Option<f64>| Reading { cpu_temp_c: t, level: t.map(level), sampled_at: None };
        assert!(!changed(&r(Some(60.0)), &r(Some(61.5))));
        assert!(changed(&r(Some(60.0)), &r(Some(62.0))));
        assert!(changed(&r(Some(84.5)), &r(Some(85.5))), "crossing into warm pushes even under 2 °C");
        assert!(changed(&r(None), &r(Some(60.0))));
        assert!(!changed(&r(None), &r(None)));
    }

    /// On an Apple Silicon Mac this reads real sensors; elsewhere it just checks nothing panics.
    #[test]
    fn reads_a_plausible_temperature_or_none() {
        let t0 = std::time::Instant::now();
        let t = sensors::max_die_temp();
        let first = t0.elapsed();
        let t1 = std::time::Instant::now();
        let again = sensors::max_die_temp();
        let cost = t1.elapsed();
        if let Some(t) = t {
            assert!((10.0..120.0).contains(&t), "implausible die temperature {t}");
            assert!(again.is_some());
            eprintln!("die temp {t:.1} °C; first read {first:?} (finds sensors), later reads {cost:?}");
            assert!(cost < std::time::Duration::from_millis(500), "a reading took {cost:?}");
        }
    }
}
