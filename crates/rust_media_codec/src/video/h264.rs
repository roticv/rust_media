//! H.264/AVC video decoder implementation using rust_h264
//!
//! Provides H.264 decoding via the rust_h264 crate (pure Rust implementation).
//!
//! # Licensing
//!
//! rust_h264 is licensed under MIT/Apache-2.0, making it compatible with the project's
//! MIT/Apache-2.0 default license. No special feature flags are required.
//!
//! # Profile Support
//!
//! This decoder supports Baseline, Main, and High profiles.
//!
//! # Current Implementation Status
//!
//! ## Decoder
//! - Supports Baseline, Main, and High Profile decoding with YUV420P (I420) output
//! - Supports both AVCC format (MP4) and Annex B format (raw H.264)
//! - Automatic SPS/PPS extraction from AVCDecoderConfigurationRecord
//! - Proper flush handling for B-frames and buffered data
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_media_core::{StreamInfo, MediaType, VideoStreamParams, PixelFormat};
//! use rust_media_codec::H264Decoder;
//!
//! let video_params = VideoStreamParams::new(1920, 1080, PixelFormat::YUV420P);
//! let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
//!     .with_time_base(1, 90000)
//!     .with_params(rust_media_core::StreamParams::Video(video_params));
//!
//! let mut decoder = H264Decoder::new(stream_info)?;
//! // ... decode packets using send_packet() and receive_frame()
//! ```

use rust_h264::decoder::Decoder as RustH264Decoder;
use rust_h264::nal::parse_annex_b;
use rust_media_core::{
    Decoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};

/// Annex B start code (4-byte version)
const ANNEX_B_START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// H.264/AVC video decoder using rust_h264
///
/// Decodes H.264-compressed video packets into raw YUV frames.
///
/// # MP4 Support
///
/// This decoder automatically handles H.264 data from MP4 containers:
/// - Parses AVCDecoderConfigurationRecord from `extra_data` to extract SPS/PPS
/// - Converts AVCC format (length-prefixed NAL units) to Annex B format
/// - Sends SPS/PPS to decoder on first packet
///
/// # Notes
///
/// - Input packets can be in AVCC format (MP4) or Annex B format (raw H.264)
/// - Output is always YUV420P (I420) format
/// - The decoder handles B-frames internally; use `flush()` to retrieve buffered frames
pub struct H264Decoder {
    stream_info: StreamInfo,
    decoder: RustH264Decoder,
    buffered_frames: Vec<Frame>,
    flushed: bool,
    /// NAL unit length size in bytes (1, 2, or 4) from AVCDecoderConfigurationRecord
    nal_length_size: usize,
    /// SPS NAL units extracted from AVCDecoderConfigurationRecord
    sps_list: Vec<Vec<u8>>,
    /// PPS NAL units extracted from AVCDecoderConfigurationRecord
    pps_list: Vec<Vec<u8>>,
    /// Whether we've sent the SPS/PPS to the decoder
    sent_sps_pps: bool,
}

impl H264Decoder {
    /// Creates a new H.264 decoder from stream information
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream information (codec must be "h264" or "avc")
    ///
    /// # Returns
    ///
    /// A new H264Decoder or an error if initialization fails
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec - accept both "h264" and "avc" (common alternative names)
        if stream_info.codec != "h264" && stream_info.codec != "avc" {
            return Err(Error::Unsupported(format!(
                "Expected h264/avc codec, got {}",
                stream_info.codec
            )));
        }

        let decoder = RustH264Decoder::new();

        // Parse AVCDecoderConfigurationRecord if present in extra_data
        let (nal_length_size, sps_list, pps_list) = if !stream_info.extra_data.is_empty() {
            parse_avcc_config(&stream_info.extra_data)?
        } else {
            // No extra_data - assume Annex B format input
            (0, Vec::new(), Vec::new())
        };

