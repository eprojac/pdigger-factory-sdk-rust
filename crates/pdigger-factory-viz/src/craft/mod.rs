//! The Craft look, vendored from storytold's TextureCraft and craft-libs (MIT OR Apache-2.0).
//!
//! These files are copies, adapted to this crate, of:
//!
//! - [`theme`]: `crates/ui-egui/src/theme.rs` of <https://github.com/storytold/texturecraft> at
//!   `97f6b819846b40b1f7f5272858f643ca285068b9` (design tokens in four brightness levels, one
//!   accent colour, cards), with `WidgetPalette` from `crates/ui/src/theme/mod.rs` of
//!   <https://github.com/storytold/craft-libs> (`craft-ui`) at
//!   `390fe98b0dd3fb84db7c3af7cbd958db4e8cc585`;
//! - [`buttons`]: `IconButton` from `crates/ui/src/buttons/mod.rs` of craft-libs at the same
//!   revision;
//! - [`split`]: `split_rect` from `crates/ui/src/layout/geometry.rs` of craft-libs at the same
//!   revision (the panel layout of TextureCraft's dock).
//!
//! They are vendored, not depended on, because craft-ui is not on crates.io and builds against
//! storytold's egui fork; vendoring keeps `pdigger-factory-viz` publishable on stock egui 0.37.
//! Copyright (c) 2026 ArtCraft Team and the TextureCraft and craft-libs contributors; licensed
//! under the MIT License or the Apache License, Version 2.0, at your option. See `NOTICE`.

pub mod buttons;
pub mod split;
pub mod theme;

pub use buttons::{ButtonVisuals, IconButton, IconButtonStyle};
pub use split::{SplitAxis, SplitLayout, SplitSize, split_rect};
pub use theme::{Accent, Brightness, Tokens, WidgetPalette};
