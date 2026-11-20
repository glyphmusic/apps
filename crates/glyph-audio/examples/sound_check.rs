//! Sound check: records what really goes to the speaker, and decodes a song
//! file as it is, so the two can be compared.
//!
//!   cargo run -p glyph-audio --example sound_check -- record <secs> <out.wav> [device name]
//!       Records the output device (Windows loopback). Waits for sound first.
//!   cargo run -p glyph-audio --example sound_check -- decode <song file> <out.wav>
//!       Decodes the file without any processing.

use rodio::cpal;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn write_wav(path: &str, rate: u32, ch: u16, samples: &[f32]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).expect("create wav"));
    let data = (samples.len() * 4) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&3u16.to_le_bytes()).unwrap(); // IEEE float
    f.write_all(&ch.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&(rate * ch as u32 * 4).to_le_bytes()).unwrap();
    f.write_all(&(ch * 4).to_le_bytes()).unwrap();
    f.write_all(&32u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data.to_le_bytes()).unwrap();
    for s in samples {
        f.write_all(&s.to_le_bytes()).unwrap();
    }
}

fn record(secs: f32, out: &str, name: Option<&str>) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = match name {
        Some(n) => host
            .output_devices()
            .unwrap()
            .find(|d| d.description().ok().is_some_and(|x| x.name().contains(n)))
            .expect("no output device with that name"),
        None => host.default_output_device().expect("no default output"),
    };
    let label = dev.description().map(|d| d.name().to_string()).unwrap_or_default();
    let cfg = dev.default_output_config().expect("output config");
    let (rate, ch) = (cfg.sample_rate(), cfg.channels());
    eprintln!("device: {label} ({rate} Hz, {ch} ch, {:?})", cfg.sample_format());
    assert_eq!(cfg.sample_format(), cpal::SampleFormat::F32, "only float mix formats");
    let want = (secs * rate as f32) as usize * ch as usize;
    let buf: Arc<Mutex<(bool, Vec<f32>)>> = Arc::new(Mutex::new((false, Vec::with_capacity(want))));
    let b = buf.clone();
    let stream = dev
        .build_input_stream::<f32, _, _>(
            cfg.config(),
            move |data: &[f32], _| {
                let mut g = b.lock().unwrap();
                if !g.0 {
                    match data.iter().position(|s| s.abs() > 0.001) {
                        Some(i) => {
                            g.0 = true;
                            let start = i - i % ch as usize;
                            g.1.extend_from_slice(&data[start..]);
                        }
                        None => return,
                    }
                } else if g.1.len() < want {
                    g.1.extend_from_slice(data);
                }
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )
        .expect("loopback stream");
    stream.play().unwrap();
    eprintln!("waiting for sound... play a song now");
    let t0 = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let g = buf.lock().unwrap();
        if g.1.len() >= want {
            break;
        }
        if !g.0 && t0.elapsed() > Duration::from_secs(300) {
            panic!("no sound within 5 minutes");
        }
    }
    drop(stream);
    let mut g = buf.lock().unwrap();
    g.1.truncate(want);
    write_wav(out, rate, ch, &g.1);
    eprintln!("wrote {out}");
}

fn decode(input: &str, out: &str) {
    use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
    let file = std::fs::File::open(input).expect("open song");
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .expect("probe");
    let mut format = probed.format;
    let track = format.default_track().expect("track").clone();
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).expect("codec");
    let (mut rate, mut ch) = (0u32, 0u16);
    let mut all = Vec::new();
    while let Ok(p) = format.next_packet() {
        if p.track_id() != track.id {
            continue;
        }
        let Ok(d) = dec.decode(&p) else { continue };
        let spec = *d.spec();
        rate = spec.rate;
        ch = spec.channels.count() as u16;
        let mut sb = SampleBuffer::<f32>::new(d.capacity() as u64, spec);
        sb.copy_interleaved_ref(d);
        all.extend_from_slice(sb.samples());
    }
    eprintln!("{input}: {rate} Hz, {ch} ch, {:.1} s, codec {:?}", all.len() as f32 / ch.max(1) as f32 / rate.max(1) as f32, track.codec_params.codec);
    write_wav(out, rate, ch, &all);
}

