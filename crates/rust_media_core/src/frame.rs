//! Frame structure for bit-level uncompressed media data

use crate::types::{
    MediaType, PixelFormat, SampleFormat, ColorSpace, ColorRange, FrameFlags,
};

/// A frame containing uncompressed/decoded media data at the bit level.
///
/// Frames represent raw media data that has been decoded from packets.
/// For video, this is pixel data in various formats. For audio, this is
/// sample data. Frames can be processed, filtered, and then encoded back
/// into packets.
#[derive(Debug, Clone)]
pub struct Frame {
    /// The frame data (can contain multiple planes for planar formats)
    data: Vec<Vec<u8>>,

    /// Presentation timestamp in time_base units
    pts: Option<i64>,

    /// Duration in time_base units
    duration: Option<i64>,

    /// Type of media in this frame
    media_type: MediaType,

    /// Frame-specific parameters
    params: FrameParams,

    /// Frame flags
    flags: FrameFlags,
}

/// Frame parameters that vary by media type
#[derive(Debug, Clone)]
pub enum FrameParams {
    Video(VideoParams),
    Audio(AudioParams),
    Unknown,
}

/// Video-specific frame parameters
#[derive(Debug, Clone)]
pub struct VideoParams {
    /// Width in pixels
    pub width: usize,

    /// Height in pixels
    pub height: usize,

    /// Pixel format
    pub format: PixelFormat,

    /// Line size (stride) for each plane in bytes
    pub linesize: Vec<usize>,

    /// Color space
    pub color_space: ColorSpace,

    /// Color range
    pub color_range: ColorRange,

    /// Sample aspect ratio (width:height)
    pub sample_aspect_ratio: (u32, u32),
}

impl VideoParams {
    /// Creates new video parameters
    pub fn new(width: usize, height: usize, format: PixelFormat) -> Self {
        // Calculate default linesize for each plane
        let linesize = Self::calculate_linesize(width, format);

        Self {
            width,
            height,
            format,
            linesize,
            color_space: ColorSpace::Unknown,
            color_range: ColorRange::Unknown,
            sample_aspect_ratio: (1, 1),
        }
    }

    /// Calculates the linesize for each plane based on format
    fn calculate_linesize(width: usize, format: PixelFormat) -> Vec<usize> {
        let bytes_per_pixel = if format.bit_depth() <= 8 { 1 } else { 2 };

        match format {
            // YUV 4:2:0 planar formats (3 planes: Y, U, V)
            PixelFormat::YUV420P | PixelFormat::YUV420P10LE => {
                vec![
                    width * bytes_per_pixel,           // Y plane
                    (width / 2) * bytes_per_pixel,     // U plane
                    (width / 2) * bytes_per_pixel,     // V plane
                ]
            }
            // YUV 4:2:2 planar formats
            PixelFormat::YUV422P | PixelFormat::YUV422P10LE => {
                vec![
                    width * bytes_per_pixel,           // Y plane
                    (width / 2) * bytes_per_pixel,     // U plane
                    (width / 2) * bytes_per_pixel,     // V plane
                ]
            }
            // YUV 4:4:4 planar formats
            PixelFormat::YUV444P | PixelFormat::YUV444P10LE => {
                vec![
                    width * bytes_per_pixel,           // Y plane
                    width * bytes_per_pixel,           // U plane
                    width * bytes_per_pixel,           // V plane
                ]
            }
            // NV12 (Y plane + interleaved UV)
            PixelFormat::NV12 => {
                vec![
                    width,                             // Y plane
                    width,                             // UV plane (interleaved)
                ]
            }
            // RGB/BGR formats (packed)
            PixelFormat::RGB24 | PixelFormat::BGR24 => {
                vec![width * 3]
            }
            PixelFormat::RGBA | PixelFormat::BGRA => {
                vec![width * 4]
            }
            // Grayscale
            PixelFormat::GRAY8 | PixelFormat::GRAY10LE => {
                vec![width * bytes_per_pixel]
            }
            _ => vec![width * bytes_per_pixel],
        }
    }

