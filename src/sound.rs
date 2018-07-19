use cpal;
use opus;
use sample::Signal;
use sample::{interpolate, signal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{error, fmt, thread};
use webm::mux;
use webm::mux::Track;

const SAMPLE_RATE: usize = 48_000;
const FRAME_SIZE: usize = 960;
const MAX_PACKET: usize = 4000;

pub fn run<T>(
    stop: Arc<AtomicBool>,
    mic: cpal::Device,
    webm: &mut mux::Segment<T>,
    bitrate: u32,
) -> Result<(), Error> {
    let fmt = mic.default_input_format()?;

    let mut opus = SendEncoder(opus::Encoder::new(
        SAMPLE_RATE as _,
        match fmt.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            x => return Err(Error::BadChannelCount(x)),
        },
        opus::Application::Audio,
    )?);

    opus.set_bitrate(opus::Bitrate::Bits((bitrate * 1000) as _))?;

    let mut at = webm.add_audio_track(
        SAMPLE_RATE as _,
        fmt.channels as _,
        None,
        mux::AudioCodecId::Opus,
    );

    let evs = cpal::EventLoop::new();
    let id = evs.build_input_stream(&mic, &fmt)?;
    evs.play_stream(id);

    macro_rules! main {{
        $b:ident;
        $d:ident => $s:expr;
        ($i:ident, $o:ident) => $w:expr;
    } => {
        if fmt.channels == 1 {
            main! { $b; 1; $d => $s; ($i, $o) => $w; }
        } else {
            main! { $b; 2; $d => $s; ($i, $o) => $w; }
        }
    }; {
        $buffer:ident;
        $chan:expr;
        $data:ident => $signal:expr;
        ($i:ident, $o:ident) => $write:expr;
    } => {
        let target = FRAME_SIZE * $chan as usize;
        let mut $i = Vec::new();
        let mut $o = [0u8; MAX_PACKET];
        let mut p = interpolate::Linear::new([0 as _; $chan], [0 as _; $chan]);
        let mut time = 0; //TODO: Won't this drift?

        evs.run(move |_, data| {
            if stop.load(Ordering::Acquire) {
                thread::park(); //TODO: End the thread.
                return;
            }

            let $data = match data {
                cpal::StreamData::Input {
                    buffer: cpal::UnknownTypeInputBuffer::$buffer(buf),
                } => buf,
                _ => return,
            };

            let s2 = interpolate::Converter::from_hz_to_hz(
                signal::from_interleaved_samples_iter::<_, [_; $chan]>($signal),
                BorrowedInterpolator(&mut p),
                fmt.sample_rate.0 as _,
                SAMPLE_RATE as _,
            );

            for frame in s2.until_exhausted() {
                $i.extend_from_slice(&frame);
                if $i.len() >= target {
                    let n = $write;
                    at.add_frame(&$o[..n], time, true); //TODO: Which frames are key?
                    $i.clear();
                    time += 20_000_000;
                }
            }
        });
    }}

    thread::spawn(move || match fmt.data_type {
        cpal::SampleFormat::I16 => {
            main! {
                I16;
                data => data.iter().cloned();
                (i, o) => opus.encode(&i, &mut o).unwrap();
            }
        }
        cpal::SampleFormat::U16 => {
            main! {
                U16;
                data => data.iter().map(cpal::Sample::to_i16);
                (i, o) => opus.encode(&i, &mut o).unwrap();
            }
        }
        cpal::SampleFormat::F32 => {
            main! {
                F32;
                data => data.iter().cloned();
                (i, o) => opus.encode_float(&i, &mut o).unwrap();
            }
        }
    });

    Ok(())
}

#[derive(Debug)]
pub enum Error {
    Opus(opus::Error),
    StreamCreation(cpal::CreationError),
    DefaultFormat(cpal::DefaultFormatError),
    BadChannelCount(u16),
}

impl From<opus::Error> for Error {
    fn from(e: opus::Error) -> Self {
        Error::Opus(e)
    }
}

impl From<cpal::DefaultFormatError> for Error {
    fn from(e: cpal::DefaultFormatError) -> Self {
        Error::DefaultFormat(e)
    }
}

impl From<cpal::CreationError> for Error {
    fn from(e: cpal::CreationError) -> Self {
        Error::StreamCreation(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Opus(e) => e.fmt(f),
            Error::StreamCreation(e) => e.fmt(f),
            Error::DefaultFormat(e) => e.fmt(f),
            Error::BadChannelCount(n) => {
                write!(f, "Expected 1 or 2 channels, but found {} channels.", n,)
            }
        }
    }
}

impl error::Error for Error {}

//TODO: Remove this.
struct BorrowedInterpolator<'a, I: 'a>(&'a mut I);

impl<'a, I> interpolate::Interpolator for BorrowedInterpolator<'a, I>
where
    I: interpolate::Interpolator,
{
    type Frame = I::Frame;

    fn interpolate(&self, x: f64) -> Self::Frame {
        self.0.interpolate(x)
    }

    fn next_source_frame(&mut self, s: Self::Frame) {
        self.0.next_source_frame(s)
    }
}

//TODO: Remove this.
struct SendEncoder(pub opus::Encoder);
unsafe impl Send for SendEncoder {}
impl ::std::ops::Deref for SendEncoder {
    type Target = opus::Encoder;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl ::std::ops::DerefMut for SendEncoder {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
