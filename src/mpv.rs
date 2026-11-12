use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Arc, Mutex, OnceLock};

#[repr(C)]
struct Handle(c_void);

#[repr(C)]
struct RawEvent {
    id: c_int,
    error: c_int,
    reply: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawLogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
}

struct Api {
    create: unsafe extern "C" fn() -> *mut Handle,
    initialize: unsafe extern "C" fn(*mut Handle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut Handle),
    set_option_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    set_property_async:
        unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int, *mut c_void) -> c_int,
    get_property_string: unsafe extern "C" fn(*mut Handle, *const c_char) -> *mut c_char,
    command_async: unsafe extern "C" fn(*mut Handle, u64, *const *const c_char) -> c_int,
    observe_property: unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int,
    wait_event: unsafe extern "C" fn(*mut Handle, f64) -> *mut RawEvent,
    free: unsafe extern "C" fn(*mut c_void),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    request_log_messages: unsafe extern "C" fn(*mut Handle, *const c_char) -> c_int,
}

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_LOG_MESSAGE: c_int = 2;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;
const END_EOF: c_int = 0;
const END_ERROR: c_int = 4;

#[cfg(windows)]
const LIB_NAMES: &[&str] = &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"];
#[cfg(target_os = "macos")]
const LIB_NAMES: &[&str] = &["libmpv.2.dylib", "libmpv.dylib"];
#[cfg(all(unix, not(target_os = "macos")))]
const LIB_NAMES: &[&str] = &["libmpv.so.2", "libmpv.so"];