/// Plays a float WAV at the device rate (no processing at all) and records
/// the loopback at the same time: the control for `record`.
fn playrec(input: &str, out: &str) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let b = std::fs::read(input).expect("read wav");
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    let i = b.windows(4).position(|w| w == b"data").unwrap() + 8;
    let samples: Vec<f32> = b[i..].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    let dev = cpal::default_host().default_output_device().expect("output");
    let cfg = dev.default_output_config().unwrap();
    assert_eq!(cfg.sample_rate(), rate, "wav must be at the device rate");
    assert_eq!(cfg.channels(), 2);
    let data = Arc::new(samples);
    let pos = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (d, p) = (data.clone(), pos.clone());
    let play = dev
        .build_output_stream::<f32, _, _>(
            cfg.config(),
            move |o: &mut [f32], _| {
                let at = p.fetch_add(o.len(), std::sync::atomic::Ordering::Relaxed);
                for (k, s) in o.iter_mut().enumerate() {
                    *s = d.get(at + k).copied().unwrap_or(0.0);
                }
            },
            |e| eprintln!("play error: {e}"),
            None,
        )
        .unwrap();
    let secs = data.len() as f32 / 2.0 / rate as f32;
    let rec = std::thread::spawn({
        let out = out.to_string();
        move || record(secs - 0.5, &out, None)
    });
    std::thread::sleep(Duration::from_millis(300));
    play.play().unwrap();
    rec.join().unwrap();
}

/// Plays a song file through Glyph's real playback engine (same settings as
/// the app: levelling Normal, equalizer off, stereo) and records it.
fn engine(input: &str, start: f32, secs: f32, out: &str, eq: bool) {
    use glyph_audio::{loudness, spatial, PlayerController};
    use glyph_protocol::{PlaybackSource, ProviderId, Track};
    let p = PlayerController::new().expect("player");
    p.set_loudness(loudness::Level::Normal, true);
    p.set_equalizer(eq, [0.0; 10]);
    p.set_channel_mode(spatial::Mode::Stereo);
    p.set_volume(0.3);
    let track = Track {
        id: "sound-check".into(), source: ProviderId::YouTube, title: "sound check".into(), artist: String::new(),
        album: None, duration_ms: None, cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
    };
    let path = std::fs::canonicalize(input).unwrap().to_string_lossy().trim_start_matches(r"\?\").to_string();
    p.load(track, PlaybackSource::Stream { provider: ProviderId::YouTube, url: path, mime: Some("audio/mp4".into()) });
    p.play();
    std::thread::sleep(Duration::from_millis(1500));
    p.seek(Duration::from_secs_f32(start));
    std::thread::sleep(Duration::from_millis(1500));
    eprintln!("state: {:?}", p.state().status);
    record(secs, out, None);
    p.stop();
}

/// Runs a decoded WAV (44.1 kHz stereo) through Glyph's chain offline:
/// levelling, equalizer (off), stereo renderer, rate converter to 48 kHz.
fn chain(input: &str, out: &str) {
    use glyph_audio::{eq, loudness, resample::Resampler, spatial};
    let b = std::fs::read(input).unwrap();
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    let i = b.windows(4).position(|w| w == b"data").unwrap() + 8;
    let x: Vec<f32> = b[i..].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    loudness::set(loudness::Level::Normal, true);
    eq::set(false, [0.0; 10]);
    spatial::set_mode(spatial::Mode::Stereo);
    let shared = loudness::TrackLoudness::default();
    let mut lev = loudness::Leveller::new(rate, 2);
    let mut e = eq::Equalizer::new(rate, 2);
    let mut r = spatial::Renderer::new(rate, 2, 0);
    let mut mid = Vec::with_capacity(x.len());
    let mut fr = [0f32; 2];
    let mut o = [0f32; 2];
    let skip = std::env::var("SKIP").unwrap_or_default();
    for f in x.chunks_exact(2) {
        for c in 0..2 {
            let mut s = f[c];
            if !skip.contains("gain") { s = lev.gain(s, &shared); }
            e.set_room(lev.room_db());
            if !skip.contains("eq") { s = e.process(s); }
            if !skip.contains("limit") { s = lev.limit(s); }
            fr[c] = s;
        }
        if skip.contains("render") { o = fr; } else { r.process(&fr, &mut o); }
        mid.extend_from_slice(&o);
    }
    if skip.contains("rs") {
        // FFT-free: plain 44.1k output for comparison with a 44.1k reference.
        return write_wav(out, rate, 2, &mid);
    }
    let mut rs = Resampler::new(rate, 48_000, 2).unwrap();
    let mut it = mid.chunks_exact(2);
    let mut y = Vec::new();
    let mut fo = [0f32; 2];
    while rs.next_frame(&mut fo, |f| match it.next() { Some(v) => { f.copy_from_slice(v); true } None => false }) {
        y.extend_from_slice(&fo);
    }
    write_wav(out, 48_000, 2, &y);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("record") => record(a[2].parse().unwrap(), &a[3], a.get(4).map(String::as_str)),
        Some("decode") => decode(&a[2], &a[3]),
        Some("playrec") => playrec(&a[2], &a[3]),
        Some("chain") => chain(&a[2], &a[3]),
        Some("engine") => engine(&a[2], a[3].parse().unwrap(), a[4].parse().unwrap(), &a[5], a.get(6).is_some_and(|v| v == "eq")),
        _ => eprintln!("usage: sound_check record <secs> <out.wav> [device] | decode <file> <out.wav>"),
    }
}
