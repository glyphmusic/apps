//! Plugins people publish on GitHub. Two kinds share one manifest, catalog
//! and install flow:
//! - **theme**: colours and a look (pure data).
//! - **extension**: anything else. Without code it can rename texts
//!   (`strings`), hide settings rows (`hide`), bring a theme and a look. With code
//!   (`main`, a WebAssembly module run in a sandbox by
//!   [`crate::plugin_host`]) it can also add settings pages and sections,
//!   song menu entries, and use Glyph's API (playback, library, search,
//!   settings, network) as far as its `permissions` allow.
//!
//! Discovery runs on this device, never through a Glyph server:
//! - **Checked**: `index.json` of the `glyphmusic/plugins` repo. Authors add
//!   their plugin there by pull request; CI checks the manifest and pins the
//!   commit.
//! - **Community**: every GitHub repo with the topic `glyph-plugin`, found
//!   with GitHub's search API. Nobody checked these; only plugins without
//!   code may be installed from here.
//!
//! The catalog is cached in `<data>/plugins/catalog.json` for an hour (the
//! unauthenticated search API allows 10 calls a minute). Installed plugins
//! live in `<data>/plugins/<id>/` (`glyph-plugin.json` + `origin.json`, the
//! code as `code.wasm`, the files of its look in `look/`, the plugin's
//! settings in `values.json` and its own data in `storage.json`).
//!
//! A **look** (`look`, any kind, no code) can change the whole interface:
//! the font, how round corners are, pictures for the background picker,
//! Glyph's icons, and a style sheet for every window, menu and dialog. The
//! style sheet only loads the look's own files (`url()` of a listed asset,
//! sent to the UI as `data:` addresses), never anything from the network.
//! The UI pauses every look while Settings â€º Plugins is open, so the plugin
//! warnings and the switch to turn a look off always look like Glyph.

use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// GitHub topic that marks a repo as a Glyph plugin.
pub const TOPIC: &str = "glyph-plugin";
/// Manifest file at the root of a plugin repo (or of its `path`).
pub const MANIFEST_FILE: &str = "glyph-plugin.json";
/// Checked plugins (built by CI in `glyphmusic/plugins`).
pub const INDEX_URL: &str = "https://raw.githubusercontent.com/glyphmusic/plugins/main/index.json";
/// Where authors read how to publish a plugin.
pub const GUIDE_URL: &str = "https://github.com/glyphmusic/plugins#readme";
/// The catalog on the website (each plugin at `/plugins/<id>`).
pub const WEB_URL: &str = "https://glyph.my/plugins";
/// Highest manifest schema this build understands (1: themes, 2: extensions).
pub const SCHEMA: u32 = 2;
/// Installed plugin code, next to the manifest.
pub const CODE_FILE: &str = "code.wasm";
/// Plugin code bigger than this is refused.
pub const MAX_CODE: usize = 16 * 1024 * 1024;
/// Placeholder id of glyphmusic/plugin-template; never a real plugin.
pub const TEMPLATE_ID: &str = "change-me";

const SEARCH_URL: &str = "https://api.github.com/search/repositories?q=topic:glyph-plugin+fork:true&sort=updated&order=desc&per_page=100";
const CATALOG_TTL_S: u64 = 3600;
/// Community repos whose manifest is fetched per refresh (newest first).
const COMMUNITY_MAX: usize = 60;
/// Manifests are small (texts in a few languages at most); anything bigger
/// is not one.
const MAX_MANIFEST: usize = 256 * 1024;


mod catalog;
mod installed;
mod manifest;
#[cfg(test)]
mod tests;

pub use catalog::*;
pub use installed::*;
pub use manifest::*;