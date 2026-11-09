use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Arc, OnceLock};

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
    set_property_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    get_property_string: unsafe extern "C" fn(*mut Handle, *const c_char) -> *mut c_char,
    command: unsafe extern "C" fn(*mut Handle, *const *const c_char) -> c_int,
    observe_property: unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int,
    wait_event: unsafe extern "C" fn(*mut Handle, f64) -> *mut RawEvent,
    free: unsafe extern "C" fn(*mut c_void),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    request_log_messages: unsafe extern "C" fn(*mut Handle, *const c_char) -> c_int,
}

const FORMAT_NONE: c_int = 0;
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
            set_property_string: sym!("mpv_set_property_string"),
            get_property_string: sym!("mpv_get_property_string"),
            command: sym!("mpv_command"),
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
    ("track-list", FORMAT_NONE),
];

struct Ctx {
    api: &'static Api,
    h: *mut Handle,
}

// libmpv's client API is thread-safe; the handle is only freed by the event thread after SHUTDOWN.
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
        let ctx = Arc::new(Ctx { api, h });
        for (k, v) in options {
            unsafe { (api.set_option_string)(h, cstr(k).as_ptr(), cstr(v).as_ptr()) };
        }
        log::info!("mpv: initializing with {} option(s)", options.len());
        let rc = unsafe { (api.initialize)(h) };
        if rc < 0 {
            unsafe { (api.terminate_destroy)(h) };
            return Err(format!("mpv_initialize: {}", error_string(api, rc)));
        }
        for (i, (name, fmt)) in OBSERVED.iter().enumerate() {
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
        unsafe { (self.0.api.command)(self.0.h, ptrs.as_ptr()) };
    }

    pub fn set(&self, name: &str, value: &str) {
        unsafe {
            (self.0.api.set_property_string)(self.0.h, cstr(name).as_ptr(), cstr(value).as_ptr())
        };
    }

    pub fn get(&self, name: &str) -> Option<String> {
        unsafe {
            let p = (self.0.api.get_property_string)(self.0.h, cstr(name).as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.0.api.free)(p.cast());
            Some(s)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Build {
    pub mpv: String,
    pub ffmpeg: String,
    pub gpl: bool,
}

pub fn build_info() -> Result<Build, String> {
    let m = Mpv::new(&[("vo", "null"), ("ao", "null")], |_| {})?;
    let config = m.get("mpv-configuration").unwrap_or_default();
    Ok(Build {
        mpv: m.get("mpv-version").unwrap_or_default(),
        ffmpeg: m.get("ffmpeg-version").unwrap_or_default(),
        gpl: !(config.contains("-Dgpl=false") || config.contains("--enable-lgpl")),
    })
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
            EVENT_FILE_LOADED => on_event(Event::Loaded),
            EVENT_PLAYBACK_RESTART => on_event(Event::Started),
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
                let ev = match name {
                    b"time-pos" => double().map(Event::Time),
                    b"duration" => double().map(Event::Duration),
                    b"demuxer-cache-time" => double().map(Event::Cached),
                    b"volume" => double().map(Event::Volume),
                    b"pause" => flag().map(Event::Pause),
                    b"paused-for-cache" => flag().map(Event::Buffering),
                    b"mute" => flag().map(Event::Mute),
                    b"track-list" => Some(Event::Tracks),
                    _ => None,
                };
                if let Some(ev) = ev {
                    on_event(ev);
                }
            }
            _ => {}
        }
    }
    unsafe { (ctx.api.terminate_destroy)(ctx.h) };
}
