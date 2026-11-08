use gpui::{AssetSource, Hsla, Rgba, SharedString, rgb, rgba};
use std::borrow::Cow;

pub fn bg() -> Rgba {
    rgb(0x0E0F12)
}
pub fn surface() -> Rgba {
    rgb(0x15171C)
}
pub fn text() -> Rgba {
    rgb(0xEDEDED)
}
pub fn muted() -> Hsla {
    Hsla::from(text()).opacity(0.6)
}
pub fn accent() -> Rgba {
    rgb(0xEDEDED)
}
pub fn on_accent() -> Rgba {
    rgb(0x0E0F12)
}
pub fn outline() -> Rgba {
    rgba(0xFFFFFF1F)
}
pub fn tint(alpha: f32) -> Hsla {
    Hsla::from(text()).opacity(alpha)
}
pub fn black(alpha: f32) -> Hsla {
    Hsla::from(rgb(0x000000)).opacity(alpha)
}

pub const SIDEBAR: f32 = 220.;
pub const SIDEBAR_COMPACT: f32 = 72.;
pub const MINI_BAR: f32 = 88.;
pub const MINI_HOLE: (f32, f32, f32, f32) = (12., 12., 114., 64.);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Menu,
    Home,
    Search,
    Movie,
    Tv,
    Fullscreen,
    FullscreenExit,
    Logout,
    Server,
    Back,
    Play,
    Pause,
    Replay,
    Forward,
    Stop,
    Volume,
    VolumeOff,
    Subtitles,
    Check,
    Close,
    ChevronLeft,
    ChevronRight,
    ExpandMore,
    Sort,
    PlayCircle,
    Refresh,
    Sparkle,
    OpenFull,
    Tune,
    Speed,
    HighQuality,
    Info,
    Speaker,
    FastForward,
    Remove,
    Add,
    ScaleFit,
    ScaleFill,
    ScaleStretch,
    SkipPrevious,
    SkipNext,
    Queue,
    Settings,
    OpenInNew,
    Person,
}

impl Icon {
    pub fn path(self) -> SharedString {
        format!("icons/{self:?}.svg").into()
    }

