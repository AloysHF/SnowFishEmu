//! SnowFish audio: SMF (standard MIDI file) playback.
//!
//! The titles' background music is standard MIDI data (PsmPlayer-converted
//! SMF embedded in the packages or their resource containers). The guest
//! hands a `(pointer, length, mode, arg)` tuple to the platform through
//! root slot +0x38, and the platform renders the tune into a stereo
//! 44.1 kHz sample stream the frontends drain each frame.
//!
//! Feature-phone melodies are low-fi, so the tunes render once at 8 kHz
//! mono (sine + one harmonic voices with a light envelope) and resample to
//! the shared stereo output rate; this keeps headless tests fast and the
//! whole engine deterministic.

use std::collections::VecDeque;
use thiserror::Error;

pub const AUDIO_SAMPLE_RATE: u32 = 44_100;

/// Ten seconds of stereo head-room: a play call re-queues the tune and the
/// frontends drain continuously, so the queue should never hit this.
const MAX_QUEUED_SAMPLES: usize = AUDIO_SAMPLE_RATE as usize * 2 * 10;

/// MIDI playback engine: parse SMF once, render to PCM, and serve stereo
/// frames to the frontends.
#[derive(Default)]
pub struct AudioEngine {
    samples: VecDeque<i16>,
    playing: bool,
    /// The tune bytes to restart when the queue drains (`loop` plays).
    loop_source: Option<Vec<u8>>,
    /// The `(pointer, length)` of the tune currently playing, used to
    /// collapse the guest's repeated play calls for the same tune.
    current_key: Option<(u32, u32)>,
}

impl AudioEngine {
    /// Decode SMF bytes and queue the resulting stereo PCM.
    ///
    /// `loop_forever` arms the tune to restart whenever the queue drains —
    /// background music plays once on the real handset and the hardware
    /// loops it until an explicit stop.
    pub fn play_midi(&mut self, data: &[u8], loop_forever: bool) -> Result<(), AudioError> {
        let decoded = decode_midi(data)?;
        let frames = decoded.len() / 2;
        if frames == 0 {
            return Err(AudioError::Empty);
        }
        if loop_forever {
            self.loop_source = Some(data.to_vec());
        } else {
            self.loop_source = None;
        }
        self.samples.extend(decoded);
        let overflow = self.samples.len().saturating_sub(MAX_QUEUED_SAMPLES);
        if overflow > 0 {
            self.samples.drain(..overflow);
        }
        self.playing = true;
        Ok(())
    }

    /// Marks `key` as the tune currently playing; a play call for the same
    /// key is the guest re-issuing its background-music request and is
    /// answered without restarting the tune.
    pub fn set_current_key(&mut self, key: Option<(u32, u32)>) {
        self.current_key = key;
    }

    pub fn current_key(&self) -> Option<(u32, u32)> {
        self.current_key
    }

    pub fn stop(&mut self) {
        self.samples.clear();
        self.playing = false;
        self.loop_source = None;
        self.current_key = None;
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Pull up to `max_frames` stereo frames, restarting a looping tune
    /// when the queue drains.
    pub fn pull_samples(&mut self, max_frames: usize) -> Vec<i16> {
        if !self.playing {
            return Vec::new();
        }
        if self.samples.is_empty() {
            // Restart a looping tune; otherwise the cue has finished.
            if let Some(source) = self.loop_source.clone() {
                if self.play_midi(&source, true).is_err() {
                    self.playing = false;
                }
            } else {
                self.playing = false;
                return Vec::new();
            }
        }
        let take = (max_frames * 2).min(self.samples.len());
        self.samples.drain(..take).collect()
    }

    /// Number of stereo frames still buffered and ready to pull.
    pub fn buffered_frames(&self) -> usize {
        self.samples.len() / 2
    }
}

/// SMF decode/render errors.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("not a MIDI file")]
    NotMidi,
    #[error("unsupported MIDI division {0}")]
    UnsupportedDivision(u16),
    #[error("truncated MIDI structure at offset {0}")]
    Truncated(usize),
    #[error("expected MTrk chunk at offset {0}")]
    BadChunk(usize),
    #[error("MIDI file has no note events")]
    NoNotes,
    #[error("unterminated MIDI varint")]
    UnterminatedVarint,
    #[error("audio data decoded to no samples")]
    Empty,
}

