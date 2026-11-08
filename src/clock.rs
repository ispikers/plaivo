#[cfg(windows)]
pub fn local_time_in(secs: f64) -> String {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToFileTime};
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        let mut ft: FILETIME = std::mem::zeroed();
        SystemTimeToFileTime(&st, &mut ft);
        let ticks = ((ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64) + (secs.max(0.0) * 1e7) as u64;
        let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
        FileTimeToSystemTime(&ft, &mut st);
        let mut buf = [0u16; 64];
        let n = GetTimeFormatEx(std::ptr::null(), TIME_NOSECONDS, &st, std::ptr::null(), buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
    }
}

#[cfg(not(windows))]
pub fn local_time_in(secs: f64) -> String {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        rest: [i32; 6],
        gmtoff: i64,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let t = now + secs.max(0.0) as i64;
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    unsafe { localtime_r(&t, &mut tm) };
    let h = match tm.hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h}:{:02} {}", tm.min, if tm.hour < 12 { "AM" } else { "PM" })
}
