//! LaTeX preview: renders `.tex` documents without a PDF viewer round trip.
//! The SVG path compiles to DVI/XDV with the dvisvgm driver injected and
//! converts pages with dvisvgm; the PDF path compiles the unmodified source
//! and serves as cache and fallback when the SVG would miss content.
//!
//! Like the other file previews it has no switch or workspace service: the
//! preview pane opens a session when it mounts and closes it when it
//! unmounts, and builds run only while a pane is visible.

pub mod build;
pub mod manager;
pub mod pipeline;
pub mod toolchain;
