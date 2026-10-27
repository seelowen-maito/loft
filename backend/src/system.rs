//! This Mac's CPU temperature and memory, for the apps' headers. Sampled in the background
//! and cached, so requests never touch the sensors.
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
/// Or when memory used moves at least this much.
const NOTIFY_MEM_DELTA: u64 = 512 << 20;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Reading {
    /// Hottest CPU die sensor, °C. None where it can't be read.
    pub cpu_temp_c: Option<f64>,
    /// nominal | warm | hot, from the temperature (Apple Silicon throttles around 100 °C).
    pub level: Option<&'static str>,
    /// Active + wired + compressed pages, like Activity Monitor's "Memory Used".
    pub mem_used_bytes: Option<u64>,
    /// The machine's memory size.
    pub mem_total_bytes: Option<u64>,
    /// nominal | high (≥ 80 %) | critical (≥ 90 %), from memory used.
    pub mem_level: Option<&'static str>,
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

pub fn mem_level(used: u64, total: u64) -> &'static str {
    match used * 100 / total.max(1) {
        p if p >= 90 => "critical",
        p if p >= 80 => "high",
        _ => "nominal",
    }
}

/// Whether a new reading is worth pushing to connected clients.
pub fn changed(old: &Reading, new: &Reading) -> bool {
    let temp = match (old.cpu_temp_c, new.cpu_temp_c) {
        (Some(a), Some(b)) => (a - b).abs() >= NOTIFY_DELTA || old.level != new.level,
        (a, b) => a.is_some() != b.is_some(),
    };
    let mem = match (old.mem_used_bytes, new.mem_used_bytes) {
        (Some(a), Some(b)) => a.abs_diff(b) >= NOTIFY_MEM_DELTA || old.mem_level != new.mem_level,
        (a, b) => a.is_some() != b.is_some(),
    };
    temp || mem
}

/// Takes a reading and caches it. Returns it when clients should hear about it.
async fn refresh(state: &AppState) -> Option<Reading> {
    let (temp, mem) = tokio::task::spawn_blocking(|| (sensors::max_die_temp(), memory::used_total()))
        .await
        .unwrap_or_default();
    let reading = Reading {
        cpu_temp_c: temp.map(|t| (t * 10.0).round() / 10.0),
        level: temp.map(level),
        mem_used_bytes: mem.map(|m| m.0),
        mem_total_bytes: mem.map(|m| m.1),
        mem_level: mem.map(|(u, t)| mem_level(u, t)),
        sampled_at: Some(crate::db::now()),
    };
    let mut cur = state.system.0.lock().unwrap();
    let push = changed(&cur, &reading);
    *cur = reading.clone();
    push.then_some(reading)
}

pub async fn sampler(state: AppState) {
    // Without die sensors (Intel) a temperature read is a no-op, so memory still samples every 30 s.
    let mut tick = tokio::time::interval(INTERVAL);
    loop {
        tick.tick().await;
        if let Some(reading) = refresh(&state).await {
            state.emit(Event::System { system: json!(reading) });
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

#[cfg(target_os = "macos")]
mod memory {
    unsafe extern "C" {
        fn mach_host_self() -> libc::mach_port_t;
        fn host_statistics64(
            host: libc::mach_port_t,
            flavor: libc::c_int,
            info: *mut libc::c_int,
            count: *mut libc::mach_msg_type_number_t,
        ) -> libc::c_int;
    }

    /// (used, total) bytes. Used is active + wired + compressed pages.
    pub fn used_total() -> Option<(u64, u64)> {
        let mut total: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let ok = unsafe {
            libc::sysctlbyname(c"hw.memsize".as_ptr(), &mut total as *mut u64 as *mut _, &mut len, std::ptr::null_mut(), 0)
        };
        if ok != 0 || total == 0 {
            return None;
        }
        let mut vm: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let kr = unsafe { host_statistics64(mach_host_self(), libc::HOST_VM_INFO64, &mut vm as *mut _ as *mut _, &mut count) };
        if kr != 0 {
            return None;
        }
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        let pages = vm.active_count as u64 + vm.wire_count as u64 + vm.compressor_page_count as u64;
        Some(((pages * page).min(total), total))
    }
}

#[cfg(not(target_os = "macos"))]
mod memory {
    pub fn used_total() -> Option<(u64, u64)> {
        None
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
    fn mem_levels() {
        assert_eq!(mem_level(20, 32), "nominal");
        assert_eq!(mem_level(26, 32), "high");
        assert_eq!(mem_level(29, 32), "critical");
    }

    #[test]
    fn pushes_only_meaningful_changes() {
        let r = |t: Option<f64>| Reading { cpu_temp_c: t, level: t.map(level), ..Default::default() };
        assert!(!changed(&r(Some(60.0)), &r(Some(61.5))));
        assert!(changed(&r(Some(60.0)), &r(Some(62.0))));
        assert!(changed(&r(Some(84.5)), &r(Some(85.5))), "crossing into warm pushes even under 2 °C");
        assert!(changed(&r(None), &r(Some(60.0))));
        assert!(!changed(&r(None), &r(None)));

        const GB: u64 = 1 << 30;
        let m = |used: u64| Reading { mem_used_bytes: Some(used), mem_total_bytes: Some(32 * GB), mem_level: Some(mem_level(used, 32 * GB)), ..Default::default() };
        assert!(!changed(&m(20 * GB), &m(20 * GB + (100 << 20))));
        assert!(changed(&m(20 * GB), &m(21 * GB)));
        assert!(changed(&m(25 * GB + (600 << 20)), &m(25 * GB + (700 << 20))), "crossing 80 % pushes even under 512 MB");
    }

    #[tokio::test]
    async fn api_returns_memory_used_and_total() {
        let state = crate::tasks::tests::state();
        refresh(&state).await;
        let Json(v) = get(State(state)).await;
        if cfg!(target_os = "macos") {
            let total = v["mem_total_bytes"].as_u64().expect("total");
            let used = v["mem_used_bytes"].as_u64().expect("used");
            assert!(total > 0 && used <= total, "used {used} of {total}");
            assert!(v["mem_level"].is_string());
        }
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
