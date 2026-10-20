//! The icon set from `frontend/index.html` (`<g id="i-*">`), plus the brand mark, as SVG
//! files baked into the binary and served by [`Assets`].
//!
//! The rules of the set still hold: 24×24 grid, stroke 2, no fill, square caps, miter
//! joins; the size is set by the caller, never by the symbol; the colour is never in the
//! file. GPUI paints an SVG as a mask in one colour, so an icon takes the text colour of
//! the element it sits in: presence, danger and accent reach it the same way they did
//! through `currentColor`.
//!
//! ```ignore
//! div().text_color(theme::TEXT_MUTED).child(Icon::Mic.size(16.))
//! ```

use gpui_kit::{px, AssetSource, Result, SharedString, Styled};
use std::borrow::Cow;

macro_rules! icons {
    ($($(#[$doc:meta])* $variant:ident => $file:literal),* $(,)?) => {
        /// One drawing from the set. See the module docs for how to render one.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($(#[$doc])* $variant,)*
        }

        impl Icon {
            pub const ALL: &'static [Icon] = &[$(Icon::$variant,)*];

            /// The asset path the [`Assets`] source answers for.
            pub fn path(self) -> &'static str {
                match self {
                    $(Icon::$variant => concat!("fitcom/icons/", $file, ".svg"),)*
                }
            }

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!("assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    Hash => "hash",
    At => "at",
    Gear => "gear",
    Mic => "mic",
    MicOff => "mic-off",
    Head => "head",
    HeadOff => "head-off",
    Moon => "moon",
    Monitor => "monitor",
    Clip => "clip",
    Download => "download",
    Retry => "retry",
    X => "x",
    Min => "min",
    Max => "max",
    Chev => "chev",
    Plus => "plus",
    Users => "users",
    Search => "search",
    Leave => "leave",
    File => "file",
    Image => "image",
    Alert => "alert",
    Check => "check",
    Edit => "edit",
    Msg => "msg",
    Trash => "trash",
    Signal => "signal",
    Wave => "wave",
    Share => "share",
    Cam => "cam",
    CamOff => "cam-off",
    Open => "open",
    Play => "play",
    Tiles => "tiles",
    Music => "music",
    /// One toggle for a listener: they do not know whether the DJ's Spotify is playing.
    PlayPause => "play-pause",
    Next => "next",
    /// The titlebar mark: three nodes on a 16×16 grid, filled, with faint links.
    Brand => "brand",
}

impl Icon {
    /// The icon as an element of `px` × `px`, tinted by the inherited text colour. Override
    /// the colour with `.text_color(..)` on the result when the parent's is not the one.
    pub fn size(self, px_size: f32) -> gpui_kit::component::Icon {
        gpui_kit::component::Icon::default()
            .path(self.path())
            .size(px(px_size))
    }
}

/// The asset source for the whole app: our icons first, then gpui-kit's own bundle, which
/// its components (Select's chevron, Checkbox's tick, …) load by their own paths.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = Icon::ALL.iter().find(|i| i.path() == path) {
            return Ok(Some(Cow::Borrowed(icon.bytes())));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = Icon::ALL
            .iter()
            .map(|i| i.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect();
        all.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant must resolve to a real, parseable-looking SVG; a typo in the table
    /// would otherwise show up as an empty square somewhere in the window.
    #[test]
    fn every_icon_is_served_as_an_svg() {
        for icon in Icon::ALL {
            let bytes = Assets.load(icon.path()).unwrap().expect("served");
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(
                text.starts_with("<svg") && text.trim_end().ends_with("</svg>"),
                "{icon:?}"
            );
        }
    }
}