/// Renders a standard MIDI file into stereo samples at [`AUDIO_SAMPLE_RATE`].
fn decode_midi(data: &[u8]) -> Result<Vec<i16>, AudioError> {
    if data.len() < 14 || &data[0..4] != b"MThd" {
        return Err(AudioError::NotMidi);
    }
    let track_count = u16::from_be_bytes(data[10..12].try_into().unwrap()) as usize;
    let division = u16::from_be_bytes(data[12..14].try_into().unwrap());
    if division == 0 || division & 0x8000 != 0 {
        return Err(AudioError::UnsupportedDivision(division));
    }
    let ticks_per_quarter = division as u64;

    #[derive(Debug, Clone, Copy)]
    struct NoteEvent {
        tick: u64,
        note: u8,
        velocity: u8,
        on: bool,
    }

    let mut events = Vec::new();
    let mut tempo_events = Vec::new();
    let mut cursor = 14usize;
    for _ in 0..track_count {
        if cursor + 8 > data.len() {
            return Err(AudioError::Truncated(cursor));
        }
        if &data[cursor..cursor + 4] != b"MTrk" {
            return Err(AudioError::BadChunk(cursor));
        }
        let length = u32::from_be_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let end = cursor.saturating_add(8).saturating_add(length);
        if end > data.len() {
            return Err(AudioError::Truncated(cursor));
        }
        let mut pos = cursor + 8;
        let mut tick = 0u64;
        let mut running_status = 0u8;
        while pos < end {
            let (delta, used) = read_varint(&data[pos..end])?;
            pos += used;
            tick = tick.saturating_add(delta);
            if pos >= end {
                break;
            }
            let mut status = data[pos];
            if status & 0x80 == 0 {
                status = running_status;
            } else {
                pos += 1;
                running_status = status;
            }
            match status & 0xF0 {
                0xC0 | 0xD0 => {
                    pos = pos.saturating_add(1);
                }
                0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => {
                    if pos + 2 > end {
                        break;
                    }
                    let data1 = data[pos];
                    let data2 = data[pos + 1];
                    pos += 2;
                    match status & 0xF0 {
                        0x80 => events.push(NoteEvent {
                            tick,
                            note: data1,
                            velocity: 0,
                            on: false,
                        }),
                        // A note-on with velocity zero is the note-off idiom.
                        0x90 if data2 == 0 => events.push(NoteEvent {
                            tick,
                            note: data1,
                            velocity: 0,
                            on: false,
                        }),
                        0x90 => events.push(NoteEvent {
                            tick,
                            note: data1,
                            velocity: data2,
                            on: true,
                        }),
                        _ => {}
                    }
                }
                0xF0 if status == 0xFF => {
                    if pos >= end {
                        break;
                    }
                    let meta = data[pos];
                    pos += 1;
                    let (length, used) = read_varint(&data[pos..end])?;
                    let length = length as usize;
                    pos += used;
                    let body = pos;
                    pos = pos.saturating_add(length);
                    if body.saturating_add(length) > end {
                        break;
                    }
                    if meta == 0x51 && length >= 3 {
                        let tempo = ((data[body] as u32) << 16)
                            | ((data[body + 1] as u32) << 8)
                            | data[body + 2] as u32;
                        tempo_events.push((tick, tempo as u64));
                    }
                }
                0xF0 => {
                    let (length, used) = read_varint(&data[pos..end])?;
                    pos = pos.saturating_add(used).saturating_add(length as usize);
                }
                _ => break,
            }
        }
        cursor = end;
    }

    events.sort_by_key(|event| event.tick);
    if events.is_empty() {
        return Err(AudioError::NoNotes);
    }
    if tempo_events.is_empty() {
        tempo_events.push((0, 500_000));
    }
    tempo_events.sort_by_key(|event| event.0);
    let max_tick = events.iter().map(|event| event.tick).max().unwrap_or(0);
    let end_tick = max_tick.saturating_add(ticks_per_quarter);

    let seconds_per_tick = |tempo: u64| tempo as f64 / ticks_per_quarter as f64 / 1_000_000.0;
    let tick_time = |tick: u64| -> f64 {
        let mut time = 0.0;
        let mut previous_tick = 0u64;
        for &(tempo_tick, tempo) in &tempo_events {
            if tempo_tick >= tick {
                break;
            }
            time += (tempo_tick - previous_tick) as f64 * seconds_per_tick(tempo);
            previous_tick = tempo_tick;
        }
        time += (tick - previous_tick) as f64
            * seconds_per_tick(tempo_events.last().map_or(500_000, |event| event.1));
        time
    };

    // Feature-phone melodies are low-fi, so render at 8 kHz and resample to
    // the shared 44.1 kHz stereo output; this keeps headless tests fast.
    const MIDI_RENDER_RATE: f64 = 8_000.0;
    let total_seconds = tick_time(end_tick).clamp(0.05, 120.0);
    let total_samples = (total_seconds * MIDI_RENDER_RATE) as usize;
    let mut samples = vec![0.0f32; total_samples];
    let mut active: Vec<(f64, f64, f64, f32)> = Vec::new(); // (frequency, start, end, amplitude)
    let mut event_index = 0usize;

    for (sample_index, output) in samples.iter_mut().enumerate() {
        let time = sample_index as f64 / MIDI_RENDER_RATE;
        while event_index < events.len() {
            let event = events[event_index];
            let event_time = tick_time(event.tick);
            if event_time > time {
                break;
            }
            event_index += 1;
            let frequency = 440.0 * 2.0f64.powf((event.note as f64 - 69.0) / 12.0);
            if event.on {
                let amplitude = event.velocity as f32 / 127.0;
                active.push((frequency, time, time + 1.0, amplitude));
            } else {
                for voice in active.iter_mut() {
                    if (voice.0 - frequency).abs() < 0.5 && voice.2 == time + 1.0 {
                        voice.2 = time;
                    }
                }
            }
        }
        active.retain(|voice| voice.2 > time);
        let mut value = 0.0f32;
        for &(frequency, start, end, amplitude) in &active {
            let age = (time - start) as f32;
            let attack = (age / 0.005).min(1.0);
            let decay = (-age * 3.0).exp();
            let release = (((end - time) as f32) * 20.0).clamp(0.0, 1.0);
            let phase = 2.0 * std::f64::consts::PI * frequency * time;
            let fundamental = phase.sin() as f32;
            let harmonic = (2.0 * phase).sin() as f32 * 0.35;
            value += (fundamental + harmonic) * attack * decay * release * amplitude * 0.18f32;
        }
        *output = value.clamp(-1.0, 1.0);
    }

    Ok(resample_to_stereo(
        &samples,
        1,
        MIDI_RENDER_RATE as u32,
        AUDIO_SAMPLE_RATE,
    ))
}