    /// Calculates the total size needed for frame data
    pub fn calculate_buffer_size(&self) -> usize {
        match self.format {
            // YUV 4:2:0 formats
            PixelFormat::YUV420P | PixelFormat::YUV420P10LE => {
                let y_size = self.linesize[0] * self.height;
                let uv_size = self.linesize[1] * (self.height / 2);
                y_size + uv_size * 2
            }
            // YUV 4:2:2 formats
            PixelFormat::YUV422P | PixelFormat::YUV422P10LE => {
                let y_size = self.linesize[0] * self.height;
                let uv_size = self.linesize[1] * self.height;
                y_size + uv_size * 2
            }
            // YUV 4:4:4 formats
            PixelFormat::YUV444P | PixelFormat::YUV444P10LE => {
                let y_size = self.linesize[0] * self.height;
                let u_size = self.linesize[1] * self.height;
                let v_size = self.linesize[2] * self.height;
                y_size + u_size + v_size
            }
            // NV12
            PixelFormat::NV12 => {
                let y_size = self.linesize[0] * self.height;
                let uv_size = self.linesize[1] * (self.height / 2);
                y_size + uv_size
            }
            // Packed formats
            PixelFormat::RGB24 | PixelFormat::BGR24 |
            PixelFormat::RGBA | PixelFormat::BGRA => {
                self.linesize[0] * self.height
            }
            // Grayscale
            PixelFormat::GRAY8 | PixelFormat::GRAY10LE => {
                self.linesize[0] * self.height
            }
            _ => 0,
        }
    }

    /// Returns the number of planes for this format
    pub fn num_planes(&self) -> usize {
        self.linesize.len()
    }
}

/// Audio-specific frame parameters
#[derive(Debug, Clone)]
pub struct AudioParams {
    /// Sample rate in Hz
    pub sample_rate: u32,

    /// Number of audio channels
    pub channels: usize,

    /// Sample format
    pub format: SampleFormat,

    /// Number of samples per channel
    pub num_samples: usize,

    /// Channel layout (e.g., "stereo", "5.1")
    pub channel_layout: String,
}

impl AudioParams {
    /// Creates new audio parameters
    pub fn new(
        sample_rate: u32,
        channels: usize,
        format: SampleFormat,
        num_samples: usize,
    ) -> Self {
        let channel_layout = match channels {
            1 => "mono".to_string(),
            2 => "stereo".to_string(),
            6 => "5.1".to_string(),
            8 => "7.1".to_string(),
            _ => format!("{}ch", channels),
        };

        Self {
            sample_rate,
            channels,
            format,
            num_samples,
            channel_layout,
        }
    }

    /// Calculates the total size needed for audio data
    pub fn calculate_buffer_size(&self) -> usize {
        let bytes_per_sample = self.format.bytes_per_sample();

        if self.format.is_planar() {
            // For planar formats, each channel has its own buffer
            bytes_per_sample * self.num_samples * self.channels
        } else {
            // For packed formats, samples are interleaved
            bytes_per_sample * self.num_samples * self.channels
        }
    }

    /// Returns the number of planes for this format
    pub fn num_planes(&self) -> usize {
        if self.format.is_planar() {
            self.channels
        } else {
            1
        }
    }
}

impl Frame {
    /// Creates a new video frame
    pub fn new_video(width: usize, height: usize, format: PixelFormat) -> Self {
        let params = VideoParams::new(width, height, format);
        let num_planes = params.num_planes();

        // Allocate data buffers for each plane
        let mut data = Vec::with_capacity(num_planes);
        for i in 0..num_planes {
            let plane_height = match format {
                PixelFormat::YUV420P | PixelFormat::YUV420P10LE |
                PixelFormat::NV12 if i > 0 => height / 2,
                PixelFormat::YUV422P | PixelFormat::YUV422P10LE if i > 0 => height,
                PixelFormat::YUV444P | PixelFormat::YUV444P10LE if i > 0 => height,
                _ => height,
            };
            data.push(vec![0; params.linesize[i] * plane_height]);
        }

        Self {
            data,
            pts: None,
            duration: None,
            media_type: MediaType::Video,
            params: FrameParams::Video(params),
            flags: FrameFlags::new(),
        }
    }

