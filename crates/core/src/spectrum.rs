//! The sound playing as frequency bands, for the live tray icons. A thread captures
//! the default output through WASAPI loopback while the tray asks for bands, and
//! sleeps once it stops asking. Nothing leaves the process.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::thread::Thread;
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient,
    IAudioClient, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::core::GUID;

use crate::util::now_ms;

// Mix format tags, from mmreg.h and ksmedia.h.
const WAVE_FORMAT_IEEE_FLOAT: u32 = 3;
const WAVE_FORMAT_EXTENSIBLE: u32 = 0xFFFE;
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

/// How many bands, from bass to treble.
pub const BANDS: usize = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spectrum {
    /// Each band's loudness against the loudest band's recent peak, 0 to 1.
    pub bands: [f32; BANDS],
    /// The lowest bands together, 0 to 1.
    pub bass: f32,
    /// Overall loudness against its recent peak, 0 to 1.
    pub level: f32,
}

static LATEST: Mutex<Spectrum> = Mutex::new(Spectrum { bands: [0.0; BANDS], bass: 0.0, level: 0.0 });
/// When the tray last asked for bands.
static WANTED_MS: AtomicU64 = AtomicU64::new(0);
static THREAD: OnceLock<Thread> = OnceLock::new();

/// Capture stops this long after the last request.
const IDLE_MS: u64 = 1000;
/// Samples per analysis, about 21 ms at 48 kHz.
const WINDOW: usize = 1024;
const FRAME: Duration = Duration::from_millis(30);
/// The band edges, in Hz, spaced evenly on a log scale.
const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 12_000.0;

/// The latest bands. Starts the capture on first use.
pub fn latest() -> Spectrum {
    WANTED_MS.store(now_ms(), Relaxed);
    let thread = THREAD.get_or_init(|| {
        std::thread::Builder::new()
            .name("bartender-spectrum".into())
            .spawn(run)
            .expect("spawn spectrum")
            .thread()
            .clone()
    });
    thread.unpark();
    *LATEST.lock().unwrap()
}

fn wanted() -> bool {
    now_ms().saturating_sub(WANTED_MS.load(Relaxed)) <= IDLE_MS
}

