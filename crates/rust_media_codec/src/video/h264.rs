//! H.264/AVC video decoder implementation using rust_h264
//!
//! Provides H.264 decoding via the rust_h264 crate (pure Rust implementation,
//! MIT/Apache-2.0). The default build needs no external libraries or feature
//! flags.
//!
//! # Profile Support
//!
//! Baseline, Main, and High profiles. YUV420P (I420) output only.
//!
//! # Implementation Notes
//!
//! As of `rust_h264` 0.3.0, two pieces of functionality that we used to
//! implement manually now live in the upstream crate:
//!
//! 1. **AVCC parsing** — `nal::parse_avcc_config` extracts SPS/PPS from an
//!    `avcC` (AVCDecoderConfigurationRecord) box and `nal::parse_avcc` parses
//!    length-prefixed sample data into NAL units. We previously did this with
//!    a hand-rolled `avcc_to_annex_b` plus `parse_annex_b`.
//!
//! 2. **Display-order frame reordering** — `decoder::OrderedDecoder` wraps
//!    the raw decoder, tracks GOP boundaries via IDR slices, and emits frames
//!    in display order via a POC-based reorder buffer. We previously did this
//!    with our own `BinaryHeap<PocFrame>` and IDR-counter logic.
//!
//! Both responsibilities now belong to upstream, so this file is mostly a
//! thin adapter between the rust_h264 types and the `rust_media_core`
//! `Decoder` trait.

use rust_h264::decoder::OrderedDecoder;
use rust_h264::nal::{parse_annex_b, parse_avcc, parse_avcc_config, AvccConfig};
use rust_media_core::{
    Decoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};
use std::collections::VecDeque;

/// H.264/AVC video decoder using rust_h264.
///
/// Decodes H.264-compressed video packets into raw YUV frames in display
/// order. Accepts both AVCC sample data (from MP4/MKV) and Annex B data
/// (from raw H.264 streams).
///
/// # MP4 Support
///
/// Pass the `avcC` box payload via `StreamInfo.extra_data`. We parse it via
/// `rust_h264::nal::parse_avcc_config`, feed the SPS/PPS NALs to the decoder
/// once, and use the recorded `length_size` for every subsequent sample.
///
/// # PTS Handling
///
/// rust_h264 doesn't carry PTS through the decoder, but `OrderedDecoder`
/// already returns frames in display order — which is the same order PTS
/// values arrive in input packets. We push input PTSes into a FIFO and pop
/// them as output frames are produced.
///
/// # Output Format
///
/// Always YUV420P (I420). Use `flush()` to drain remaining frames at EOS.
pub struct H264Decoder {
    stream_info: StreamInfo,
    decoder: OrderedDecoder,
    /// Frames already converted to `rust_media_core::Frame` and ready to hand
    /// out via `receive_frame`. They're in display order.
    output_frames: VecDeque<Frame>,
    flushed: bool,
    /// Length-prefix size from `avcC`. `None` means input is Annex B and we
    /// use `parse_annex_b` instead of `parse_avcc`.
    avcc_length_size: Option<usize>,
    /// Whether SPS/PPS have been fed to the decoder. Only relevant for the
    /// AVCC path; Annex B streams carry parameter sets inline.
    sent_parameter_sets: bool,
    /// Cached extra_data so we can re-parse SPS/PPS on `reset()`. The
    /// `AvccConfig` returned by `parse_avcc_config` borrows from this slice,
    /// so we can't store the parsed result directly.
    extra_data: Vec<u8>,
    /// PTS values from input packets in arrival order. Output frames come out
    /// in display (== arrival) order, so a FIFO is sufficient — no heap.
    pts_queue: VecDeque<Option<i64>>,
}

impl H264Decoder {
    /// Creates a new H.264 decoder from stream information.
    ///
    /// `stream_info.codec` must be `"h264"` or `"avc"`. If `extra_data` is
    /// non-empty it's parsed as an `avcC` configuration record and the
    /// resulting SPS/PPS are fed to the decoder before any sample data.
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "h264" && stream_info.codec != "avc" {
            return Err(Error::Unsupported(format!(
                "Expected h264/avc codec, got {}",
                stream_info.codec
            )));
        }

        let extra_data = stream_info.extra_data.clone();
        let mut decoder = Self {
            stream_info,
            decoder: OrderedDecoder::new(),
            output_frames: VecDeque::new(),
            flushed: false,
            avcc_length_size: None,
            sent_parameter_sets: false,
            extra_data,
            pts_queue: VecDeque::new(),
        };

        // If we have an avcC config, parse it now and prime the decoder with
        // the SPS/PPS NALs. This lets us decode the very first sample without
        // having to wait for inline parameter sets.
        if !decoder.extra_data.is_empty() {
            decoder.feed_parameter_sets()?;
        }