        Ok(Self {
            stream_info,
            decoder,
            buffered_frames: Vec::new(),
            flushed: false,
            nal_length_size,
            sps_list,
            pps_list,
            sent_sps_pps: false,
        })
    }

    /// Converts AVCC format data to Annex B format
    ///
    /// AVCC format: [length][NAL unit][length][NAL unit]...
    /// Annex B format: [start code][NAL unit][start code][NAL unit]...
    fn avcc_to_annex_b(&self, data: &[u8]) -> Vec<u8> {
        if self.nal_length_size == 0 {
            // Not AVCC format, return as-is
            return data.to_vec();
        }

        let mut output = Vec::with_capacity(data.len() + 64);
        let mut pos = 0;

        while pos + self.nal_length_size <= data.len() {
            // Read NAL unit length
            let nal_length = match self.nal_length_size {
                1 => data[pos] as usize,
                2 => u16::from_be_bytes([data[pos], data[pos + 1]]) as usize,
                4 => u32::from_be_bytes([
                    data[pos],
                    data[pos + 1],
                    data[pos + 2],
                    data[pos + 3],
                ]) as usize,
                _ => break,
            };

            pos += self.nal_length_size;

            if pos + nal_length > data.len() {
                break;
            }

            // Add start code and NAL unit
            output.extend_from_slice(&ANNEX_B_START_CODE);
            output.extend_from_slice(&data[pos..pos + nal_length]);

            pos += nal_length;
        }

        output
    }

    /// Sends SPS/PPS to the decoder
    fn send_sps_pps(&mut self) -> Result<()> {
        if self.sent_sps_pps || (self.sps_list.is_empty() && self.pps_list.is_empty()) {
            return Ok(());
        }

        // Build Annex B data with SPS and PPS
        let mut config_data = Vec::new();

        for sps in &self.sps_list {
            config_data.extend_from_slice(&ANNEX_B_START_CODE);
            config_data.extend_from_slice(sps);
        }

        for pps in &self.pps_list {
            config_data.extend_from_slice(&ANNEX_B_START_CODE);
            config_data.extend_from_slice(pps);
        }

        if !config_data.is_empty() {
            // Parse the config data into NAL units and feed to decoder
            let nals = parse_annex_b(&config_data);
            for nal in &nals {
                let _ = self.decoder.decode_nal(nal);
            }
        }

        self.sent_sps_pps = true;
        Ok(())
    }

    /// Convert a rust_h264 Frame to a rust_media_core Frame
    fn convert_frame(
        h264_frame: &rust_h264::decoder::Frame,
        pts: Option<i64>,
    ) -> Result<Frame> {
        let width = h264_frame.width as usize;
        let height = h264_frame.height as usize;

        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
        frame.set_pts(pts);

        // Copy Y plane
        let y_plane = frame
            .plane_mut(0)
            .ok_or_else(|| Error::InvalidData("Failed to get Y plane".to_string()))?;
        y_plane.copy_from_slice(&h264_frame.y);

        // Copy U plane
        let u_plane = frame
            .plane_mut(1)
            .ok_or_else(|| Error::InvalidData("Failed to get U plane".to_string()))?;
        u_plane.copy_from_slice(&h264_frame.u);

        // Copy V plane
        let v_plane = frame
            .plane_mut(2)
            .ok_or_else(|| Error::InvalidData("Failed to get V plane".to_string()))?;
        v_plane.copy_from_slice(&h264_frame.v);

        Ok(frame)
    }
}

