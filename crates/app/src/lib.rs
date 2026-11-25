#[cfg(target_os = "macos")]
pub mod app;
#[cfg(target_os = "macos")]
pub mod editor;
#[cfg(target_os = "macos")]
mod theme;
pub use gpui_editor_document as document;
#[cfg(target_os = "macos")]
pub use gpui_editor_preview::media;
pub use gpui_editor_preview::rendering;
pub use gpui_editor_workspace as workspace;
