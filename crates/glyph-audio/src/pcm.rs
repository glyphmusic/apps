//! Decoded audio for senders that push samples themselves (AirPlay): the
//! same probe and codecs as playback ([`crate::native`]), turned into
//! 16-bit stereo at one fixed rate. Pull-based: the caller asks for frames
//! as fast as the receiver wants them.

use crate::native::open_format;
use std::collections::VecDeque;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::units::Time;

pub struct Pcm {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    /// Rate out of [`Pcm::read`].
    out_rate: u32,
    /// Rate of the decoded stream (known after the first packet).
    in_rate: u32,
    /// Decoded stereo frames at `in_rate`, not handed out yet.
    pending: VecDeque<[f32; 2]>,
    /// Linear resampler: position between `pending[0]` and `pending[1]`.
    frac: f64,
    sample_buf: Option<SampleBuffer<f32>>,
    duration: Option<f64>,
    eof: bool,
}

impl Pcm {
    /// Opens `url` (http(s) or a local path) for output at `out_rate` Hz.
    pub fn open(url: &str, mime: Option<&str>, out_rate: u32) -> Result<Self, String> {
        let format = open_format(url, mime)?;
        let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("no audio track")?.clone();
        let duration = match (track.codec_params.n_frames, track.codec_params.time_base) {
            (Some(n), Some(tb)) if n > 0 => {
                let t = tb.calc_time(n);
                Some(t.seconds as f64 + t.frac)
            }
            _ => None,
        };
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("unsupported codec: {e}"))?;
        Ok(Self {
            format,
            decoder,
            track_id: track.id,
            out_rate,
            in_rate: track.codec_params.sample_rate.unwrap_or(out_rate),
            pending: VecDeque::new(),
            frac: 0.0,
            sample_buf: None,
            duration,
            eof: false,
        })
    }

    /// Length in seconds, when the container says.
    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    /// Jumps to `secs`; returns where it landed.
    pub fn seek(&mut self, secs: f64) -> Result<f64, String> {
        let to = SeekTo::Time { time: Time::from(secs.max(0.0)), track_id: Some(self.track_id) };
        let landed = self.format.seek(SeekMode::Accurate, to).map_err(|e| format!("seek failed: {e}"))?;
        self.decoder.reset();
        self.pending.clear();
        self.frac = 0.0;
        self.eof = false;
        let tb = self.format.tracks().iter().find(|t| t.id == self.track_id).and_then(|t| t.codec_params.time_base);
        Ok(tb.map(|tb| {
            let t = tb.calc_time(landed.actual_ts);
            t.seconds as f64 + t.frac
        })
        .unwrap_or(secs))
    }

    /// Appends up to `frames` interleaved stereo frames (`L, R`) to `out`.
    /// Returns how many it added; fewer than asked only at the end.
    pub fn read(&mut self, out: &mut Vec<i16>, frames: usize) -> usize {
        let step = self.in_rate as f64 / self.out_rate as f64;
        let mut n = 0;
        while n < frames {
            // The resampler looks at two neighbouring input frames.
            while self.pending.len() < 2 && !self.eof {
                self.decode_more();
            }
            if self.pending.len() < 2 {
                if let Some(last) = self.pending.pop_front() {
                    out.extend(last.map(to_i16));
                    n += 1;
                }
                break;
            }
            let (a, b) = (self.pending[0], self.pending[1]);
            let f = self.frac as f32;
            out.push(to_i16(a[0] + (b[0] - a[0]) * f));
            out.push(to_i16(a[1] + (b[1] - a[1]) * f));
            n += 1;
            self.frac += step;
            while self.frac >= 1.0 && !self.pending.is_empty() {
                self.pending.pop_front();
                self.frac -= 1.0;
            }
        }
        n
    }

    fn decode_more(&mut self) {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return;
                }
                Err(e) => {
                    tracing::warn!("airplay decode: {e}");
                    self.eof = true;
                    return;
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
                Err(e) => {
                    tracing::warn!("airplay decode: {e}");
                    self.eof = true;
                    return;
                }
            };
            if decoded.frames() == 0 {
                continue;
            }
            let spec = *decoded.spec();
            let sb = self.sample_buf.get_or_insert_with(|| SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
            if sb.capacity() < decoded.frames() * spec.channels.count() {
                *sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            }
            sb.copy_interleaved_ref(decoded);
            self.in_rate = spec.rate.max(1);
            let ch = spec.channels.count().max(1);
            for f in sb.samples().chunks_exact(ch) {
                // Mono to both sides; more than two channels: front left / right.
                self.pending.push_back(if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] });
            }
            return;
        }
    }
}

fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_to_16_bit() {
        assert_eq!(to_i16(1.5), 32767);
        assert_eq!(to_i16(-1.0), -32767);
        assert_eq!(to_i16(0.0), 0);
    }
}