fn run() {
    crate::util::com_init();
    loop {
        if !wanted() {
            *LATEST.lock().unwrap() = Spectrum::default();
            std::thread::park_timeout(Duration::from_secs(5));
            continue;
        }
        // Returns when the device goes away or the tray stops asking.
        if capture().is_none() {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Captures the default output until it changes or the tray stops asking.
fn capture() -> Option<()> {
    unsafe {
        let device = crate::audio::enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
        let format = client.GetMixFormat().ok()?;
        let reader = Reader::new(&*format);
        let started = reader.is_some()
            && client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, format, None)
                .is_ok();
        let rate = (*format).nSamplesPerSec as f32;
        CoTaskMemFree(Some(format as *const _));
        let reader = reader.filter(|_| started)?;
        let capture: IAudioCaptureClient = client.GetService().ok()?;
        client.Start().ok()?;

        let mut analyzer = Analyzer::new(rate);
        let mut samples = vec![0.0f32; WINDOW];
        let mut since_device_check = Instant::now();
        while wanted() {
            loop {
                let Ok(size) = capture.GetNextPacketSize() else { return Some(()) };
                if size == 0 {
                    break;
                }
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                if capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                    return Some(());
                }
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                let fresh: Vec<f32> = if silent {
                    vec![0.0; frames as usize]
                } else {
                    (0..frames as usize).map(|f| reader.mono(data, f)).collect()
                };
                let _ = capture.ReleaseBuffer(frames);
                // Keep the newest `WINDOW` samples.
                let keep = fresh.len().min(WINDOW);
                samples.drain(..keep);
                samples.extend_from_slice(&fresh[fresh.len() - keep..]);
            }
            *LATEST.lock().unwrap() = analyzer.update(&samples);
            // Loopback goes quiet rather than failing when the default output changes.
            if since_device_check.elapsed() > Duration::from_secs(2) {
                since_device_check = Instant::now();
                let now = crate::audio::enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                if now.GetId().ok().map(|id| id.to_string().ok()) != device.GetId().ok().map(|id| id.to_string().ok()) {
                    let _ = client.Stop();
                    return Some(());
                }
            }
            std::thread::sleep(FRAME);
        }
        let _ = client.Stop();
        Some(())
    }
}

/// Reads one frame of the mix format as a mono sample.
struct Reader {
    channels: usize,
    float: bool,
    bytes: usize,
}

impl Reader {
    unsafe fn new(format: &WAVEFORMATEX) -> Option<Reader> {
        let tag = format.wFormatTag as u32;
        let float = match tag {
            WAVE_FORMAT_IEEE_FLOAT => true,
            WAVE_FORMAT_EXTENSIBLE => {
                let ext = unsafe { (format as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE).read_unaligned() };
                let sub = ext.SubFormat;
                sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            }
            _ => false,
        };
        let bits = format.wBitsPerSample;
        // 32-bit float, or 16-bit integers. The shared mix format is float in practice.
        let ok = if float { bits == 32 } else { bits == 16 };
        ok.then_some(Reader { channels: format.nChannels.max(1) as usize, float, bytes: bits as usize / 8 })
    }

    unsafe fn mono(&self, data: *const u8, frame: usize) -> f32 {
        let mut sum = 0.0;
        for c in 0..self.channels {
            let at = unsafe { data.add((frame * self.channels + c) * self.bytes) };
            sum += if self.float {
                unsafe { (at as *const f32).read_unaligned() }
            } else {
                unsafe { (at as *const i16).read_unaligned() as f32 / 32768.0 }
            };
        }
        sum / self.channels as f32
    }
}

/// Turns samples into bands, scaled against recent peaks so quiet songs fill the range.
struct Analyzer {
    /// The first FFT bin of each band, and one past the last band's end.
    edges: [usize; BANDS + 1],
    window: Vec<f32>,
    /// Recent peaks of the bands together, the bass, and the level.
    peaks: [f32; 3],
    shown: Spectrum,
}

/// How fast a band's peak falls each frame, so a quiet song still fills the range.
const PEAK_FALL: f32 = 0.995;
/// Below this a band stays dark, so noise doesn't dance.
const FLOOR: f32 = 0.02;

impl Analyzer {
    fn new(rate: f32) -> Analyzer {
        let bin = |hz: f32| ((hz / rate * WINDOW as f32).round() as usize).clamp(1, WINDOW / 2 - 1);
        let mut edges = [0; BANDS + 1];
        for (i, edge) in edges.iter_mut().enumerate() {
            let hz = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f32 / BANDS as f32);
            *edge = bin(hz);
        }
        // Low bands can share a bin; give each at least one.
        for i in 1..=BANDS {
            edges[i] = edges[i].max(edges[i - 1] + 1);
        }
        let window = (0..WINDOW).map(|n| 0.5 - 0.5 * (TAU * n as f32 / (WINDOW - 1) as f32).cos()).collect();
        Analyzer { edges, window, peaks: [FLOOR; 3], shown: Spectrum::default() }
    }

    fn update(&mut self, samples: &[f32]) -> Spectrum {
        let mut re: Vec<f32> = samples.iter().zip(&self.window).map(|(s, w)| s * w).collect();
        let mut im = vec![0.0; WINDOW];
        fft(&mut re, &mut im);
        let magnitude = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt() / WINDOW as f32;
        let mut raw = [0.0f32; BANDS + 2];
        for b in 0..BANDS {
            let (from, to) = (self.edges[b], self.edges[b + 1].min(WINDOW / 2));
            let peak = (from..to.max(from + 1)).map(magnitude).fold(0.0, f32::max);
            // Compress, and lift the treble a little, since music has less of it.
            raw[b] = (1.0 + 60.0 * peak).ln() * (1.0 + 0.25 * b as f32);
        }
        raw[BANDS] = (raw[0] + raw[1]) / 2.0;
        raw[BANDS + 1] = (1.0 + 60.0 * (samples.iter().map(|s| s * s).sum::<f32>() / WINDOW as f32).sqrt()).ln();
        // The bands share one peak, so the spectrum keeps its shape.
        let loudest = raw[..BANDS].iter().fold(0.0f32, |a, b| a.max(*b));
        for (peak, now) in self.peaks.iter_mut().zip([loudest, raw[BANDS], raw[BANDS + 1]]) {
            *peak = now.max(*peak * PEAK_FALL).max(FLOOR);
        }
        let scale = |v: f32, peak: f32| if v < FLOOR { 0.0 } else { (v / peak).clamp(0.0, 1.0) };
        let mut scaled = [0.0f32; BANDS + 2];
        for b in 0..BANDS {
            scaled[b] = scale(raw[b], self.peaks[0]);
        }
        scaled[BANDS] = scale(raw[BANDS], self.peaks[1]);
        scaled[BANDS + 1] = scale(raw[BANDS + 1], self.peaks[2]);
        // Rise at once and fall gently, so motion reads as beats rather than flicker.
        let ease = |shown: f32, now: f32| if now > shown { now } else { shown * 0.7 + now * 0.3 };
        for b in 0..BANDS {
            self.shown.bands[b] = ease(self.shown.bands[b], scaled[b]);
        }
        self.shown.bass = ease(self.shown.bass, scaled[BANDS]);
        self.shown.level = ease(self.shown.level, scaled[BANDS + 1]);
        self.shown
    }
}

/// An in-place radix-2 FFT. The length must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (w_re, w_im) = ((angle * k as f32).cos(), (angle * k as f32).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let t_re = re[b] * w_re - im[b] * w_im;
                let t_im = re[b] * w_im + im[b] * w_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 Hz tone lights the bass bands and leaves the treble dark.
    #[test]
    fn a_low_tone_lights_the_bass() {
        let rate = 48_000.0;
        let mut analyzer = Analyzer::new(rate);
        let tone: Vec<f32> = (0..WINDOW).map(|n| 0.5 * (TAU * 100.0 * n as f32 / rate).sin()).collect();
        let spectrum = analyzer.update(&tone);
        let loudest = (0..BANDS).max_by(|a, b| spectrum.bands[*a].total_cmp(&spectrum.bands[*b])).unwrap();
        assert!(loudest <= 2, "loudest band {loudest}: {:?}", spectrum.bands);
        assert!(spectrum.bands[BANDS - 1] < 0.3, "treble {:?}", spectrum.bands);
        assert!(spectrum.bass > 0.5);
    }

    /// Prints the bands of whatever plays for three seconds:
    /// `cargo test -p audio_bartender_core spectrum_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn spectrum_probe() {
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            let s = latest();
            let bars: String = s.bands.iter().map(|b| [' ', '.', ':', '|', '#'][(b * 4.0).round() as usize]).collect();
            println!("[{bars}] bass {:.2} level {:.2}", s.bass, s.level);
        }
    }
}
