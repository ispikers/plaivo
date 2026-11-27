#[cfg(windows)]
pub fn local_time_in(secs: f64) -> String {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let mut ft = FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    let ticks =
        ((ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64) + (secs.max(0.0) * 1e7) as u64;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    unsafe { FileTimeToSystemTime(&ft, &mut utc) };
    let mut st = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut st) };
    let mut buf = [0u16; 64];
    let n = unsafe {
        GetTimeFormatEx(
            std::ptr::null(),
            TIME_NOSECONDS,
            &st,
            std::ptr::null(),
            buf.as_mut_ptr(),
            buf.len() as i32,
        )
    };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
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
