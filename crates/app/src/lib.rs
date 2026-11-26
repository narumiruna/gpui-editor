#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod app;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod editor;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod theme;
pub use gpui_editor_document as document;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use gpui_editor_preview::media;
pub use gpui_editor_preview::rendering;
pub use gpui_editor_workspace as workspace;
