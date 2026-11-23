//! Windows' sound, through the `wasapi` crate: what plays on a set of
//! speakers (loopback), a microphone, or one program's sound with or
//! without the rest (process loopback, Windows 10 2004 and later).
//!
//! Everything is asked for as 48 kHz stereo floats and Windows converts;
//! everything is polled, never waited on, so a silent device costs the
//! mixer nothing. Windows keeps what it captured until it is read, up to
//! the size of the buffer asked for: every stream asks for a roomy one,
//! because what does not fit is lost, and heard as a gap.

use super::{Layout, Opener, Stream, Tap};
use std::collections::VecDeque;
use wasapi::{AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, SampleType, StreamMode, WasapiError, WaveFormat};

/// How much Windows holds for us between two reads, in 100 ns: half a
/// second, far more than the mixer's 10 ms, so a slow moment (the game
/// taking every processor) loses nothing.
const BUFFER_HNS: i64 = 5_000_000;
/// Packets read in one go at most, so a flood cannot hold the mixer.
const MAX_PACKETS: usize = 200;

/// Make the calling thread ready for the sound interfaces. Harmless to
/// call again; a thread that already chose another COM mode keeps it.
pub fn prepare_thread() {
    let result = wasapi::initialize_mta();
    if result.is_err() {
        crate::note!("sound: COM could not be initialised on this thread: {result:?}");
    }
}

fn format() -> WaveFormat {
    WaveFormat::new(32, 32, SampleType::Float, crate::mixer::RATE as usize, crate::mixer::CHANNELS, None)
}

pub struct Wasapi {
    enumerator: Option<DeviceEnumerator>,
}

impl Wasapi {
    pub fn new() -> Self {
        prepare_thread();
        Self { enumerator: None }
    }

    fn enumerator(&mut self) -> Result<&DeviceEnumerator, WasapiError> {
        if self.enumerator.is_none() {
            self.enumerator = Some(DeviceEnumerator::new()?);
        }
        // Just filled in above.
        self.enumerator.as_ref().ok_or(WasapiError::ClientNotInit)
    }

    /// Speakers (their loopback) or a microphone, by id or the default.
    fn endpoint(&mut self, id: Option<&str>, output: bool) -> Result<Capture, WasapiError> {
        let direction = if output { Direction::Render } else { Direction::Capture };
        let enumerator = self.enumerator()?;
        let device = match id {
            Some(id) => enumerator.get_device(id)?,
            None => enumerator.get_default_device(direction)?,
        };
        let mut client = device.get_iaudioclient()?;
        // Capturing from a render device is what makes it a loopback.
        let converted = StreamMode::PollingShared { autoconvert: true, buffer_duration_hns: BUFFER_HNS };
        match client.initialize_client(&format(), Direction::Capture, converted, None) {
            Ok(()) => Capture::start(client, None, Layout::OURS),
            Err(first) => {
                // Windows would not convert for this device: take what the
                // device mixes in, and convert here what can be converted.
                let mut client = device.get_iaudioclient()?;
                let native = client.get_mixformat()?;
                let layout = layout_of(&native).ok_or(first)?;
                let plain = StreamMode::PollingShared { autoconvert: false, buffer_duration_hns: BUFFER_HNS };
                client.initialize_client(&native, Direction::Capture, plain, None)?;
                crate::note!("sound: a device is read in its own format ({layout:?}) and converted here");
                Capture::start(client, None, layout)
            }
        }
    }

    /// One program's tree, or everything but it.
    fn tree(pid: u32, include: bool) -> Result<Capture, WasapiError> {
        // This kind of client is event driven, and the event must exist
        // even though we poll. Left to itself (a buffer of 0) Windows
        // gives it room for a few hundredths of a second, which a poll
        // that comes a little late overflows: the sound then skips.
        let open = |buffer_duration_hns: i64| -> Result<Capture, WasapiError> {
            let mut client = AudioClient::new_application_loopback_client(pid, include)?;
            let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns };
            client.initialize_client(&format(), Direction::Capture, mode, None)?;
            let event = client.set_get_eventhandle()?;
            Capture::start(client, Some(event), Layout::OURS)
        };
        open(BUFFER_HNS).or_else(|err| {
            crate::note!("sound: program {pid} could not be opened with a roomy buffer ({err}); with Windows' own");
            open(0)
        })
    }
}

