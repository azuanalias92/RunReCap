//! Platform-independent wallpaper renderer for RunReCap.

pub mod caption;
pub mod color;
mod render;
pub mod text;
pub mod theme;

pub use color::Rgba;
pub use render::{render, Canvas, Insets, Layout, RenderError, RenderNotes, Scene};
pub use text::Fonts;
pub use theme::{builtin_themes, Theme};
pub use tiny_skia::Pixmap;
