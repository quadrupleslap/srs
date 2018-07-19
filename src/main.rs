#[macro_use]
extern crate serde_derive;

extern crate cpal;
extern crate crossbeam_channel as channel;
extern crate docopt;
extern crate opus;
extern crate quest;
extern crate sample;
extern crate scrap;
extern crate serde;
extern crate vpx_sys;
extern crate webm;

mod convert;
mod sound;
mod vpx;

use docopt::Docopt;
use scrap::{Capturer, Display};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{io, thread};
use webm::mux;
use webm::mux::Track;

const USAGE: &'static str = "
Simple WebM screen capture.

Usage:
  srs <path> [--time=<s>] [--fps=<fps>] [--bv=<kbps>] [--ba=<kbps>]
  srs (-h | --help)

Options:
  -h --help    Show this screen.
  --time=<s>   Recording duration in seconds.
  --fps=<fps>  Frames per second [default: 30].
  --bv=<kbps>  Video bitrate in kilobits per second [default: 5000].
  --ba=<kbps>  Audio bitrate in kilobits per second [default: 96].
";

#[derive(Debug, Deserialize)]
struct Args {
    arg_path: PathBuf,
    flag_time: Option<u64>,
    flag_fps: u64,
    flag_bv: u32,
    flag_ba: u32,
}

fn main() -> io::Result<()> {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    let duration = args.flag_time.map(Duration::from_secs);

    // Get the display.

    let displays = Display::all()?;

    let i = if displays.is_empty() {
        error("No displays found.");
        return Ok(());
    } else if displays.len() == 1 {
        0
    } else {
        let names: Vec<_> = displays
            .iter()
            .enumerate()
            .map(
                |(i, display)| format!("Display {} [{}x{}]", i, display.width(), display.height(),),
            )
            .collect();

        quest::ask("Which display?\n")?;
        let i = quest::choose(Default::default(), &names)?;
        println!();

        i
    };

    let display = displays.into_iter().nth(i).unwrap();

    // Get the microphone.

    let mics: Vec<_> = cpal::input_devices().collect();
    let mic = if mics.is_empty() {
        None
    } else {
        let mut names = vec!["None".into()];
        names.extend(mics.iter().map(|m| m.name()));

        quest::ask("Which audio source?\n")?;
        let i = quest::choose(Default::default(), &names)?;
        println!();

        if i == 0 {
            None
        } else {
            Some(mics.into_iter().nth(i - 1).unwrap())
        }
    };

    // Setup the recorder.

    let mut capturer = Capturer::new(display)?;
    let width = capturer.width() as u32;
    let height = capturer.height() as u32;

    // Setup the multiplexer.

    let out = match {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args.arg_path)
    } {
        Ok(file) => file,
        Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if loop {
                quest::ask("Overwrite the existing file? [yN] ")?;
                if let Some(b) = quest::yesno(false)? {
                    break b;
                }
            } {
                File::create(&args.arg_path)?
            } else {
                return Ok(());
            }
        }
        Err(e) => return Err(e.into()),
    };

    let mut webm =
        mux::Segment::new(mux::Writer::new(out)).expect("Could not initialize the multiplexer.");

    let mut vt = webm.add_video_track(width, height, None, mux::VideoCodecId::VP9);

    // Setup the encoder.

    let mut vpx = vpx::Encoder::new(vpx::Config {
        width: width,
        height: height,
        timebase: [1, 1000],
        bitrate: args.flag_bv,
    });

    // Start recording.

    let start = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));

    if let Some(mic) = mic {
        if let Err(e) = sound::run(stop.clone(), mic, &mut webm, args.flag_ba) {
            error(e);
        }
    }

    thread::spawn({
        let stop = stop.clone();
        move || {
            let _ = quest::ask("Recording! Press ⏎ to stop.");
            let _ = quest::text();
            stop.store(true, Ordering::Release);
        }
    });

    let spf = Duration::from_nanos(1_000_000_000 / args.flag_fps);
    let mut yuv = Vec::new();

    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let time = now - start;

        if Some(true) == duration.map(|d| time > d) {
            break;
        }

        match capturer.frame() {
            Ok(frame) => {
                let ms = time.as_secs() * 1000 + time.subsec_millis() as u64;

                convert::argb_to_i420(width as usize, height as usize, &frame, &mut yuv);

                for frame in vpx.encode(ms as i64, &yuv) {
                    vt.add_frame(frame.data, frame.pts as u64 * 1_000_000, frame.key);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Wait.
            }
            Err(e) => {
                println!("{}", e);
                break;
            }
        }

        let dt = now.elapsed();
        if dt < spf {
            thread::sleep(spf - dt);
        }
    }

    // End things.

    let mut frames = vpx.finish();
    while let Some(frame) = frames.next() {
        vt.add_frame(frame.data, frame.pts as u64 * 1_000_000, frame.key);
    }

    let _ = webm.finalize(None);

    Ok(())
}

fn error<S: fmt::Display>(s: S) {
    println!("\u{1B}[1;31m{}\u{1B}[0m", s);
}
