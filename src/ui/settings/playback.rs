use super::*;

// ------------------------------------------------------------------ pages

/// Live state of the network device search / output (Settings > Playback).
fn devices_status(app: &Glyph) -> Option<SharedString> {
    if let Some(o) = &app.device_out {
        return Some(match o.error() {
            Some(e) => format!("{} {}", t("sp.mcp.problem"), e).into(),
            None => tf("sp.devices.playing", &[("name", &o.device.name)]),
        });
    }
    let d = app.discovery.as_ref()?;
    let n = d.devices().len();
    Some(if n > 0 {
        tf("sp.devices.found", &[("n", &n)])
    } else if d.searching() {
        t("sp.devices.searching")
    } else {
        t("sp.devices.none")
    })
}

pub(super) fn playback(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    // Sub-tabs: general (sound and output), content (what plays), sources.
    match app.settings_sub {
        1 => return content(app, cx),
        2 => return sources(app, cx),
        3 => return player(app, cx),
        _ => {}
    }
    let cur = app.config.audio.quality;
    let options = [AudioQuality::Auto, AudioQuality::Low, AudioQuality::Standard, AudioQuality::High]
        .into_iter()
        .map(|q| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_audio_quality(q, cx));
            (quality_label(q), q == cur, f)
        })
        .collect();
    let quality = select(app, "sel-quality", quality_label(cur), options, cx);
    let cur_dl = app.config.audio.download_quality;
    let dl_options = [AudioQuality::Auto, AudioQuality::Low, AudioQuality::Standard, AudioQuality::High]
        .into_iter()
        .map(|q| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_download_quality(q, cx));
            (quality_label(q), q == cur_dl, f)
        })
        .collect();
    let dl_quality = select(app, "sel-dl-quality", quality_label(cur_dl), dl_options, cx);
    let mut items = vec![
        row(t("sp.audio.quality"), Some(t("sp.audio.quality.desc")), quality),
        row(t("sp.audio.dl_quality"), Some(t("sp.audio.dl_quality.desc")), dl_quality),
        row(t("sp.crossfade"), Some(t("sp.crossfade.desc")), {
            let cur = app.config.audio.crossfade_s;
            let label = |s: u32| -> String { if s == 0 { t("sp.crossfade.off").to_string() } else { format!("{s} s") } };
            let weak = cx.entity().downgrade();
            let pick: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
                weak.update(cx, |this, cx| this.set_crossfade((f * 20.0).round() as u32, cx)).ok();
            });
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(crate::ui::widgets::thin_bar("crossfade", 180.0, cur as f32 / 20.0, pick, Some(Rc::new(move |f: f32| label((f * 20.0).round() as u32)) as crate::ui::widgets::BarLabel), cx))
                .child(div().w(px(36.0)).text_size(px(13.0)).text_color(white(0.75)).child(label(cur)))
        }),
        row(t("sp.output"), Some(t("sp.output.desc")), {
            let cur = app.config.audio.output_device.clone();
            let mut options: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)> =
                vec![(t("sp.output.default"), cur.is_none(), Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_output_device(None, cx)))];
            let mut names = app.output_devices.clone();
            if let Some(c) = &cur {
                if !names.contains(c) {
                    names.push(c.clone());
                }
            }
            for n in names {
                let sel = cur.as_deref() == Some(n.as_str());
                let n2 = n.clone();
                options.push((n.into(), sel, Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.set_output_device(Some(n2.clone()), cx))));
            }
            let label: SharedString = cur.map(Into::into).unwrap_or_else(|| t("sp.output.default"));
            select(app, "sel-output", label, options, cx)
        }),
        row(t("sp.level"), Some(t("sp.level.desc")), {
            use glyph_core::config::VolumeLevel as V;
            let cur = app.config.audio.level;
            let label = |l: V| -> SharedString {
                match l {
                    V::Loud => t("sp.level.loud"),
                    V::Normal => t("sp.level.normal"),
                    V::Quiet => t("sp.level.quiet"),
                }
            };
            let options = [V::Loud, V::Normal, V::Quiet]
                .into_iter()
                .map(|l| {
                    let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_volume_level(l, cx));
                    (label(l), l == cur, f)
                })
                .collect();
            select(app, "sel-level", label(cur), options, cx)
        }),
        row(
            t("sp.normalize"),
            Some(t("sp.normalize.desc")),
            switch("normalize", app.config.audio.normalize).on_click(cx.listener(|this, _, _, cx| this.toggle_normalize(cx))),
        ),
        // MPD_ROW
        row_full(t("sp.mpd"), Some(t("sp.mpd.desc")), match (&app.mpd, app.config.mpd.enabled) {
            (Some(m), _) if m.error().is_some() => Some(format!("{} {}", t("sp.mcp.problem"), m.error().unwrap_or_default()).into()),
            (Some(m), _) if m.status().is_some() => Some(tf("sp.mpd.connected", &[("host", &m.address.label())])),
            (_, true) if crate::mpd::Address::parse(&app.config.mpd.address).is_none() => Some(t("sp.mpd.need_address")),
            _ => None,
        }, {
            let on = app.config.mpd.enabled;
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(input_box(app.mpd_input.clone()).w(px(170.0)))
                .child(switch("mpd-on", on).on_click(cx.listener(move |this, _, _, cx| this.set_mpd_enabled(!on, cx))))
        }),
        row_full(t("sp.devices"), Some(t("sp.devices.desc")), devices_status(app), {
            let on = app.config.devices.discovery;
            switch("devices-on", on).on_click(cx.listener(move |this, _, _, cx| this.set_discovery(!on, cx)))
        }),
        row(t("sp.audio.channels"), Some(t("sp.audio.channels.desc")), {
            use glyph_core::config::ChannelMode as C;
            let cur = app.config.audio.channels;
            segmented(
                [(C::Mono, t("sp.audio.channels.mono")), (C::Stereo, t("sp.audio.channels.stereo")), (C::Spatial, t("sp.audio.channels.spatial"))]
                    .into_iter()
                    .map(|(m, label)| (label, m == cur, Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.set_channel_mode(m, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>))
                    .collect(),
                cx,
                "channels",
            )
        }),
    ];
    // MPD is a music daemon on a computer; phones have nothing to talk to.
    if crate::ui::compact::TOUCH {
        items.remove(MPD_ROW);
    }
    rows(items)
}

/// Index of the "Play on MPD" row in the General list.
const MPD_ROW: usize = 6;
