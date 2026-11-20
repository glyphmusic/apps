//! Background pictures (Settings › Appearance › Background): the user's own
//! (`glyph_core::backgrounds`) and those of enabled plugins' looks. The UI
//! picks one by setting `config.visuals.bg_image` to the id listed here.

use crate::controller::Core;
use serde_json::{json, Value};
use std::sync::Arc;

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

/// `{ items: [{ id, url, plugin }] }`: own pictures (newest first), then the
/// plugins' (`plugin` = the plugin's name).
pub fn backgrounds_list(core: C<'_>) -> Value {
    let own = glyph_core::backgrounds::list().into_iter().filter_map(|n| {
        let p = glyph_core::backgrounds::path(&n)?;
        Some(json!({ "id": format!("own:{n}"), "url": crate::host::asset_url(&p), "plugin": Value::Null }))
    });
    let theirs = crate::plugins::look_backgrounds(core)
        .into_iter()
        .map(|(id, p, name)| json!({ "id": id, "url": crate::host::asset_url(&p), "plugin": name }));
    json!({ "items": own.chain(theirs).collect::<Vec<_>>() })
}

/// Stores a picture the UI sends (base64, already scaled down); returns its id.
pub async fn background_add(data: String) -> Res<String> {
    let name = crate::rt::spawn_blocking(move || glyph_core::backgrounds::add_base64(&data).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())??;
    Ok(format!("own:{name}"))
}

/// Deletes one of the user's own pictures (plugins' go with the plugin).
pub fn background_remove(id: String) -> Res {
    let name = id.strip_prefix("own:").ok_or_else(|| "only own pictures can be removed".to_string())?;
    glyph_core::backgrounds::remove(name).map_err(|e| e.to_string())
}