    fn d(self) -> &'static str {
        match self {
            Icon::Menu => "M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z",
            Icon::Home => "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z",
            Icon::Search => "M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z",
            Icon::Movie => "M18 4l2 4h-3l-2-4h-2l2 4h-3l-2-4H8l2 4H7L5 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V4h-4z",
            Icon::Tv => "M21 3H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h5v2h8v-2h5c1.1 0 1.99-.9 1.99-2L23 5c0-1.1-.9-2-2-2zm0 14H3V5h18v12z",
            Icon::Fullscreen => "M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z",
            Icon::FullscreenExit => "M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z",
            Icon::Logout => "M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z",
            Icon::Server => "M20 13H4c-.55 0-1 .45-1 1v6c0 .55.45 1 1 1h16c.55 0 1-.45 1-1v-6c0-.55-.45-1-1-1zM7 19c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zM20 3H4c-.55 0-1 .45-1 1v6c0 .55.45 1 1 1h16c.55 0 1-.45 1-1V4c0-.55-.45-1-1-1zM7 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2z",
            Icon::Back => "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z",
            Icon::Play => "M8 5v14l11-7z",
            Icon::Pause => "M6 19h4V5H6v14zm8-14v14h4V5h-4z",
            Icon::Replay => "M12 5V1L7 6l5 5V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z",
            Icon::Forward => "M18 13c0 3.31-2.69 6-6 6s-6-2.69-6-6 2.69-6 6-6v4l5-5-5-5v4c-4.42 0-8 3.58-8 8s3.58 8 8 8 8-3.58 8-8h-2z",
            Icon::Stop => "M6 6h12v12H6z",
            Icon::Volume => "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z",
            Icon::VolumeOff => "M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z",
            Icon::Subtitles => "M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zM4 12h4v2H4v-2zm10 6H4v-2h10v2zm6 0h-4v-2h4v2zm0-4H10v-2h10v2z",
            Icon::Check => "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z",
            Icon::Close => "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z",
            Icon::ChevronLeft => "M15.41 7.41L14 6l-6 6 6 6 1.41-1.41L10.83 12z",
            Icon::ChevronRight => "M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z",
            Icon::ExpandMore => "M16.59 8.59L12 13.17 7.41 8.59 6 10l6 6 6-6z",
            Icon::Sort => "M3 18h6v-2H3v2zM3 6v2h18V6H3zm0 7h12v-2H3v2z",
            Icon::PlayCircle => "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 14.5v-9l6 4.5-6 4.5z",
            Icon::Refresh => "M17.65 6.35C16.2 4.9 14.21 4 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08c-.82 2.33-3.04 4-5.65 4-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z",
            Icon::Sparkle => "M19 9l1.25-2.75L23 5l-2.75-1.25L19 1l-1.25 2.75L15 5l2.75 1.25L19 9zm-7.5.5L9 4 6.5 9.5 1 12l5.5 2.5L9 20l2.5-5.5L17 12l-5.5-2.5zM19 15l-1.25 2.75L15 19l2.75 1.25L19 23l1.25-2.75L23 19l-2.75-1.25L19 15z",
            Icon::OpenFull => "M21 11V3h-8l3.29 3.29-10 10L3 13v8h8l-3.29-3.29 10-10z",
            Icon::Tune => "M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z",
            Icon::Speed => "M20.38 8.57l-1.23 1.85a8 8 0 0 1-.22 7.58H5.07A8 8 0 0 1 15.58 6.85l1.85-1.23A10 10 0 0 0 3.35 19a2 2 0 0 0 1.72 1h13.85a2 2 0 0 0 1.74-1 10 10 0 0 0-.27-10.44zm-9.79 6.84a2 2 0 0 0 2.83 0l5.66-8.49-8.49 5.66a2 2 0 0 0 0 2.83z",
            Icon::HighQuality => "M19 4H5c-1.11 0-2 .9-2 2v12c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm-8 11H9.5v-2h-2v2H6V9h1.5v2.5h2V9H11v6zm7-1c0 .55-.45 1-1 1h-.75v1.5h-1.5V15H14c-.55 0-1-.45-1-1v-4c0-.55.45-1 1-1h3c.55 0 1 .45 1 1v4zm-3.5-.5h2v-3h-2v3z",
            Icon::Info => "M11 7h2v2h-2zm0 4h2v6h-2zm1-9C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8z",
            Icon::Speaker => "M17 2H7c-1.1 0-2 .9-2 1.99v16c0 1.1.9 2 2 2h10c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-5 2c1.1 0 2 .9 2 2s-.9 2-2 2c-1.11 0-2-.9-2-2s.89-2 2-2zm0 16c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z",
            Icon::FastForward => "M4 18l8.5-6L4 6v12zm9-12v12l8.5-6L13 6z",
            Icon::Remove => "M19 13H5v-2h14v2z",
            Icon::Add => "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z",
            Icon::ScaleFit => "M17 4h3c1.1 0 2 .9 2 2v2h-2V6h-3V4zM4 8V6h3V4H4c-1.1 0-2 .9-2 2v2h2zm16 8v2h-3v2h3c1.1 0 2-.9 2-2v-2h-2zM7 18H4v-2H2v2c0 1.1.9 2 2 2h3v-2zM18 8H6v8h12V8z",
            Icon::ScaleFill => "M3 5v4h2V5h4V3H5c-1.1 0-2 .9-2 2zm2 10H3v4c0 1.1.9 2 2 2h4v-2H5v-4zm14 4h-4v2h4c1.1 0 2-.9 2-2v-4h-2v4zm0-16h-4v2h4v4h2V5c0-1.1-.9-2-2-2z",
            Icon::SkipPrevious => "M6 6h2v12H6zm3.5 6l8.5 6V6z",
            Icon::SkipNext => "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z",
            Icon::Queue => "M19 9H2v2h17V9zm0-4H2v2h17V5zM2 15h13v-2H2v2zm15-2v6l5-3-5-3z",
            Icon::Settings => "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z",
            Icon::OpenInNew => "M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z",
            Icon::Person => "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z",
            Icon::ScaleStretch => "M10 9h4V6h3l-5-5-5 5h3v3zm-1 1H6V7l-5 5 5 5v-3h3v-4zm14 2l-5-5v3h-3v4h3v3l5-5zm-9 3h-4v3H7l5 5 5-5h-3v-3z",
        }
    }
}

const ALL: &[Icon] = &[
    Icon::Menu,
    Icon::Home,
    Icon::Search,
    Icon::Movie,
    Icon::Tv,
    Icon::Fullscreen,
    Icon::FullscreenExit,
    Icon::Logout,
    Icon::Server,
    Icon::Back,
    Icon::Play,
    Icon::Pause,
    Icon::Replay,
    Icon::Forward,
    Icon::Stop,
    Icon::Volume,
    Icon::VolumeOff,
    Icon::Subtitles,
    Icon::Check,
    Icon::Close,
    Icon::ChevronLeft,
    Icon::ChevronRight,
    Icon::ExpandMore,
    Icon::Sort,
    Icon::PlayCircle,
    Icon::Refresh,
    Icon::Sparkle,
    Icon::OpenFull,
    Icon::Tune,
    Icon::Speed,
    Icon::HighQuality,
    Icon::Info,
    Icon::Speaker,
    Icon::FastForward,
    Icon::Remove,
    Icon::Add,
    Icon::ScaleFit,
    Icon::ScaleFill,
    Icon::ScaleStretch,
    Icon::SkipPrevious,
    Icon::SkipNext,
    Icon::Queue,
    Icon::Settings,
    Icon::OpenInNew,
    Icon::Person,
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ALL.iter().find(|i| i.path() == path).map(|i| {
            Cow::Owned(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="white" d="{}"/></svg>"#, i.d()).into_bytes())
        }))
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ALL.iter().map(|i| i.path()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_served() {
        for i in ALL {
            let svg = Assets.load(&i.path()).unwrap().expect("icon missing");
            assert!(svg.starts_with(b"<svg"));
        }
        assert!(Assets.load("icons/Nope.svg").unwrap().is_none());
    }
}
