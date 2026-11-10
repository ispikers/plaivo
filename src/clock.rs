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
        let ticks = ((ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64)
            + (secs.max(0.0) * 1e7) as u64;
        let ft = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        FileTimeToSystemTime(&ft, &mut st);
        let mut buf = [0u16; 64];
        let n = GetTimeFormatEx(
            std::ptr::null(),
            TIME_NOSECONDS,
            &st,
            std::ptr::null(),
            buf.as_mut_ptr(),
            buf.len() as i32,
        );
        String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
    }
}

#[cfg(target_os = "macos")]
pub fn local_time_in(secs: f64) -> String {
    use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle};
    let f = NSDateFormatter::new();
    f.setDateStyle(NSDateFormatterStyle::NoStyle);
    f.setTimeStyle(NSDateFormatterStyle::ShortStyle);
    f.stringFromDate(&NSDate::dateWithTimeIntervalSinceNow(secs.max(0.0)))
        .to_string()
}
