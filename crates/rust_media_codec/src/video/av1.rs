//! AV1 video decoder implementation using dav1d
//!
//! Provides AV1 decoding via the dav1d crate (MIT bindings to libdav1d).
//! libdav1d is the reference AV1 decoder from VideoLAN, BSD-2-Clause licensed.
//!
//! # System Requirements
//!
//! Requires libdav1d to be installed on the system:
//! - macOS: `brew install dav1d`
//! - Debian/Ubuntu: `apt-get install libdav1d-dev`
//!
//! # Profile Support
//!
//! - **Main Profile** (8-bit) — fully supported
//! - **Main 10** (10-bit, HDR) — currently rejected; the decoder layer is
//!   8-bit only end-to-end
//!
//! # Bitstream Format
//!
//! AV1 packets are expected to contain raw OBU (Open Bitstream Unit) data,
//! which is the standard format used by both MP4 (`av01` sample entry) and
//! WebM/MKV (`V_AV1` codec ID) containers.

use dav1d::{Decoder as Dav1dDecoder, Error as Dav1dError, PixelLayout, PlanarImageComponent};
use rust_media_core::{
    Decoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};
use std::collections::VecDeque;

/// AV1 video decoder using dav1d
pub struct Av1Decoder {
    stream_info: StreamInfo,
    decoder: Dav1dDecoder,
    buffered_frames: VecDeque<Frame>,
    flushed: bool,
    /// Monotonically increasing counter used as the dav1d timestamp so we can
    /// correlate output pictures with their source packets.
    next_timestamp: i64,
    /// Maps `next_timestamp` values back to the original packet PTS.
    pts_map: VecDeque<(i64, Option<i64>)>,
}

impl Av1Decoder {
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "av1" && stream_info.codec != "av01" {
            return Err(Error::Unsupported(format!(
                "Expected av1 codec, got {}",
                stream_info.codec
            )));
        }

        let decoder = Dav1dDecoder::new()
            .map_err(|e| Error::Decode(format!("Failed to create AV1 decoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            decoder,
            buffered_frames: VecDeque::new(),
            flushed: false,
            next_timestamp: 0,
            pts_map: VecDeque::new(),
        })
    }

    /// Drain all available pictures from the dav1d decoder into the output queue.
    fn drain_pictures(&mut self) -> Result<()> {
        loop {
            match self.decoder.get_picture() {
                Ok(picture) => {
                    let frame = self.picture_to_frame(picture)?;
                    self.buffered_frames.push_back(frame);
                }
                Err(Dav1dError::Again) => break,
                Err(e) => {
                    return Err(Error::Decode(format!("dav1d get_picture failed: {:?}", e)));
                }
            }
        }
        Ok(())
    }

    /// Convert a dav1d Picture into a rust_media_core Frame, accounting for
    /// per-row stride.
    fn picture_to_frame(&mut self, picture: dav1d::Picture) -> Result<Frame> {
        // dav1d's `bit_depth()` returns the actual bit depth (8, 10, or 12),
        // not the storage size. 8-bit content uses 1 byte per sample; 10/12-bit
        // content uses 2 bytes per sample (little-endian u16, lower bits significant).
        let bit_depth = picture.bit_depth();
        let pixel_format = match bit_depth {
            8 => PixelFormat::YUV420P,
            10 => PixelFormat::YUV420P10LE,
            12 => {
                return Err(Error::Unsupported(
                    "12-bit AV1 (Profile 2) is not yet supported. \
                    The codec layer and pixel format support are 8-bit/10-bit only."
                        .to_string(),
                ));
            }
            other => {
                return Err(Error::Unsupported(format!(
                    "AV1 decoder: unexpected bit depth {}",
                    other
                )));
            }
        };

        // We only support YUV 4:2:0 (I420) for now.
        if picture.pixel_layout() != PixelLayout::I420 {
            return Err(Error::Unsupported(format!(
                "AV1 decoder only supports YUV 4:2:0 (I420), got {:?}",
                picture.pixel_layout()
            )));
        }

        let width = picture.width() as usize;
        let height = picture.height() as usize;

        let mut frame = Frame::new_video(width, height, pixel_format);

        // Map dav1d's monotonic timestamp back to the source packet PTS.
        let ts = picture.timestamp();
        let pts = match ts {
            Some(t) => {
                // Find and consume the corresponding entry from pts_map.
                let mut found = None;
                while let Some(&(map_ts, map_pts)) = self.pts_map.front() {
                    if map_ts == t {
                        found = Some(map_pts);
                        self.pts_map.pop_front();
                        break;
                    } else if map_ts < t {
                        // Stale entry (lost packet), drop it
                        self.pts_map.pop_front();
                    } else {
                        break;
                    }
                }
                found.unwrap_or(None)
            }
            None => None,
        };
        frame.set_pts(pts);

        // Bytes per pixel: 1 for 8-bit, 2 for 10/12-bit storage
        let bps = if bit_depth == 8 { 1 } else { 2 };

        // Copy each plane, honoring stride (which may be > width*bps for SIMD alignment).
        // dav1d stride is in bytes; we copy `plane_width * bps` bytes per row.
        let copy_plane = |frame: &mut Frame,
                          plane_idx: usize,
                          src: &[u8],
                          src_stride: usize,
                          plane_width: usize,
                          plane_height: usize|
         -> Result<()> {
            let row_bytes = plane_width * bps;
            let dst = frame
                .plane_mut(plane_idx)
                .ok_or_else(|| Error::InvalidData(format!("missing plane {}", plane_idx)))?;
            for row in 0..plane_height {
                let src_off = row * src_stride;
                let dst_off = row * row_bytes;
                dst[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&src[src_off..src_off + row_bytes]);
            }
            Ok(())
        };

        // Y plane (full resolution)
        let y_plane = picture.plane(PlanarImageComponent::Y);
        let y_stride = picture.stride(PlanarImageComponent::Y) as usize;
        copy_plane(&mut frame, 0, y_plane.as_ref(), y_stride, width, height)?;

        // U plane (half resolution for I420)
        let u_plane = picture.plane(PlanarImageComponent::U);
        let u_stride = picture.stride(PlanarImageComponent::U) as usize;
        copy_plane(
            &mut frame,
            1,
            u_plane.as_ref(),
            u_stride,
            width / 2,
            height / 2,
        )?;

        // V plane (half resolution for I420)
        let v_plane = picture.plane(PlanarImageComponent::V);
        let v_stride = picture.stride(PlanarImageComponent::V) as usize;
        copy_plane(
            &mut frame,
            2,
            v_plane.as_ref(),
            v_stride,
            width / 2,
            height / 2,
        )?;

        Ok(frame)
    }
}