    /// Creates a new audio frame
    pub fn new_audio(
        sample_rate: u32,
        channels: usize,
        format: SampleFormat,
        num_samples: usize,
    ) -> Self {
        let params = AudioParams::new(sample_rate, channels, format, num_samples);
        let num_planes = params.num_planes();

        // Allocate data buffers
        let mut data = Vec::with_capacity(num_planes);
        let bytes_per_sample = format.bytes_per_sample();

        if format.is_planar() {
            // One buffer per channel
            for _ in 0..channels {
                data.push(vec![0; bytes_per_sample * num_samples]);
            }
        } else {
            // Single interleaved buffer
            data.push(vec![0; bytes_per_sample * num_samples * channels]);
        }

        Self {
            data,
            pts: None,
            duration: None,
            media_type: MediaType::Audio,
            params: FrameParams::Audio(params),
            flags: FrameFlags::new(),
        }
    }

    /// Returns a reference to the frame data planes
    pub fn data(&self) -> &[Vec<u8>] {
        &self.data
    }

    /// Returns a mutable reference to the frame data planes
    pub fn data_mut(&mut self) -> &mut [Vec<u8>] {
        &mut self.data
    }

    /// Returns a reference to a specific plane
    pub fn plane(&self, index: usize) -> Option<&[u8]> {
        self.data.get(index).map(|v| v.as_slice())
    }

