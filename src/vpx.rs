use std::os::raw::{c_int, c_uint, c_ulong};
use std::{ptr, slice};
use vpx_sys::*;
use vpx_sys::vp8e_enc_control_id::*;
use vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_CX_FRAME_PKT;

const ABI_VERSION: c_int = 14;
const DEADLINE: c_ulong = 1;

pub struct Encoder {
    ctx: vpx_codec_ctx_t,
    width: usize,
    height: usize,
}

impl Encoder {
    pub fn new(config: Config) -> Self {
        let i = unsafe { vpx_codec_vp9_cx() };

        assert!(config.width % 2 == 0);
        assert!(config.height % 2 == 0);

        let mut c = Default::default();
        unsafe { vpx_codec_enc_config_default(i, &mut c, 0) }; //TODO: Error.

        c.g_w = config.width;
        c.g_h = config.height;
        c.g_timebase.num = config.timebase[0];
        c.g_timebase.den = config.timebase[1];
        c.rc_target_bitrate = config.bitrate;

        c.g_threads = 8;
        c.g_error_resilient = VPX_ERROR_RESILIENT_DEFAULT;

        let mut ctx = Default::default();
        unsafe {
            vpx_codec_enc_init_ver(&mut ctx, i, &c, 0, ABI_VERSION); //TODO: Error.
            vpx_codec_control_(&mut ctx, VP8E_SET_CPUUSED as _, 6 as c_int); //TODO: Error.
            vpx_codec_control_(&mut ctx, VP9E_SET_ROW_MT as _, 1 as c_int); //TODO: Error.
        }

        Self {
            ctx,
            width: config.width as usize,
            height: config.height as usize,
        }
    }

    pub fn encode(&mut self, pts: i64, data: &[u8]) -> Packets {
        assert!(2 * data.len() >= 3 * self.width * self.height);

        let mut image = Default::default();
        unsafe {
            vpx_img_wrap(
                &mut image,
                vpx_img_fmt::VPX_IMG_FMT_I420,
                self.width as _,
                self.height as _,
                1,
                data.as_ptr() as _
            );
        }

        unsafe {
            vpx_codec_encode(
                &mut self.ctx,
                &image,
                pts,
                1, // Alignment
                0, // Flags
                DEADLINE,
            ); //TODO: Error.
        }

        Packets {
            ctx: &mut self.ctx,
            iter: ptr::null(),
        }
    }

    pub fn finish(mut self) -> Finish {
        unsafe {
            vpx_codec_encode(
                &mut self.ctx,
                ptr::null(),
                -1, // PTS
                1, // Alignment
                0, // Flags
                DEADLINE,
            ); //TODO: Error.
        }

        Finish {
            enc: self,
            iter: ptr::null(),
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = vpx_codec_destroy(&mut self.ctx);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// Compressed data.
    pub data: &'a [u8],
    /// Whether the frame is a keyframe.
    pub key: bool,
    /// Presentation timestamp (in timebase units).
    pub pts: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// The width (in pixels).
    pub width: c_uint,
    /// The height (in pixels).
    pub height: c_uint,
    /// The timebase (in seconds).
    pub timebase: [c_int; 2],
    /// The target bitrate (in kilobits per second).
    pub bitrate: c_uint,
}

pub struct Packets<'a> {
    ctx: &'a mut vpx_codec_ctx_t,
    iter: vpx_codec_iter_t,
}

impl<'a> Iterator for Packets<'a> {
    type Item = Frame<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            unsafe {
                let pkt = vpx_codec_get_cx_data(self.ctx, &mut self.iter);

                if pkt.is_null() {
                    return None;
                } else if (*pkt).kind == VPX_CODEC_CX_FRAME_PKT {
                    let f = &(*pkt).data.frame;
                    return Some(Frame {
                        data: slice::from_raw_parts(f.buf as _, f.sz),
                        key: (f.flags & VPX_FRAME_IS_KEY) != 0,
                        pts: f.pts,
                    });
                } else {
                    // Ignore the packet.
                }
            }
        }
    }
}

pub struct Finish {
    enc: Encoder,
    iter: vpx_codec_iter_t,
}

impl Finish {
    pub fn next(&mut self) -> Option<Frame> {
        let mut tmp = Packets {
            ctx: &mut self.enc.ctx,
            iter: self.iter,
        };

        if let Some(packet) = tmp.next() {
            self.iter = tmp.iter;
            Some(packet)
        } else {
            unsafe {
                vpx_codec_encode(
                    tmp.ctx,
                    ptr::null(),
                    -1, // PTS
                    1, // Alignment
                    0, // Flags
                    DEADLINE,
                ); //TODO: Error.
            }

            tmp.iter = ptr::null();
            if let Some(packet) = tmp.next() {
                self.iter = tmp.iter;
                Some(packet)
            } else {
                None
            }
        }
    }
}