        Ok(decoder)
    }

    /// Parse the cached `avcC` extra_data, feed SPS/PPS to the underlying
    /// decoder, and remember the length-prefix size for sample parsing.
    fn feed_parameter_sets(&mut self) -> Result<()> {
        let cfg: AvccConfig<'_> = parse_avcc_config(&self.extra_data)
            .map_err(|e| Error::InvalidData(format!("avcC parse failed: {}", e)))?;

        // Feed SPS/PPS to the decoder. These produce no frames, so we can
        // ignore the (empty) Vec returned by OrderedDecoder.
        for nal in cfg.sps_nals.iter().chain(cfg.pps_nals.iter()) {
            self.decoder
                .decode_nal(nal)
                .map_err(|e| Error::Decode(format!("H.264 SPS/PPS decode error: {}", e)))?;
        }

        self.avcc_length_size = Some(cfg.length_size);
        self.sent_parameter_sets = true;
        Ok(())
    }

    /// Convert a rust_h264 Frame to a rust_media_core Frame. PTS is assigned
    /// later from the FIFO since rust_h264 has no PTS field.
    fn convert_frame(h264_frame: &rust_h264::decoder::Frame) -> Result<Frame> {
        let width = h264_frame.width as usize;
        let height = h264_frame.height as usize;

        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);

        let y_plane = frame
            .plane_mut(0)
            .ok_or_else(|| Error::InvalidData("Failed to get Y plane".to_string()))?;
        y_plane.copy_from_slice(&h264_frame.y);

        let u_plane = frame
            .plane_mut(1)
            .ok_or_else(|| Error::InvalidData("Failed to get U plane".to_string()))?;
        u_plane.copy_from_slice(&h264_frame.u);

        let v_plane = frame
            .plane_mut(2)
            .ok_or_else(|| Error::InvalidData("Failed to get V plane".to_string()))?;
        v_plane.copy_from_slice(&h264_frame.v);

        Ok(frame)
    }

    /// Push a batch of decoded frames into the output queue, attaching PTS
    /// values from the FIFO in arrival order.
    fn push_decoded(&mut self, frames: Vec<rust_h264::decoder::Frame>) -> Result<()> {
        for h264_frame in frames {
            let mut frame = Self::convert_frame(&h264_frame)?;
            // Pop the next PTS — frames come out in display order (which
            // matches input order), so a simple FIFO is correct.
            let pts = self.pts_queue.pop_front().flatten();
            frame.set_pts(pts);
            self.output_frames.push_back(frame);
        }
        Ok(())
    }
}

impl Decoder for H264Decoder {
    fn codec(&self) -> &str {
        "h264"
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

        // Track this packet's PTS so we can attach it to the matching output
        // frame later. One push per input packet, one pop per output frame.
        self.pts_queue.push_back(packet.pts());

        let data = packet.data();

        // Two parsing strategies depending on what the upstream container
        // gave us. The borrow from `data` lives only as long as `nals`, so
        // we decode within the same scope.
        let decoded = if let Some(length_size) = self.avcc_length_size {
            // MP4/MKV path: length-prefixed AVCC samples.
            let nals = parse_avcc(data, length_size);
            let mut all = Vec::new();
            for nal in &nals {
                let frames = self
                    .decoder
                    .decode_nal(nal)
                    .map_err(|e| Error::Decode(format!("H.264 decode error: {}", e)))?;
                all.extend(frames);
            }
            all
        } else {
            // Annex B path: start-code delimited stream. Parameter sets are
            // inline so there's nothing to feed up front.
            let nals = parse_annex_b(data);
            let mut all = Vec::new();
            for nal in &nals {
                let frames = self
                    .decoder
                    .decode_nal(nal)
                    .map_err(|e| Error::Decode(format!("H.264 decode error: {}", e)))?;
                all.extend(frames);
            }
            all
        };

        self.push_decoded(decoded)?;
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if let Some(frame) = self.output_frames.pop_front() {
            Ok(frame)
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // Drain whatever's still buffered inside OrderedDecoder. This emits
        // any pending in-progress frame plus all frames still sitting in the
        // reorder buffer at end-of-stream.
        let remaining = self.decoder.flush();
        self.push_decoded(remaining)?;
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.decoder = OrderedDecoder::new();
        self.output_frames.clear();
        self.pts_queue.clear();
        self.flushed = false;
        self.sent_parameter_sets = false;
        self.avcc_length_size = None;
        // Re-feed parameter sets so the next packet can decode immediately.
        if !self.extra_data.is_empty() {
            self.feed_parameter_sets()?;
        }
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::VideoStreamParams;

    #[test]
    fn test_h264_decoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_h264_decoder_accepts_avc() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "avc".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_h264_decoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_h264_decoder_codec_name() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info).unwrap();
        assert_eq!(decoder.codec(), "h264");
    }

    #[test]
    fn test_h264_decoder_parses_avcc_extra_data() {
        // Real avcC payload from a 320x240 baseline file. Using bytes from an
        // actual SPS so the construction (which now feeds parameter sets to
        // the decoder up front) actually succeeds. The dimensions in the
        // VideoStreamParams below are placeholder — what we're testing is
        // that the avcC parser fills in `avcc_length_size` and feeds the
        // SPS/PPS without error.
        let avcc_data = vec![
            0x01, // configurationVersion
            0x42, // AVCProfileIndication (Baseline)
            0xc0, // profile_compatibility
            0x14, // AVCLevelIndication (2.0)
            0xff, // reserved + lengthSizeMinusOne (3 = 4-byte lengths)
            0xe1, // reserved + numOfSequenceParameterSets (1)
            0x00, 0x16, // SPS length (22)
            // SPS NAL: real baseline 320x240 SPS
            0x67, 0x42, 0xc0, 0x14, 0x96, 0x54, 0x05, 0x01,
            0x7b, 0xcb, 0x37, 0x01, 0x01, 0x01, 0x40, 0x00,
            0x00, 0xfa, 0x00, 0x00, 0x3a, 0x98,
            0x01, // numOfPictureParameterSets
            0x00, 0x04, // PPS length (4)
            0x68, 0xce, 0x38, 0x80, // PPS NAL
        ];

        let video_params = VideoStreamParams::new(320, 240, PixelFormat::YUV420P);
        let mut stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));
        stream_info.extra_data = avcc_data;

        let decoder = H264Decoder::new(stream_info).expect("avcC should parse");
        // 4-byte length prefix per lengthSizeMinusOne = 3 in the payload above.
        assert_eq!(decoder.avcc_length_size, Some(4));
        assert!(decoder.sent_parameter_sets);
    }
}
