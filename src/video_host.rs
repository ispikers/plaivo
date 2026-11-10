use raw_window_handle::RawWindowHandle;

pub use imp::VideoHost;
#[cfg(windows)]
pub use imp::hide_window;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Region {
    pub x: f32,
    pub bottom: f32,
    pub w: f32,
    pub h: f32,
}

pub fn create(raw: RawWindowHandle) -> Result<VideoHost, String> {
    imp::VideoHost::new(raw)
}

#[cfg(windows)]
mod imp {
    use super::{RawWindowHandle, Region};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    static SHOWN: AtomicBool = AtomicBool::new(false);
    static REGION: Mutex<Option<Region>> = Mutex::new(None);
    static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);

    // mpv renders into a disabled child window; GPUI draws into a DirectComposition visual that composites above
    // child windows, so video, UI, thumbnails and Peek all come from this one window.
    pub struct VideoHost {
        app: HWND,
        host: HWND,
    }

    impl VideoHost {
        pub fn new(raw: RawWindowHandle) -> Result<Self, String> {
            let RawWindowHandle::Win32(h) = raw else {
                return Err("not a Win32 window".into());
            };
            let app = h.hwnd.get() as HWND;
            unsafe {
                let class: Vec<u16> = "PlaivoVideoHost\0".encode_utf16().collect();
                let instance = GetModuleHandleW(std::ptr::null());
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(DefWindowProcW),
                    hInstance: instance,
                    lpszClassName: class.as_ptr(),
                    hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
                    ..std::mem::zeroed()
                };
                RegisterClassW(&wc);
                // WS_CLIPCHILDREN keeps the host's background paints from invalidating mpv's window; without it a
                // paused mpv re-renders its frame continuously (~30% of a core).
                let host = CreateWindowExW(
                    WS_EX_NOPARENTNOTIFY,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | WS_DISABLED | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                    0,
                    0,
                    1,
                    1,
                    app,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if host.is_null() {
                    return Err("CreateWindowExW failed".into());
                }
                SetWindowSubclass(app, Some(follow), 1, host as usize);
                Ok(Self { app, host })
            }
        }

        pub fn wid(&self) -> i64 {
            self.host as i64
        }

        pub fn set_visible(&self, on: bool) {
            SHOWN.store(on, Ordering::Relaxed);
            unsafe {
                if on {
                    place(self.app, self.host);
                    ShowWindow(self.host, SW_SHOWNA);
                } else {
                    ShowWindow(self.host, SW_HIDE);
                }
            }
        }

        pub fn sync(&self) {
            unsafe { place(self.app, self.host) };
        }

        pub fn set_region(&self, region: Option<Region>) {
            *REGION.lock().unwrap() = region;
            self.sync();
        }

        pub fn set_cursor_hidden(&self, hidden: bool) {
            CURSOR_HIDDEN.store(hidden, Ordering::Relaxed);
            unsafe {
                SetCursor(if hidden {
                    std::ptr::null_mut()
                } else {
                    LoadCursorW(std::ptr::null_mut(), IDC_ARROW)
                })
            };
        }

        // GPUI doesn't expose its DirectComposition visual, so the UI stays composited over the video here.
        pub fn set_ui_hidden(&self, _hidden: bool) {}
    }

    pub fn hide_window(raw: RawWindowHandle) {
        if let RawWindowHandle::Win32(h) = raw {
            unsafe { ShowWindow(h.hwnd.get() as HWND, SW_HIDE) };
        }
    }

    unsafe fn place(app: HWND, host: HWND) {
        unsafe {
            let mut rc = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            GetClientRect(app, &mut rc);
            let (x, y, w, h) = match *REGION.lock().unwrap() {
                None => (0, 0, rc.right, rc.bottom),
                Some(r) => {
                    let k = GetDpiForWindow(app) as f32 / 96.0;
                    let px = |v: f32| (v * k).round() as i32;
                    (
                        px(r.x),
                        rc.bottom - px(r.bottom) - px(r.h),
                        px(r.w),
                        px(r.h),
                    )
                }
            };
            SetWindowPos(host, HWND_BOTTOM, x, y, w.max(1), h.max(1), SWP_NOACTIVATE);
        }
    }

    unsafe extern "system" fn follow(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        unsafe {
            if msg == WM_SIZE && SHOWN.load(Ordering::Relaxed) {
                place(hwnd, data as HWND);
            }
            // GPUI sets the cursor on every WM_SETCURSOR; while the player has hidden it, answer first.
            if msg == WM_SETCURSOR
                && (lp & 0xffff) as u32 == HTCLIENT
                && CURSOR_HIDDEN.load(Ordering::Relaxed)
            {
                SetCursor(std::ptr::null_mut());
                return 1;
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{RawWindowHandle, Region};
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSCursor, NSView, NSWindowOrderingMode};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    // mpv hosts its Metal layer in this view (deps/mpv-macos-embed.patch adds --wid to macvk). It sits below GPUI's
    // transparent view in the same window, so the video moves, resizes, goes full screen and composites with the UI natively.
    pub struct VideoHost {
        view: Retained<NSView>,
        ui: Retained<NSView>,
    }

    impl VideoHost {
        pub fn new(raw: RawWindowHandle) -> Result<Self, String> {
            let RawWindowHandle::AppKit(h) = raw else {
                return Err("not an AppKit window".into());
            };
            let mtm =
                MainThreadMarker::new().ok_or("video host must be created on the main thread")?;
            let ui = unsafe { Retained::retain(h.ns_view.cast::<NSView>().as_ptr()) }
                .ok_or("no GPUI view")?;
            let content = unsafe { ui.superview() }.ok_or("GPUI view has no superview")?;
            let view = NSView::initWithFrame(mtm.alloc(), content.bounds());
            view.setHidden(true);
            content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(&ui));
            let host = Self { view, ui };
            host.set_region(None);
            Ok(host)
        }

        pub fn wid(&self) -> i64 {
            Retained::as_ptr(&self.view) as i64
        }

        pub fn set_visible(&self, on: bool) {
            self.view.setHidden(!on);
        }

        pub fn set_region(&self, region: Option<Region>) {
            let Some(content) = (unsafe { self.view.superview() }) else {
                return;
            };
            let (frame, mask) = match region {
                None => (
                    content.bounds(),
                    NSAutoresizingMaskOptions::ViewWidthSizable
                        | NSAutoresizingMaskOptions::ViewHeightSizable,
                ),
                Some(r) => (
                    NSRect::new(
                        NSPoint::new(r.x as f64, r.bottom as f64),
                        NSSize::new(r.w as f64, r.h as f64),
                    ),
                    NSAutoresizingMaskOptions::ViewMaxXMargin
                        | NSAutoresizingMaskOptions::ViewMaxYMargin,
                ),
            };
            self.view.setAutoresizingMask(mask);
            self.view.setFrame(frame);
        }

        pub fn set_cursor_hidden(&self, hidden: bool) {
            if hidden {
                NSCursor::setHiddenUntilMouseMoves(true);
            }
        }

        // Only the layer: the view keeps its tracking area and first responder, so mouse moves and keys still reach GPUI.
        pub fn set_ui_hidden(&self, hidden: bool) {
            if let Some(layer) = self.ui.layer() {
                layer.setHidden(hidden);
            }
        }
    }
}
