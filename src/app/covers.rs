use super::*;

impl Glyph {
    pub fn set_language(&mut self, l: crate::i18n::Lang, cx: &mut Context<Self>) {
        crate::i18n::set_lang(l);
        self.lang = l;
        crate::config_ext::set_language(&mut self.config, l);
        self.save_config(cx);
        cx.notify();
    }

    /// YouTube / SoundCloud as fallback (official, visible embed player).
    pub fn embed_fallback(&self) -> bool {
        let en = &self.config.providers.enabled;
        en.contains(&ProviderId::YouTube) || en.contains(&ProviderId::SoundCloud)
    }

    pub fn toggle_embed_fallback(&mut self, cx: &mut Context<Self>) {
        let on = !self.embed_fallback();
        let en = &mut self.config.providers.enabled;
        en.retain(|p| !matches!(p, ProviderId::YouTube | ProviderId::SoundCloud));
        if on {
            en.push(ProviderId::SoundCloud);
            en.push(ProviderId::YouTube);
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn embed_playing(&self) -> bool {
        self.player_state
            .as_ref()
            .is_some_and(|s| s.backend == Some(glyph_audio::BackendKind::Embed))
    }

    /// Cached cover path for a track; kicks off a fetch the first time.
    pub fn cover_path(&mut self, t: &Track, cx: &mut Context<Self>) -> Option<PathBuf> {
        let key = track_key(t);
        if let Some(v) = self.covers.get(&key) {
            return v.clone();
        }
        // Not cached as a miss before the backend exists: the home view renders
        // before it is ready, and those covers must still load afterwards.
        let b = self.backend.clone()?;
        if self.covers.len() >= 1024 {
            if let Some(evict) = self.covers.keys().find(|k| !self.covers_loading.contains(*k)).cloned() {
                self.covers.remove(&evict);
            }
        }
        self.covers.insert(key.clone(), None);
        let t = t.clone();
        self.covers_loading.insert(key.clone());
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.cover(&t)).await;
            this.update(cx, |s, cx| {
                s.covers_loading.remove(&key);
                if let Ok(Some(p)) = r {
                    s.covers.insert(key, Some(p));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// The ASCII picture for this cover size is rendered already.
    pub fn ascii_ready(&self, t: &Track, size: f32) -> bool {
        self.ascii_covers.get(&(track_key(t), (size * 2.0) as u32)).is_some_and(|v| v.is_some())
    }

    /// The cover at `path` as an ASCII picture of `px` pixels (rendered once
    /// on the background executor, then cached).
    pub fn ascii_cover(&mut self, t: &Track, path: &std::path::Path, px: u32, cx: &mut Context<Self>) -> Option<Arc<gpui::RenderImage>> {
        let key = (track_key(t), px);
        if let Some(v) = self.ascii_covers.get(&key) {
            return v.clone();
        }
        if self.ascii_covers.len() >= 512 {
            if let Some(evict) = self.ascii_covers.iter().find(|(_, v)| v.is_some()).map(|(k, _)| k.clone()) {
                if let Some(Some(im)) = self.ascii_covers.remove(&evict) {
                    self.stale_heroes.push(im);
                }
            }
        }
        self.ascii_covers.insert(key.clone(), None);
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let img = cx
                .background_executor()
                .spawn(async move { crate::visuals::ascii::cover_raster(&path, px) })
                .await;
            this.update(cx, |s, cx| {
                if let Some(img) = img {
                    s.ascii_covers.insert(key, Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(img)]))));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// `path` as the artist hero (ASCII raster, or the faded photo when ASCII
    /// covers are off), computed on a background thread. The fade is baked
    /// into the raster, so it must never be cropped off: `min_w` is the
    /// raster width needed for the hero to crop only at the sides
    /// (`object-fit: cover`), bucketed so resizing re-renders rarely. While a
    /// new width renders, the previous one stays up.
    pub fn hero_image(&mut self, path: &std::path::Path, min_w: f32, h: u32, photo: bool, cx: &mut Context<Self>) -> Option<Arc<gpui::RenderImage>> {
        const STEP: u32 = 320;
        let ascii = self.config.visuals.ascii_covers && !photo;
        let w = ((min_w.max(0.0) as u32).div_ceil(STEP) * STEP).max(1680);
        let key = (path.to_path_buf(), ascii, w, h);
        if let Some(Some(v)) = self.heroes.get(&key) {
            return Some(v.clone());
        }
        let fallback = self.heroes.iter().find(|((p, a, _, hh), v)| p == path && *a == ascii && *hh == h && v.is_some()).and_then(|(_, v)| v.clone());
        if self.heroes.contains_key(&key) {
            return fallback;
        }
        if self.heroes.len() >= 32 {
            if let Some(evict) = self.heroes.iter().find(|((p, _, _, _), v)| p != path && v.is_some()).map(|(k, _)| k.clone()) {
                if let Some(Some(im)) = self.heroes.remove(&evict) {
                    self.stale_heroes.push(im);
                }
            }
        }
        self.heroes.insert(key.clone(), None);
        cx.spawn(async move |this, cx| {
            let src = key.0.clone();
            let img = cx
                .background_executor()
                .spawn(async move {
                    if ascii {
                        crate::visuals::ascii::raster(&src, w, h, 0.3)
                    } else {
                        crate::visuals::ascii::photo_fade(&src, w, h, 0.3)
                    }
                })
                .await;
            this.update(cx, |s, cx| {
                if let Some(img) = img {
                    // Other finished widths of this picture are superseded.
                    let old: Vec<_> = s.heroes.keys().filter(|k| k.0 == key.0 && k.1 == key.1 && k.3 == key.3 && k.2 != key.2).cloned().collect();
                    for k in old {
                        if let Some(Some(im)) = s.heroes.remove(&k) {
                            s.stale_heroes.push(im);
                        }
                    }
                    s.heroes.insert(key, Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(img)]))));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        fallback
    }

    pub fn cover_loading(&self, t: &Track) -> bool {
        self.covers_loading.contains(&track_key(t))
    }
}