pub fn preload() {
    let _ = api();
}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_owned()));
        let lib = LIB_NAMES
            .iter()
            .flat_map(|n| {
                exe_dir
                    .iter()
                    .map(move |d| d.join(n))
                    .chain(std::iter::once(n.into()))
            })
            .find_map(|p| libloading::Library::new(p).ok())
            .ok_or_else(|| {
                format!(
                    "libmpv not found: put {} next to the Plaivo executable",
                    LIB_NAMES[0]
                )
            })?;
        let lib: &'static libloading::Library = Box::leak(Box::new(lib));
        macro_rules! sym {
            ($n:literal) => {
                *lib.get(concat!($n, "\0").as_bytes())
                    .map_err(|e| format!("{}: {e}", $n))?
            };
        }
        Ok(Api {
            create: sym!("mpv_create"),
            initialize: sym!("mpv_initialize"),
            terminate_destroy: sym!("mpv_terminate_destroy"),
            set_option_string: sym!("mpv_set_option_string"),
            set_property_async: sym!("mpv_set_property_async"),
            get_property_string: sym!("mpv_get_property_string"),
            command_async: sym!("mpv_command_async"),
            observe_property: sym!("mpv_observe_property"),
            wait_event: sym!("mpv_wait_event"),
            free: sym!("mpv_free"),
            error_string: sym!("mpv_error_string"),
            request_log_messages: sym!("mpv_request_log_messages"),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

#[derive(Debug)]
pub enum Event {
    Time(f64),
    Duration(f64),
    Cached(f64),
    Pause(bool),
    Buffering(bool),
    Volume(f64),
    Mute(bool),
    Tracks,
    Loaded,
    Started,
    Ended { eof: bool, error: Option<String> },
}

const OBSERVED: &[(&str, c_int)] = &[
    ("time-pos", FORMAT_DOUBLE),
    ("duration", FORMAT_DOUBLE),
    ("demuxer-cache-time", FORMAT_DOUBLE),
    ("pause", FORMAT_FLAG),
    ("paused-for-cache", FORMAT_FLAG),
    ("volume", FORMAT_DOUBLE),
    ("mute", FORMAT_FLAG),
    ("track-list", FORMAT_STRING),
    ("audio-device-list", FORMAT_STRING),
];

/// Read only by the file info panel; observed as strings so `get` serves them from the cache without blocking the UI.
const INFO: &[&str] = &[
    "hwdec-current",
    "file-format",
    "video-format",
    "video-params/w",
    "video-params/h",
    "video-params/pixelformat",
    "container-fps",
    "video-bitrate",
    "audio-codec-name",
    "audio-params/hr-channels",
    "audio-params/samplerate",
    "audio-bitrate",
    "current-ao",
    "sid",
    "current-tracks/sub/title",
    "current-tracks/sub/lang",
    "current-tracks/sub/codec",
    "current-tracks/sub/forced",
];

struct Ctx {
    api: &'static Api,
    h: *mut Handle,
    cache: Mutex<HashMap<&'static str, String>>,
}

impl Ctx {
    fn read(&self, name: &str) -> Option<String> {
        unsafe {
            let p = (self.api.get_property_string)(self.h, cstr(name).as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.api.free)(p.cast());
            Some(s)
        }
    }

    fn store(&self, name: &'static str, value: Option<String>) {
        let mut cache = self.cache.lock().unwrap();
        match value {
            Some(v) => cache.insert(name, v),
            None => cache.remove(name),
        };
    }

    fn fetch(&self, name: &'static str) {
        self.store(name, self.read(name));
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        unsafe { (self.api.terminate_destroy)(self.h) };
    }
}

// libmpv's client API is thread-safe; the handle is freed only when the last owner drops it, normally the
// event thread after SHUTDOWN.
// Only the event thread makes blocking calls: on macOS mpv's video output waits on the main thread while it
// (re)configures, so a blocking call from the main thread can deadlock.
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}

pub struct Mpv(Arc<Ctx>);

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

impl Mpv {
    pub fn new(
        options: &[(&str, &str)],
        on_event: impl Fn(Event) + Send + 'static,
    ) -> Result<Self, String> {
        let api = api()?;
        let h = unsafe { (api.create)() };
        if h.is_null() {
            return Err("mpv_create failed".into());
        }
        let ctx = Arc::new(Ctx {
            api,
            h,
            cache: Mutex::default(),
        });
        for (k, v) in options {
            unsafe { (api.set_option_string)(h, cstr(k).as_ptr(), cstr(v).as_ptr()) };
        }
        log::info!("mpv: initializing with {} option(s)", options.len());
        let rc = unsafe { (api.initialize)(h) };
        if rc < 0 {
            return Err(format!("mpv_initialize: {}", error_string(api, rc)));
        }
        let info = INFO.iter().map(|name| (name, &FORMAT_STRING));
        for (i, (name, fmt)) in OBSERVED.iter().map(|(n, f)| (n, f)).chain(info).enumerate() {
            unsafe { (api.observe_property)(h, i as u64, cstr(name).as_ptr(), *fmt) };
        }
        let ev = ctx.clone();
        std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn(move || event_loop(ev, on_event))
            .map_err(|e| e.to_string())?;
        let mpv = Self(ctx);
        mpv.sync_log_level();
        Ok(mpv)
    }

    pub fn sync_log_level(&self) {
        let level = if log::log_enabled!(target: "mpv", log::Level::Debug) {
            "v"
        } else if log::log_enabled!(target: "mpv", log::Level::Info) {
            "info"
        } else {
            "warn"
        };
        unsafe { (self.0.api.request_log_messages)(self.0.h, cstr(level).as_ptr()) };
    }

    pub fn command(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        unsafe { (self.0.api.command_async)(self.0.h, 0, ptrs.as_ptr()) };
    }

    pub fn set(&self, name: &str, value: &str) {
        let value = cstr(value);
        let mut data = value.as_ptr();
        unsafe {
            (self.0.api.set_property_async)(
                self.0.h,
                0,
                cstr(name).as_ptr(),
                FORMAT_STRING,
                (&raw mut data).cast(),
            )
        };
    }

    pub fn get(&self, name: &str) -> Option<String> {
        self.0.cache.lock().unwrap().get(name).cloned()
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        self.command(&["quit"]);
    }
}

fn error_string(api: &Api, rc: c_int) -> String {
    unsafe {
        CStr::from_ptr((api.error_string)(rc))
            .to_string_lossy()
            .into_owned()
    }
}

fn event_loop(ctx: Arc<Ctx>, on_event: impl Fn(Event)) {
    loop {
        let e = unsafe { &*(ctx.api.wait_event)(ctx.h, -1.0) };
        match e.id {
            EVENT_SHUTDOWN => break,
            EVENT_LOG_MESSAGE => {
                let m = unsafe { &*(e.data as *const RawLogMessage) };
                let level = match m.log_level {
                    ..=20 => log::Level::Error,
                    21..=30 => log::Level::Warn,
                    31..=40 => log::Level::Info,
                    41..=60 => log::Level::Debug,
                    _ => log::Level::Trace,
                };
                let (prefix, text) = unsafe {
                    (
                        CStr::from_ptr(m.prefix).to_string_lossy(),
                        CStr::from_ptr(m.text).to_string_lossy(),
                    )
                };
                log::log!(target: "mpv", level, "[{prefix}] {}", text.trim_end());
            }
            EVENT_FILE_LOADED => {
                ctx.fetch("track-list");
                on_event(Event::Loaded);
            }
            EVENT_PLAYBACK_RESTART => {
                ctx.fetch("video-params/primaries");
                ctx.fetch("video-params/gamma");
                on_event(Event::Started);
            }
            EVENT_END_FILE => {
                let end = unsafe { &*(e.data as *const RawEndFile) };
                let error = (end.reason == END_ERROR).then(|| error_string(ctx.api, end.error));
                on_event(Event::Ended {
                    eof: end.reason == END_EOF,
                    error,
                });
            }
            EVENT_PROPERTY_CHANGE => {
                let p = unsafe { &*(e.data as *const RawProperty) };
                let name = unsafe { CStr::from_ptr(p.name) }.to_bytes();
                let double =
                    || (p.format == FORMAT_DOUBLE).then(|| unsafe { *(p.data as *const f64) });
                let flag =
                    || (p.format == FORMAT_FLAG).then(|| unsafe { *(p.data as *const c_int) } != 0);
                let string = || {
                    (p.format == FORMAT_STRING).then(|| {
                        unsafe { CStr::from_ptr(*(p.data as *const *const c_char)) }
                            .to_string_lossy()
                            .into_owned()
                    })
                };
                let ev = match name {
                    b"time-pos" => double().map(Event::Time),
                    b"duration" => double().map(Event::Duration),
                    b"demuxer-cache-time" => double().map(Event::Cached),
                    b"volume" => double().map(Event::Volume),
                    b"pause" => flag().map(Event::Pause),
                    b"paused-for-cache" => {
                        let f = flag();
                        ctx.store(
                            "paused-for-cache",
                            f.map(|b| if b { "yes" } else { "no" }.into()),
                        );
                        f.map(Event::Buffering)
                    }
                    b"mute" => flag().map(Event::Mute),
                    b"track-list" => {
                        ctx.store("track-list", string());
                        Some(Event::Tracks)
                    }
                    b"audio-device-list" => {
                        ctx.store("audio-device-list", string());
                        None
                    }
                    _ => {
                        if let Some(&key) = INFO.iter().find(|k| k.as_bytes() == name) {
                            ctx.store(key, string());
                        }
                        None
                    }
                };
                if let Some(ev) = ev {
                    on_event(ev);
                }
            }
            _ => {}
        }
    }
}
