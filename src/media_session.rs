use raw_window_handle::RawWindowHandle;

pub use imp::{Session, artwork};

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Play,
    Pause,
    #[cfg_attr(windows, expect(dead_code, reason = "SMTC has no toggle button"))]
    Toggle,
    Stop,
    Next,
    Previous,
    Seek(f64),
}

#[derive(Clone, Copy, Default)]
pub struct State {
    pub playing: bool,
    pub speed: f64,
    pub position: f64,
    pub duration: f64,
    pub previous: bool,
    pub next: bool,
}

#[cfg(windows)]
mod imp {
    use super::{Command, RawWindowHandle, State};
    use std::sync::Arc;
    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Media::*;
    use windows::Storage::Streams::{
        DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::core::{HSTRING, Ref, Result, factory};

    pub struct Artwork(RandomAccessStreamReference);

    pub fn artwork(bytes: Vec<u8>) -> Option<Artwork> {
        let load = || -> Result<_> {
            let stream = InMemoryRandomAccessStream::new()?;
            let writer = DataWriter::CreateDataWriter(&stream)?;
            writer.WriteBytes(&bytes)?;
            writer.StoreAsync()?.join()?;
            writer.DetachStream()?;
            stream.Seek(0)?;
            RandomAccessStreamReference::CreateFromStream(&stream)
        };
        load()
            .inspect_err(|e| log::warn!("media session artwork: {e}"))
            .ok()
            .map(Artwork)
    }

    fn span(secs: f64) -> TimeSpan {
        TimeSpan {
            Duration: (secs.max(0.0) * 1e7) as i64,
        }
    }

    fn warn(what: &str, r: Result<()>) {
        if let Err(e) = r {
            log::warn!("media session {what}: {e}");
        }
    }

    pub struct Session(SystemMediaTransportControls);

    impl Session {
        pub fn new(
            window: RawWindowHandle,
            on: impl Fn(Command) + Send + Sync + 'static,
        ) -> std::result::Result<Self, String> {
            let RawWindowHandle::Win32(h) = window else {
                return Err("not a Win32 window".into());
            };
            let on = Arc::new(on);
            let pressed = on.clone();
            let smtc = || -> Result<SystemMediaTransportControls> {
                let smtc: SystemMediaTransportControls = unsafe {
                    factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?
                        .GetForWindow(HWND(h.hwnd.get() as _))?
                };
                smtc.ButtonPressed(&TypedEventHandler::<
                    SystemMediaTransportControls,
                    SystemMediaTransportControlsButtonPressedEventArgs,
                >::new(move |_, args| {
                    pressed(match args.ok()?.Button()? {
                        SystemMediaTransportControlsButton::Play => Command::Play,
                        SystemMediaTransportControlsButton::Pause => Command::Pause,
                        SystemMediaTransportControlsButton::Stop => Command::Stop,
                        SystemMediaTransportControlsButton::Next => Command::Next,
                        SystemMediaTransportControlsButton::Previous => Command::Previous,
                        _ => return Ok(()),
                    });
                    Ok(())
                }))?;
                smtc.PlaybackPositionChangeRequested(&TypedEventHandler::<
                    SystemMediaTransportControls,
                    PlaybackPositionChangeRequestedEventArgs,
                >::new(
                    move |_, args: Ref<PlaybackPositionChangeRequestedEventArgs>| {
                        let t = args.ok()?.RequestedPlaybackPosition()?.Duration;
                        on(Command::Seek(t as f64 / 1e7));
                        Ok(())
                    },
                ))?;
                smtc.SetIsPlayEnabled(true)?;
                smtc.SetIsPauseEnabled(true)?;
                smtc.SetIsStopEnabled(true)?;
                Ok(smtc)
            };
            smtc().map(Self).map_err(|e| e.to_string())
        }

        pub fn set_item(&mut self, title: &str, subtitle: &str) {
            warn(
                "item",
                (|| {
                    let d = self.0.DisplayUpdater()?;
                    d.ClearAll()?;
                    d.SetType(MediaPlaybackType::Video)?;
                    let v = d.VideoProperties()?;
                    v.SetTitle(&HSTRING::from(title))?;
                    v.SetSubtitle(&HSTRING::from(subtitle))?;
                    d.Update()?;
                    self.0.SetIsEnabled(true)
                })(),
            );
        }

        pub fn set_artwork(&mut self, art: Artwork) {
            warn(
                "artwork",
                (|| {
                    let d = self.0.DisplayUpdater()?;
                    d.SetThumbnail(&art.0)?;
                    d.Update()
                })(),
            );
        }

        pub fn set_state(&mut self, s: &State) {
            warn(
                "state",
                (|| {
                    self.0.SetPlaybackStatus(if s.playing {
                        MediaPlaybackStatus::Playing
                    } else {
                        MediaPlaybackStatus::Paused
                    })?;
                    self.0.SetPlaybackRate(s.speed)?;
                    self.0.SetIsNextEnabled(s.next)?;
                    self.0.SetIsPreviousEnabled(s.previous)?;
                    let t = SystemMediaTransportControlsTimelineProperties::new()?;
                    t.SetStartTime(span(0.0))?;
                    t.SetMinSeekTime(span(0.0))?;
                    t.SetEndTime(span(s.duration))?;
                    t.SetMaxSeekTime(span(s.duration))?;
                    t.SetPosition(span(s.position))?;
                    self.0.UpdateTimelineProperties(&t)
                })(),
            );
        }

        pub fn clear(&mut self) {
            warn(
                "clear",
                (|| {
                    self.0.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
                    let d = self.0.DisplayUpdater()?;
                    d.ClearAll()?;
                    d.Update()?;
                    self.0.SetIsEnabled(false)
                })(),
            );
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Command, RawWindowHandle, State};
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::rc::{Allocated, Retained, autoreleasepool};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSSize, NSString};
    use std::ffi::{CStr, c_void};
    use std::rc::Rc;

    #[link(name = "MediaPlayer", kind = "framework")]
    unsafe extern "C" {
        static MPMediaItemPropertyTitle: &'static NSString;
        static MPMediaItemPropertyArtist: &'static NSString;
        static MPMediaItemPropertyArtwork: &'static NSString;
        static MPMediaItemPropertyPlaybackDuration: &'static NSString;
        static MPNowPlayingInfoPropertyElapsedPlaybackTime: &'static NSString;
        static MPNowPlayingInfoPropertyPlaybackRate: &'static NSString;
        static MPNowPlayingInfoPropertyMediaType: &'static NSString;
    }

    const HANDLER_SUCCESS: isize = 0;
    const MEDIA_TYPE_VIDEO: usize = 2;
    const STATE_PLAYING: usize = 1;
    const STATE_PAUSED: usize = 2;
    const STATE_STOPPED: usize = 3;

    fn class(name: &CStr) -> &'static AnyClass {
        AnyClass::get(name).expect("MediaPlayer is linked")
    }

