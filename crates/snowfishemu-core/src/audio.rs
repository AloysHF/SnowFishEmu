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

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

pub const AUDIO_SAMPLE_RATE: u32 = 44_100;
pub(crate) const MAX_MIDI_BYTES: usize = 1024 * 1024;

/// MIDI playback engine: parse SMF once, render to PCM, and serve stereo
/// frames to the frontends.
#[derive(Default)]
pub struct AudioEngine {
    playback: Option<Playback>,
    revision: u64,
}

struct Playback {
    state: AudioSnapshot,
    samples: Arc<Vec<i16>>,
}

/// The SMF and interleaved sample offset suffice to rebuild deterministic PCM.
#[derive(Serialize, Deserialize)]
pub(crate) struct AudioSnapshot {
    midi: Vec<u8>,
    looping: bool,
    position: usize,
}

impl Playback {
    fn render(state: AudioSnapshot) -> Result<Self, AudioError> {
        if state.midi.len() > MAX_MIDI_BYTES {
            return Err(AudioError::TooLarge);
        }
        let samples = Arc::new(decode_midi(&state.midi)?);
        Self::with_samples(state, samples)
    }

    fn with_samples(state: AudioSnapshot, samples: Arc<Vec<i16>>) -> Result<Self, AudioError> {
        if samples.is_empty() {
            return Err(AudioError::Empty);
        }
        if !state.position.is_multiple_of(2)
            || state.position > samples.len()
            || (state.position == samples.len() && !state.looping)
        {
            return Err(AudioError::InvalidPosition);
        }
        Ok(Self { state, samples })
    }
}

impl AudioEngine {
    /// Replace the current tune and report whether playback changed.
    pub fn play_midi(&mut self, data: &[u8], looping: bool) -> Result<bool, AudioError> {
        if self
            .playback
            .as_ref()
            .is_some_and(|tune| tune.state.looping == looping && tune.state.midi == data)
        {
            return Ok(false);
        }
        self.playback = Some(Playback::render(AudioSnapshot {
            midi: data.to_vec(),
            looping,
            position: 0,
        })?);
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn stop(&mut self) {
        self.playback = None;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Identifies changes that invalidate pending frontend samples.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn snapshot(&self) -> Option<&AudioSnapshot> {
        self.playback.as_ref().map(|tune| &tune.state)
    }

    pub(crate) fn restored(&self, snapshot: Option<AudioSnapshot>) -> Result<Self, AudioError> {
        let playback = snapshot
            .map(|state| {
                // Rewinds and runahead typically revisit the current tune.
                match self.playback.as_ref() {
                    Some(current) if current.state.midi == state.midi => {
                        Playback::with_samples(state, Arc::clone(&current.samples))
                    }
                    _ => Playback::render(state),
                }
            })
            .transpose()?;
        Ok(Self {
            playback,
            revision: self.revision.wrapping_add(1),
        })
    }

    pub fn playing(&self) -> bool {
        self.playback.is_some()
    }

    /// Pull stereo frames, filling the request across loop boundaries.
    pub fn pull_samples(&mut self, max_frames: usize) -> Vec<i16> {
        let Some(tune) = self.playback.as_mut() else {
            return Vec::new();
        };
        let position = &mut tune.state.position;
        let available = (tune.samples.len() - *position) / 2;
        let frames = if tune.state.looping {
            max_frames
        } else {
            max_frames.min(available)
        };
        let mut output = Vec::with_capacity(frames * 2);
        while output.len() / 2 < frames {
            if *position == tune.samples.len() {
                *position = 0;
            }
            let take = (frames - output.len() / 2).min((tune.samples.len() - *position) / 2);
            let end = *position + take * 2;
            output.extend_from_slice(&tune.samples[*position..end]);
            *position = end;
        }
        if *position == tune.samples.len() && !tune.state.looping {
            self.playback = None;
        }
        output
    }

    /// Number of stereo frames until the end of the current tune.
    pub fn buffered_frames(&self) -> usize {
        self.playback
            .as_ref()
            .map_or(0, |tune| (tune.samples.len() - tune.state.position) / 2)
    }
}

/// SMF decode/render errors.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("MIDI data exceeds the size limit")]
    TooLarge,
    #[error("invalid saved MIDI playback position")]
    InvalidPosition,
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
        channel: u8,
        note: u8,
        velocity: u8,
    }

    struct Voice {
        channel: u8,
        note: u8,
        frequency: f64,
        start: f64,
        release: Option<f64>,
        amplitude: f32,
    }