    /// Returns a mutable reference to a specific plane
    pub fn plane_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        self.data.get_mut(index).map(|v| v.as_mut_slice())
    }

    /// Returns the number of planes in this frame
    pub fn num_planes(&self) -> usize {
        self.data.len()
    }

    /// Returns the presentation timestamp
    pub fn pts(&self) -> Option<i64> {
        self.pts
    }

    /// Sets the presentation timestamp
    pub fn set_pts(&mut self, pts: Option<i64>) {
        self.pts = pts;
    }

    /// Builder method to set presentation timestamp
    pub fn with_pts(mut self, pts: i64) -> Self {
        self.pts = Some(pts);
        self
    }

    /// Returns the duration
    pub fn duration(&self) -> Option<i64> {
        self.duration
    }

    /// Sets the duration
    pub fn set_duration(&mut self, duration: Option<i64>) {
        self.duration = duration;
    }

    /// Builder method to set duration
    pub fn with_duration(mut self, duration: i64) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Returns the media type
    pub fn media_type(&self) -> MediaType {
        self.media_type
    }

    /// Returns a reference to the frame parameters
    pub fn params(&self) -> &FrameParams {
        &self.params
    }

    /// Returns a mutable reference to the frame parameters
    pub fn params_mut(&mut self) -> &mut FrameParams {
        &mut self.params
    }

    /// Returns video parameters if this is a video frame
    pub fn video_params(&self) -> Option<&VideoParams> {
        match &self.params {
            FrameParams::Video(p) => Some(p),
            _ => None,
        }
    }

    /// Returns mutable video parameters if this is a video frame
    pub fn video_params_mut(&mut self) -> Option<&mut VideoParams> {
        match &mut self.params {
            FrameParams::Video(p) => Some(p),
            _ => None,
        }
    }

    /// Returns audio parameters if this is an audio frame
    pub fn audio_params(&self) -> Option<&AudioParams> {
        match &self.params {
            FrameParams::Audio(p) => Some(p),
            _ => None,
        }
    }

    /// Returns mutable audio parameters if this is an audio frame
    pub fn audio_params_mut(&mut self) -> Option<&mut AudioParams> {
        match &mut self.params {
            FrameParams::Audio(p) => Some(p),
            _ => None,
        }
    }

    /// Returns a reference to the frame flags
    pub fn flags(&self) -> &FrameFlags {
        &self.flags
    }

    /// Returns a mutable reference to the frame flags
    pub fn flags_mut(&mut self) -> &mut FrameFlags {
        &mut self.flags
    }

    /// Builder method to set flags
    pub fn with_flags(mut self, flags: FrameFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Returns whether this is a keyframe
    pub fn is_keyframe(&self) -> bool {
        self.flags.is_keyframe()
    }

    /// Sets the keyframe flag
    pub fn set_keyframe(&mut self, is_keyframe: bool) {
        self.flags.set_keyframe(is_keyframe);
    }

    /// Builder method to mark as keyframe
    pub fn with_keyframe(mut self) -> Self {
        self.flags.set_keyframe(true);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_frame_creation() {
        let frame = Frame::new_video(1920, 1080, PixelFormat::YUV420P);

        assert_eq!(frame.media_type(), MediaType::Video);
        assert_eq!(frame.num_planes(), 3);

        let video_params = frame.video_params().unwrap();
        assert_eq!(video_params.width, 1920);
        assert_eq!(video_params.height, 1080);
        assert_eq!(video_params.format, PixelFormat::YUV420P);
    }

    #[test]
    fn test_audio_frame_creation() {
        let frame = Frame::new_audio(48000, 2, SampleFormat::F32P, 1024);

        assert_eq!(frame.media_type(), MediaType::Audio);
        assert_eq!(frame.num_planes(), 2); // Stereo planar

        let audio_params = frame.audio_params().unwrap();
        assert_eq!(audio_params.sample_rate, 48000);
        assert_eq!(audio_params.channels, 2);
        assert_eq!(audio_params.num_samples, 1024);
    }

    #[test]
    fn test_frame_builder() {
        let frame = Frame::new_video(640, 480, PixelFormat::RGB24)
            .with_pts(1000)
            .with_duration(40)
            .with_keyframe();

        assert_eq!(frame.pts(), Some(1000));
        assert_eq!(frame.duration(), Some(40));
        assert!(frame.is_keyframe());
    }

    #[test]
    fn test_video_params_linesize() {
        let params = VideoParams::new(1920, 1080, PixelFormat::YUV420P);

        assert_eq!(params.linesize[0], 1920);  // Y plane
        assert_eq!(params.linesize[1], 960);   // U plane
        assert_eq!(params.linesize[2], 960);   // V plane
    }

    #[test]
    fn test_video_params_10bit() {
        let params = VideoParams::new(1920, 1080, PixelFormat::YUV420P10LE);

        // 10-bit uses 2 bytes per pixel
        assert_eq!(params.linesize[0], 1920 * 2);  // Y plane
        assert_eq!(params.linesize[1], 960 * 2);   // U plane
        assert_eq!(params.linesize[2], 960 * 2);   // V plane
    }

    #[test]
    fn test_frame_10bit_allocation() {
        let frame = Frame::new_video(64, 64, PixelFormat::YUV420P10LE);
        assert_eq!(frame.num_planes(), 3);
        // Y plane: 64 * 2 bytes/sample * 64 rows
        assert_eq!(frame.plane(0).unwrap().len(), 64 * 2 * 64);
        // U plane: 32 * 2 bytes/sample * 32 rows
        assert_eq!(frame.plane(1).unwrap().len(), 32 * 2 * 32);
        // V plane: same as U
        assert_eq!(frame.plane(2).unwrap().len(), 32 * 2 * 32);
    }

    #[test]
    fn test_audio_params_buffer_size() {
        let params = AudioParams::new(48000, 2, SampleFormat::F32, 1024);

        // 2 channels * 1024 samples * 4 bytes (F32)
        assert_eq!(params.calculate_buffer_size(), 2 * 1024 * 4);
    }

    #[test]
    fn test_frame_plane_access() {
        let mut frame = Frame::new_video(640, 480, PixelFormat::YUV420P);

        // Write to Y plane
        if let Some(plane) = frame.plane_mut(0) {
            plane[0] = 128;
        }

        // Read from Y plane
        assert_eq!(frame.plane(0).unwrap()[0], 128);
    }
}