/// Parses AVCDecoderConfigurationRecord from extra_data
///
/// Format:
/// - configurationVersion (1 byte) = 1
/// - AVCProfileIndication (1 byte)
/// - profile_compatibility (1 byte)
/// - AVCLevelIndication (1 byte)
/// - reserved (6 bits) + lengthSizeMinusOne (2 bits)
/// - reserved (3 bits) + numOfSequenceParameterSets (5 bits)
/// - For each SPS: length (2 bytes) + SPS data
/// - numOfPictureParameterSets (1 byte)
/// - For each PPS: length (2 bytes) + PPS data
fn parse_avcc_config(data: &[u8]) -> Result<(usize, Vec<Vec<u8>>, Vec<Vec<u8>>)> {
    if data.len() < 7 {
        return Err(Error::InvalidData(
            "AVCDecoderConfigurationRecord too short".to_string(),
        ));
    }

    let config_version = data[0];
    if config_version != 1 {
        return Err(Error::InvalidData(format!(
            "Unsupported AVCDecoderConfigurationRecord version: {}",
            config_version
        )));
    }

    // lengthSizeMinusOne is the bottom 2 bits of byte 4
    let length_size_minus_one = data[4] & 0x03;
    let nal_length_size = (length_size_minus_one + 1) as usize;

    // numOfSequenceParameterSets is the bottom 5 bits of byte 5
    let num_sps = (data[5] & 0x1F) as usize;

    let mut pos = 6;
    let mut sps_list = Vec::with_capacity(num_sps);

    // Parse SPS entries
    for _ in 0..num_sps {
        if pos + 2 > data.len() {
            break;
        }
        let sps_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        if pos + sps_length > data.len() {
            break;
        }
        sps_list.push(data[pos..pos + sps_length].to_vec());
        pos += sps_length;
    }

    // Parse PPS entries
    if pos >= data.len() {
        return Ok((nal_length_size, sps_list, Vec::new()));
    }

    let num_pps = data[pos] as usize;
    pos += 1;

    let mut pps_list = Vec::with_capacity(num_pps);

    for _ in 0..num_pps {
        if pos + 2 > data.len() {
            break;
        }
        let pps_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        if pos + pps_length > data.len() {
            break;
        }
        pps_list.push(data[pos..pos + pps_length].to_vec());
        pos += pps_length;
    }

    Ok((nal_length_size, sps_list, pps_list))
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

        // Send SPS/PPS on first packet if we have them
        self.send_sps_pps()?;

        let pts = packet.pts();
        let data = packet.data();

        // Convert from AVCC to Annex B format if needed
        let annex_b_data = if self.nal_length_size > 0 {
            self.avcc_to_annex_b(data)
        } else {
            data.to_vec()
        };

        // Parse NAL units and feed to decoder
        let nals = parse_annex_b(&annex_b_data);
        for nal in &nals {
            match self.decoder.decode_nal(nal) {
                Ok(Some(h264_frame)) => {
                    let frame = Self::convert_frame(&h264_frame, pts)?;
                    self.buffered_frames.push(frame);
                }
                Ok(None) => {
                    // No frame available yet (need more data or buffered for reordering)
                }
                Err(e) => {
                    return Err(Error::Decode(format!("H.264 decode error: {}", e)));
                }
            }
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if !self.buffered_frames.is_empty() {
            Ok(self.buffered_frames.remove(0))
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // Flush remaining frames from the decoder
        if let Some(h264_frame) = self.decoder.flush() {
            if let Ok(frame) = Self::convert_frame(&h264_frame, None) {
                self.buffered_frames.push(frame);
            }
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.decoder = RustH264Decoder::new();
        self.buffered_frames.clear();
        self.flushed = false;
        self.sent_sps_pps = false;
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
    fn test_parse_avcc_config() {
        // Example AVCDecoderConfigurationRecord
        // Version 1, profile 100, compat 0, level 31, 4-byte NAL length
        // 1 SPS (7 bytes), 1 PPS (4 bytes)
        let avcc_data = [
            0x01, // configurationVersion
            0x64, // AVCProfileIndication (High profile)
            0x00, // profile_compatibility
            0x1F, // AVCLevelIndication (3.1)
            0xFF, // reserved + lengthSizeMinusOne (3 = 4-byte lengths)
            0xE1, // reserved + numOfSequenceParameterSets (1)
            0x00, 0x07, // SPS length
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, // SPS data
            0x01, // numOfPictureParameterSets
            0x00, 0x04, // PPS length
            0x68, 0xEB, 0xE3, 0xCB, // PPS data
        ];

        let result = parse_avcc_config(&avcc_data);
        assert!(result.is_ok());

        let (nal_length_size, sps_list, pps_list) = result.unwrap();
        assert_eq!(nal_length_size, 4);
        assert_eq!(sps_list.len(), 1);
        assert_eq!(pps_list.len(), 1);
        assert_eq!(sps_list[0].len(), 7);
        assert_eq!(pps_list[0].len(), 4);
    }

    #[test]
    fn test_avcc_to_annex_b_conversion() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let mut stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        // Set up with 4-byte NAL length size
        stream_info.extra_data = vec![
            0x01, 0x64, 0x00, 0x1F, 0xFF, // config with 4-byte lengths
            0xE0, // 0 SPS
            0x00, // 0 PPS
        ];

        let decoder = H264Decoder::new(stream_info).unwrap();

        // AVCC format: 4-byte length (5) + 5-byte NAL unit
        let avcc_data = [
            0x00, 0x00, 0x00, 0x05, // length = 5
            0x65, 0x01, 0x02, 0x03, 0x04, // NAL unit data
        ];

        let annex_b = decoder.avcc_to_annex_b(&avcc_data);

        // Should be: start code (4 bytes) + NAL unit (5 bytes)
        assert_eq!(annex_b.len(), 9);
        assert_eq!(&annex_b[0..4], &ANNEX_B_START_CODE);
        assert_eq!(&annex_b[4..9], &[0x65, 0x01, 0x02, 0x03, 0x04]);
    }
}