    let mut events = Vec::new();
    let mut tempo_events = Vec::new();
    let mut end_tick = 0;
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
                    if matches!(status & 0xF0, 0x80 | 0x90) {
                        events.push(NoteEvent {
                            tick,
                            channel: status & 0x0F,
                            note: data1,
                            velocity: if status & 0xF0 == 0x80 { 0 } else { data2 },
                        });
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
                    if meta == 0x2F {
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
        end_tick = end_tick.max(tick);
        cursor = end;
    }

    events.sort_by_key(|event| event.tick);
    if events.is_empty() {
        return Err(AudioError::NoNotes);
    }
    tempo_events.sort_by_key(|event| event.0);

    // Walk the tempo map alongside the sorted notes. Each interval uses
    // the last tempo effective at its start, or 120 BPM before any change.
    let mut changes = tempo_events.into_iter().peekable();
    let mut clock_tick = 0;
    let mut clock_time = 0.0;
    let mut tempo = 500_000;
    let seconds_per_tick = |tempo: u64| tempo as f64 / (ticks_per_quarter as f64 * 1_000_000.0);
    let mut time_at = |tick: u64| {
        while let Some(&(change_tick, new_tempo)) = changes.peek() {
            if change_tick > tick {
                break;
            }
            clock_time += (change_tick - clock_tick) as f64 * seconds_per_tick(tempo);
            clock_tick = change_tick;
            tempo = new_tempo;
            changes.next();
        }
        clock_time + (tick - clock_tick) as f64 * seconds_per_tick(tempo)
    };
    let events: Vec<_> = events
        .into_iter()
        .map(|event| (time_at(event.tick), event))
        .collect();

    // Feature-phone melodies are low-fi, so render at 8 kHz and resample to
    // the shared 44.1 kHz stereo output; this keeps headless tests fast.
    const MIDI_RENDER_RATE: f64 = 8_000.0;
    let total_seconds = time_at(end_tick).clamp(0.05, 120.0);
    let total_samples = (total_seconds * MIDI_RENDER_RATE) as usize;
    let mut samples = vec![0.0f32; total_samples];
    const RELEASE_SECONDS: f64 = 0.05;
    let mut active: Vec<Voice> = Vec::new();
    let mut event_index = 0usize;

    for (sample_index, output) in samples.iter_mut().enumerate() {
        let time = sample_index as f64 / MIDI_RENDER_RATE;
        while event_index < events.len() {
            let (event_time, event) = events[event_index];
            if event_time > time {
                break;
            }
            event_index += 1;
            // MIDI uses a zero-velocity note-on as another form of note-off.
            if event.velocity > 0 {
                active.push(Voice {
                    channel: event.channel,
                    note: event.note,
                    frequency: 440.0 * 2.0f64.powf((event.note as f64 - 69.0) / 12.0),
                    start: time,
                    release: None,
                    amplitude: event.velocity as f32 / 127.0,
                });
            } else if let Some(voice) = active.iter_mut().find(|voice| {
                voice.channel == event.channel
                    && voice.note == event.note
                    && voice.release.is_none()
            }) {
                voice.release = Some(time);
            }
        }
        active.retain(|voice| {
            voice
                .release
                .is_none_or(|start| time < start + RELEASE_SECONDS)
        });
        let mut value = 0.0f32;
        for voice in &active {
            let elapsed = time - voice.start;
            let age = elapsed as f32;
            let attack = (age / 0.005).min(1.0);
            let decay = 0.35 + 0.65 * (-age * 3.0).exp();
            let release = voice
                .release
                .map_or(1.0, |start| (1.0 - (time - start) / RELEASE_SECONDS) as f32);
            let phase = 2.0 * std::f64::consts::PI * voice.frequency * elapsed;
            let fundamental = phase.sin() as f32;
            let harmonic = (2.0 * phase).sin() as f32 * 0.35;
            value +=
                (fundamental + harmonic) * attack * decay * release * voice.amplitude * 0.18f32;
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

/// Linearly resamples interleaved f32 audio to stereo i16.
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

    fn midi(track: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 96]);
        data.extend_from_slice(b"MTrk");
        data.extend_from_slice(&(track.len() as u32).to_be_bytes());
        data.extend_from_slice(track);
        data
    }

    /// A one-note format-0 SMF: quarter note C4 (0x3C) at 120 BPM.
    fn tiny_midi() -> Vec<u8> {
        // delta 0, note-on C4 velocity 100; delta 96, note-off C4; end.
        midi(&[0, 0x90, 60, 100, 96, 0x80, 60, 0, 0, 0xFF, 0x2F, 0])
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
    fn new_tune_starts_at_its_first_frame() {
        let mut engine = AudioEngine::default();
        engine.play_midi(&tiny_midi(), false).unwrap();
        engine.pull_samples(500);
        let next = midi(&[0, 0x90, 69, 100, 96, 0x80, 69, 0, 0, 0xFF, 0x2F, 0]);
        let expected = decode_midi(&next).unwrap();
        engine.play_midi(&next, false).unwrap();
        assert!(engine.pull_samples(500) == expected[..1000]);
    }

    #[test]
    fn tune_over_ten_seconds_keeps_its_opening_and_later_notes() {
        let track = midi(&[
            0, 0x90, 60, 100, 96, 0x80, 60, 0, 0x8F, 0, 0x90, 69, 100, 96, 0x80, 69, 0, 0, 0xFF,
            0x2F, 0,
        ]);
        let mut engine = AudioEngine::default();
        engine.play_midi(&track, false).unwrap();
        let rate = AUDIO_SAMPLE_RATE as usize;
        let pcm = engine.pull_samples(12 * rate);
        assert!(pcm.len() >= 22 * rate);
        assert!(pcm[..rate].iter().any(|&sample| sample != 0));
        assert!(pcm[21 * rate..].iter().any(|&sample| sample != 0));
    }

    #[test]
    fn tempo_changes_apply_only_to_their_own_intervals() {
        let tune = midi(&[
            96, 0x90, 60, 100, 48, 0x80, 60, 0, 48, 0xFF, 0x51, 3, 0x0F, 0x42, 0x40, 96, 0xFF,
            0x51, 3, 0x03, 0xD0, 0x90, 96, 0x90, 69, 100, 96, 0x80, 69, 0, 0, 0xFF, 0x2F, 0,
        ]);
        let pcm = decode_midi(&tune).unwrap();
        let rate = AUDIO_SAMPLE_RATE as usize;
        let (frames, _) = pcm.as_chunks::<2>();
        let first_note = frames.iter().position(|[left, _]| *left != 0).unwrap();
        let late_note = frames[2 * rate..]
            .iter()
            .position(|[left, _]| *left != 0)
            .unwrap()
            + 2 * rate;
        assert!((rate / 2..rate / 2 + 32).contains(&first_note));
        assert!((9 * rate / 4..9 * rate / 4 + 32).contains(&late_note));
    }

    #[test]
    fn notes_sustain_until_note_off_then_release() {
        let tune = midi(&[0, 0x90, 60, 100, 0x82, 0x20, 0x80, 60, 0, 72, 0xFF, 0x2F, 0]);
        let pcm = decode_midi(&tune).unwrap();
        let rate = AUDIO_SAMPLE_RATE as usize;
        assert!(pcm[22 * rate / 10..24 * rate / 10].iter().any(|&s| s != 0));
        assert_eq!(pcm.len() / 2, (15 * rate).div_ceil(8));
        assert!(pcm[32 * rate / 10..].iter().all(|&s| s == 0));
    }

    #[test]
    fn note_off_only_releases_the_matching_channel() {
        let both = midi(&[
            0, 0x90, 60, 100, 0, 0x91, 60, 100, 96, 0x80, 60, 0, 96, 0x91, 60, 0, 0, 0xFF, 0x2F, 0,
        ]);
        let channel_two = midi(&[0, 0x91, 60, 100, 0x81, 0x40, 0x91, 60, 0, 0, 0xFF, 0x2F, 0]);
        let mixed = decode_midi(&both).unwrap();
        let solo = decode_midi(&channel_two).unwrap();
        let rate = AUDIO_SAMPLE_RATE as usize;
        assert!(mixed[6 * rate / 5..8 * rate / 5] == solo[6 * rate / 5..8 * rate / 5]);
    }

    #[test]
    fn a_single_pull_crosses_multiple_loop_boundaries() {
        let mut engine = AudioEngine::default();
        let tune = tiny_midi();
        let pcm = decode_midi(&tune).unwrap();
        engine.play_midi(&tune, true).unwrap();
        let frames = engine.buffered_frames();
        engine.pull_samples(frames - 100);
        let samples = engine.pull_samples(frames + 200);
        let expected = [&pcm[pcm.len() - 200..], &pcm, &pcm[..200]].concat();
        assert_eq!(samples.len(), expected.len());
        assert!(samples == expected);
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
