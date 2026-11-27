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
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle, NSString};
    use std::cell::{Cell, RefCell};
    use std::sync::Once;

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        static NSCurrentLocaleDidChangeNotification: &'static NSString;
        static NSSystemTimeZoneDidChangeNotification: &'static NSString;
    }

    thread_local! {
        static CHANGES: Cell<u64> = const { Cell::new(0) };
        static FORMATTER: RefCell<Option<(u64, Retained<NSDateFormatter>)>> = const { RefCell::new(None) };
    }
    static OBSERVE: Once = Once::new();

    OBSERVE.call_once(|| unsafe {
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let main: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        for name in [
            NSCurrentLocaleDidChangeNotification,
            NSSystemTimeZoneDidChangeNotification,
        ] {
            let on_change = RcBlock::new(|_: *mut AnyObject| CHANGES.set(CHANGES.get() + 1));
            let _: *mut AnyObject = msg_send![
                center,
                addObserverForName: name,
                object: std::ptr::null::<AnyObject>(),
                queue: main,
                usingBlock: &*on_change
            ];
        }
    });

    let date = NSDate::dateWithTimeIntervalSinceNow(secs.max(0.0));
    let changes = CHANGES.get();
    FORMATTER.with_borrow_mut(|cached| {
        if cached.as_ref().is_some_and(|(c, _)| *c != changes) {
            *cached = None;
        }
        let (_, f) = cached.get_or_insert_with(|| {
            let f = NSDateFormatter::new();
            f.setDateStyle(NSDateFormatterStyle::NoStyle);
            f.setTimeStyle(NSDateFormatterStyle::ShortStyle);
            (changes, f)
        });
        f.stringFromDate(&date).to_string()
    })
}
