//! Settings that act on the player right away (src/app/boot.rs apply_config).

use crate::controller::Core;
use glyph_core::config::{ChannelMode, VolumeLevel};
use glyph_core::Config;
use std::time::Duration;

pub fn eq_gains(v: &[f32]) -> [f32; 10] {
    let mut g = [0.0; 10];
    for (i, x) in v.iter().take(10).enumerate() {
        g[i] = *x;
    }
    g
}

fn channels_of(m: ChannelMode) -> glyph_audio::spatial::Mode {
    match m {
        ChannelMode::Mono => glyph_audio::spatial::Mode::Mono,
        ChannelMode::Stereo => glyph_audio::spatial::Mode::Stereo,
        ChannelMode::Spatial => glyph_audio::spatial::Mode::Spatial,
    }
}

fn level_of(l: VolumeLevel) -> glyph_audio::loudness::Level {
    match l {
        VolumeLevel::Loud => glyph_audio::loudness::Level::Loud,
        VolumeLevel::Normal => glyph_audio::loudness::Level::Normal,
        VolumeLevel::Quiet => glyph_audio::loudness::Level::Quiet,
    }
}

pub fn apply_to_player(core: &Core, cfg: &Config) {
    let Some(p) = &core.player else { return };
    p.set_crossfade(Duration::from_secs(cfg.audio.crossfade_s as u64));
    p.set_loudness(level_of(cfg.audio.level), cfg.audio.normalize);
    p.set_equalizer(cfg.audio.eq_enabled, eq_gains(&cfg.audio.eq_gains));
    p.set_eq_extras(cfg.audio.eq_bass_db, cfg.audio.eq_headroom_db);
    p.set_channel_mode(channels_of(cfg.audio.channels));
    p.set_output_device(cfg.audio.output_device.clone());
    p.set_stream_quality(match cfg.audio.quality {
        glyph_core::config::AudioQuality::Auto => "auto",
        glyph_core::config::AudioQuality::Low => "low",
        glyph_core::config::AudioQuality::Standard => "standard",
        glyph_core::config::AudioQuality::High => "high",
    });
    glyph_core::cover::set_data_saver(cfg.storage.data_saver);
}