fn read_varint(data: &[u8]) -> Result<(u64, usize), AudioError> {
    let mut value = 0u64;
    let mut used = 0usize;
    loop {
        if used >= data.len() || used >= 4 {
            return Err(AudioError::UnterminatedVarint);
        }
        let byte = data[used];
        value = (value << 7) | (byte & 0x7F) as u64;
        used += 1;
        if byte & 0x80 == 0 {
            return Ok((value, used));
        }
    }
}

/// Nearest-neighbour resampler from interleaved f32 to interleaved stereo
/// i16 at the output rate.
fn resample_to_stereo(
    samples: &[f32],
    channels: usize,
    input_rate: u32,
    output_rate: u32,
) -> Vec<i16> {
    if samples.is_empty() || channels == 0 || input_rate == 0 || output_rate == 0 {
        return Vec::new();
    }
    let input_frames = samples.len() / channels;
    if input_frames == 0 {
        return Vec::new();
    }
    let output_frames =
        (input_frames as u64 * output_rate as u64).div_ceil(input_rate as u64) as usize;
    let mut output = Vec::with_capacity(output_frames * 2);

    for output_frame in 0..output_frames {
        let position = output_frame as u64 * input_rate as u64;
        let source_frame = (position / output_rate as u64) as usize;
        let next_frame = (source_frame + 1).min(input_frames - 1);
        let fraction = (position % output_rate as u64) as f32 / output_rate as f32;

        for channel in 0..2 {
            let source_channel = channel.min(channels - 1);
            let first = samples[source_frame.min(input_frames - 1) * channels + source_channel];
            let second = samples[next_frame * channels + source_channel];
            let value = first + (second - first) * fraction;
            output.push((value.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-note format-0 SMF: quarter note C4 (0x3C) at 120 BPM.
    fn tiny_midi() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 96]);
        data.extend_from_slice(b"MTrk");
        data.extend_from_slice(&[0, 0, 0, 11]);
        // delta 0, note-on C4 velocity 100; delta 96, note-off C4; end.
        data.extend_from_slice(&[
            0x00, 0x90, 0x3C, 0x64, 0x60, 0x80, 0x3C, 0x00, 0xFF, 0x2F, 0x00,
        ]);
        data
    }

    #[test]
    fn decodes_a_tune_to_stereo_samples() {
        let mut engine = AudioEngine::default();
        engine.play_midi(&tiny_midi(), false).unwrap();
        assert!(engine.playing());
        let frames = engine.buffered_frames();
        assert!(frames > 1000, "a one-second tune renders > 1000 frames");
        let pulled = engine.pull_samples(frames);
        // The tune contains a real note, so the pulled audio is not silence.
        let nonzero = pulled.iter().filter(|sample| **sample != 0).count();
        assert!(nonzero > 100, "expected audible samples, got {nonzero}");
    }

    #[test]
    fn looping_tune_restarts_when_drained() {
        let mut engine = AudioEngine::default();
        engine.play_midi(&tiny_midi(), true).unwrap();
        let frames = engine.buffered_frames();
        let _ = engine.pull_samples(frames);
        assert_eq!(engine.buffered_frames(), 0);
        // The drained looping tune restarts on the next pull.
        let again = engine.pull_samples(100);
        assert!(!again.is_empty(), "looping tune must restart");
    }

    #[test]
    fn one_shot_tune_stops_when_drained() {
        let mut engine = AudioEngine::default();
        engine.play_midi(&tiny_midi(), false).unwrap();
        let frames = engine.buffered_frames();
        let _ = engine.pull_samples(frames);
        let again = engine.pull_samples(100);
        assert!(again.is_empty(), "one-shot tune must not restart");
        assert!(!engine.playing());
    }

    #[test]
    fn rejects_non_midi_data() {
        let mut engine = AudioEngine::default();
        assert!(engine.play_midi(b"not a tune at all", false).is_err());
        assert!(!engine.playing());
    }

    #[test]
    fn varint_decodes_short_and_long_forms() {
        assert_eq!(read_varint(&[0x00]).unwrap(), (0, 1));
        assert_eq!(read_varint(&[0x7F]).unwrap(), (0x7F, 1));
        assert_eq!(read_varint(&[0x81, 0x00]).unwrap(), (0x80, 2));
        assert_eq!(
            read_varint(&[0xFF, 0xFF, 0xFF, 0x7F]).unwrap(),
            (0x0FFF_FFFF, 4)
        );
    }
}