    pub struct Artwork(Vec<u8>);

    pub fn artwork(bytes: Vec<u8>) -> Option<Artwork> {
        Some(Artwork(bytes))
    }

    pub struct Session {
        transport: [*mut AnyObject; 5],
        next: *mut AnyObject,
        previous: *mut AnyObject,
        title: Retained<NSString>,
        subtitle: Retained<NSString>,
        artwork: Option<Retained<AnyObject>>,
        state: State,
        has_item: bool,
    }

    impl Session {
        pub fn new(
            _: RawWindowHandle,
            on: impl Fn(Command) + Send + Sync + 'static,
        ) -> Result<Self, String> {
            let on = Rc::new(on);
            unsafe {
                let center: *mut AnyObject =
                    msg_send![class(c"MPRemoteCommandCenter"), sharedCommandCenter];
                let commands: [(*mut AnyObject, Command); 6] = [
                    (msg_send![center, playCommand], Command::Play),
                    (msg_send![center, pauseCommand], Command::Pause),
                    (msg_send![center, togglePlayPauseCommand], Command::Toggle),
                    (msg_send![center, stopCommand], Command::Stop),
                    (msg_send![center, nextTrackCommand], Command::Next),
                    (msg_send![center, previousTrackCommand], Command::Previous),
                ];
                for (command, c) in commands {
                    let on = on.clone();
                    let handler = RcBlock::new(move |_: *mut AnyObject| -> isize {
                        on(c);
                        HANDLER_SUCCESS
                    });
                    let _: *mut AnyObject = msg_send![command, addTargetWithHandler: &*handler];
                }
                let seek: *mut AnyObject = msg_send![center, changePlaybackPositionCommand];
                let handler = RcBlock::new(move |e: *mut AnyObject| -> isize {
                    let t: f64 = msg_send![e, positionTime];
                    on(Command::Seek(t));
                    HANDLER_SUCCESS
                });
                let _: *mut AnyObject = msg_send![seek, addTargetWithHandler: &*handler];
                Ok(Self {
                    transport: [
                        commands[0].0,
                        commands[1].0,
                        commands[2].0,
                        commands[3].0,
                        seek,
                    ],
                    next: commands[4].0,
                    previous: commands[5].0,
                    title: NSString::new(),
                    subtitle: NSString::new(),
                    artwork: None,
                    state: State::default(),
                    has_item: false,
                })
            }
        }