impl Default for Wasapi {
    fn default() -> Self {
        Self::new()
    }
}

impl Opener for Wasapi {
    fn open(&mut self, tap: &Tap) -> Result<Box<dyn Stream>, String> {
        let capture = match tap {
            Tap::Output(id) => self.endpoint(id.as_deref(), true),
            Tap::Input(id) => self.endpoint(id.as_deref(), false),
            Tap::Tree { pid, include } => Self::tree(*pid, *include),
        };
        capture.map(|c| Box::new(c) as Box<dyn Stream>).map_err(|err| err.to_string())
    }

    fn default_id(&mut self, output: bool) -> Option<String> {
        let direction = if output { Direction::Render } else { Direction::Capture };
        self.enumerator().ok()?.get_default_device(direction).ok()?.get_id().ok()
    }
}

/// The layout of a device's own format, when it is one that can be turned
/// into ours without changing its speed.
fn layout_of(format: &WaveFormat) -> Option<Layout> {
    let float = matches!(format.get_subformat(), Ok(SampleType::Float));
    let layout = Layout { channels: format.get_nchannels() as usize, bytes: format.get_bitspersample() as usize / 8, float };
    (format.get_samplespersec() == crate::mixer::RATE && layout.readable()).then_some(layout)
}

struct Capture {
    client: AudioClient,
    capture: AudioCaptureClient,
    _event: Option<Handle>,
    layout: Layout,
    bytes: VecDeque<u8>,
}

impl Capture {
    fn start(client: AudioClient, event: Option<Handle>, layout: Layout) -> Result<Self, WasapiError> {
        // Not every kind of client tells; the log is the better for it when one does.
        // (One program's stream answers with nonsense: that is not said.)
        if let Ok(frames) = client.get_buffer_size()
            && frames <= 10 * crate::mixer::RATE
        {
            crate::note!("sound: a stream opened, holding up to {} ms between reads", frames as u64 * 1000 / crate::mixer::RATE as u64);
        }
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        Ok(Self { client, capture, _event: event, layout, bytes: VecDeque::new() })
    }
}

impl Stream for Capture {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), String> {
        for _ in 0..MAX_PACKETS {
            match self.capture.get_next_packet_size() {
                Ok(Some(frames)) if frames > 0 => {}
                Ok(_) => break,
                Err(err) => return Err(err.to_string()),
            }
            self.capture.read_from_device_to_deque(&mut self.bytes).map_err(|err| err.to_string())?;
        }
        let whole = self.bytes.len() - self.bytes.len() % self.layout.frame();
        let bytes = self.bytes.make_contiguous();
        self.layout.convert(&bytes[..whole], out);
        self.bytes.drain(..whole);
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

/// The active speakers (named with `.monitor` at the end) and microphones,
/// by their Windows endpoint ids, which hold no spaces.
pub fn devices() -> Vec<(String, String)> {
    prepare_thread();
    let mut out = Vec::new();
    let Ok(enumerator) = DeviceEnumerator::new() else { return out };
    for (direction, suffix) in [(Direction::Render, super::MONITOR_SUFFIX), (Direction::Capture, "")] {
        let Ok(collection) = enumerator.get_device_collection(direction) else { continue };
        for index in 0..collection.get_nbr_devices().unwrap_or(0) {
            let Ok(device) = collection.get_device_at_index(index) else { continue };
            if let (Ok(id), Ok(label)) = (device.get_id(), device.get_friendlyname()) {
                out.push((format!("{id}{suffix}"), label));
            }
        }
    }
    out
}
