use cpal;
use opus;
use std::{error, fmt, thread};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use webm::mux;

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SIZE: u32 = SAMPLE_RATE * 20 / 1000;
const BITRATE: u32 = 64_000;
const MAX_PACKET: usize = 4000;

pub fn run<T>(
    webm: &mut mux::Segment<T>,
    stop: Arc<AtomicBool>,
    mic: cpal::Device,
) -> Result<(), Error> {
    let fmt = mic.default_input_format()?;

    let _opus = opus::Encoder::new(
        SAMPLE_RATE,
        match fmt.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            x => return Err(Error::BadChannelCount(x)),
        },
        opus::Application::Audio,
    );

    let _at = webm.add_audio_track(
        SAMPLE_RATE as _,
        fmt.channels as _,
        None,
        mux::AudioCodecId::Opus,
    );

    let evs = cpal::EventLoop::new();
    let id = evs.build_input_stream(&mic, &fmt)?;
    evs.play_stream(id);

    thread::spawn(move || {
        evs.run(move |_, _| {
            if stop.load(Ordering::Acquire) {
                //TODO: End the thread.
                thread::park();
                return;
            }

            //TODO: Audio!
        });
    });

    Ok(())
}

#[derive(Debug)]
pub enum Error {
    StreamCreation(cpal::CreationError),
    DefaultFormat(cpal::DefaultFormatError),
    BadChannelCount(u16),
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
            Error::StreamCreation(e) => e.fmt(f),
            Error::DefaultFormat(e) => e.fmt(f),
            Error::BadChannelCount(n) => write!(
                f,
                "Expected 1 or 2 channels, but found {} channels.",
                n,
            ),
        }
    }
}

impl error::Error for Error {}