        pub fn set_item(&mut self, title: &str, subtitle: &str) {
            self.title = NSString::from_str(title);
            self.subtitle = NSString::from_str(subtitle);
            self.artwork = None;
            self.has_item = true;
            self.set_transport_enabled(true);
            self.publish();
        }

        fn set_transport_enabled(&self, enabled: bool) {
            for command in self.transport {
                unsafe {
                    let _: () = msg_send![command, setEnabled: enabled];
                }
            }
        }

        pub fn set_artwork(&mut self, art: Artwork) {
            if !self.has_item {
                return;
            }
            self.artwork = autoreleasepool(|_| unsafe {
                let data: *mut AnyObject = msg_send![
                    class(c"NSData"),
                    dataWithBytes: art.0.as_ptr().cast::<c_void>(),
                    length: art.0.len()
                ];
                let image: Allocated<AnyObject> = msg_send![class(c"NSImage"), alloc];
                let image: Option<Retained<AnyObject>> = msg_send![image, initWithData: data];
                let image = image?;
                let size: NSSize = msg_send![&*image, size];
                let handler = RcBlock::new(move |_: NSSize| -> *mut AnyObject {
                    Retained::as_ptr(&image).cast_mut()
                });
                let artwork: Allocated<AnyObject> = msg_send![class(c"MPMediaItemArtwork"), alloc];
                msg_send![artwork, initWithBoundsSize: size, requestHandler: &*handler]
            });
            self.publish();
        }

        pub fn set_state(&mut self, s: &State) {
            self.state = *s;
            if !self.has_item {
                return;
            }
            unsafe {
                let _: () = msg_send![self.next, setEnabled: s.next];
                let _: () = msg_send![self.previous, setEnabled: s.previous];
            }
            self.publish();
        }

        pub fn clear(&mut self) {
            self.has_item = false;
            unsafe {
                let center: *mut AnyObject =
                    msg_send![class(c"MPNowPlayingInfoCenter"), defaultCenter];
                let _: () = msg_send![center, setNowPlayingInfo: std::ptr::null_mut::<AnyObject>()];
                let _: () = msg_send![center, setPlaybackState: STATE_STOPPED];
                let _: () = msg_send![self.next, setEnabled: false];
                let _: () = msg_send![self.previous, setEnabled: false];
            }
            self.set_transport_enabled(false);
        }

        fn publish(&self) {
            let s = &self.state;
            autoreleasepool(|_| unsafe {
                let info: *mut AnyObject = msg_send![class(c"NSMutableDictionary"), dictionary];
                let set = |key: &NSString, value: *const AnyObject| {
                    let _: () = msg_send![info, setObject: value, forKey: key];
                };
                let number = |v: f64| -> *const AnyObject {
                    msg_send![class(c"NSNumber"), numberWithDouble: v]
                };
                set(
                    MPMediaItemPropertyTitle,
                    Retained::as_ptr(&self.title).cast(),
                );
                set(
                    MPMediaItemPropertyArtist,
                    Retained::as_ptr(&self.subtitle).cast(),
                );
                set(MPMediaItemPropertyPlaybackDuration, number(s.duration));
                set(
                    MPNowPlayingInfoPropertyElapsedPlaybackTime,
                    number(s.position),
                );
                set(
                    MPNowPlayingInfoPropertyPlaybackRate,
                    number(if s.playing { s.speed } else { 0.0 }),
                );
                set(MPNowPlayingInfoPropertyMediaType, {
                    msg_send![class(c"NSNumber"), numberWithUnsignedInteger: MEDIA_TYPE_VIDEO]
                });
                if let Some(a) = &self.artwork {
                    set(MPMediaItemPropertyArtwork, Retained::as_ptr(a));
                }
                let center: *mut AnyObject =
                    msg_send![class(c"MPNowPlayingInfoCenter"), defaultCenter];
                let _: () = msg_send![center, setNowPlayingInfo: info];
                let state = if s.playing {
                    STATE_PLAYING
                } else {
                    STATE_PAUSED
                };
                let _: () = msg_send![center, setPlaybackState: state];
            });
        }
    }
}