impl Decoder for Av1Decoder {
    fn codec(&self) -> &str {
        "av1"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if packet.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "Expected video packet, got {:?}",
                packet.media_type()
            )));
        }

        // dav1d takes ownership of the buffer (T: 'static), so clone it.
        let data: Vec<u8> = packet.data().to_vec();

        // Use a monotonic counter as dav1d's internal timestamp so we can
        // correlate output pictures with their source PTS values.
        let internal_ts = self.next_timestamp;
        self.next_timestamp += 1;
        self.pts_map.push_back((internal_ts, packet.pts()));

        // Standard dav1d push/pull pattern: if send_data returns Again, we
        // need to drain pictures first, then retry send_pending_data.
        match self.decoder.send_data(data, None, Some(internal_ts), None) {
            Ok(()) => {}
            Err(Dav1dError::Again) => {
                // Drain pending pictures, then send the buffered data
                self.drain_pictures()?;
                self.decoder
                    .send_pending_data()
                    .map_err(|e| Error::Decode(format!("dav1d send_pending_data: {:?}", e)))?;
            }
            Err(e) => {
                return Err(Error::Decode(format!("dav1d send_data: {:?}", e)));
            }
        }

        // Drain any pictures that became available after sending
        self.drain_pictures()?;

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if let Some(frame) = self.buffered_frames.pop_front() {
            return Ok(frame);
        }

        // Try to drain more pictures from the decoder
        self.drain_pictures()?;
        if let Some(frame) = self.buffered_frames.pop_front() {
            return Ok(frame);
        }

        if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // Drain any remaining pictures
        self.drain_pictures()?;
        // dav1d's flush() resets the decoder state — instead of calling that,
        // we just mark ourselves as flushed so receive_frame can return EOS
        // once buffered_frames is empty.
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.decoder.flush();
        self.buffered_frames.clear();
        self.pts_map.clear();
        self.next_timestamp = 0;
        self.flushed = false;
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::{StreamInfo, StreamParams, VideoStreamParams};

    fn make_stream_info() -> StreamInfo {
        StreamInfo::new(0, MediaType::Video, "av1".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                640,
                480,
                PixelFormat::YUV420P,
            )))
            .with_time_base(1, 30)
    }

    #[test]
    fn test_av1_decoder_creation() {
        let stream_info = make_stream_info();
        let decoder = Av1Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_av1_decoder_accepts_av01_codec_name() {
        let stream_info = StreamInfo::new(0, MediaType::Video, "av01".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                640,
                480,
                PixelFormat::YUV420P,
            )));
        let decoder = Av1Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_av1_decoder_rejects_wrong_codec() {
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                640,
                480,
                PixelFormat::YUV420P,
            )));
        let decoder = Av1Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_av1_decoder_codec_name() {
        let decoder = Av1Decoder::new(make_stream_info()).unwrap();
        assert_eq!(decoder.codec(), "av1");
    }
}
