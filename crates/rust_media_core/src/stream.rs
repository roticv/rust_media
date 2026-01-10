//! Stream metadata and information types

use crate::types::{MediaType, PixelFormat, SampleFormat, ColorSpace, ColorRange};

/// Information about a media stream
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// Stream index
    pub index: usize,

    /// Media type
    pub media_type: MediaType,

    /// Codec identifier (e.g., "h264", "vp9", "aac")
    pub codec: String,

    /// Time base (rational number: num/den)
    pub time_base: (u32, u32),

    /// Duration in time_base units
    pub duration: Option<i64>,

    /// Bitrate in bits per second
    pub bitrate: Option<u64>,

    /// Stream-specific parameters
    pub params: StreamParams,

    /// Extra codec data (e.g., SPS/PPS for H.264)
    pub extra_data: Vec<u8>,
}

impl StreamInfo {
    /// Creates a new stream info
    pub fn new(index: usize, media_type: MediaType, codec: String) -> Self {
        Self {
            index,
            media_type,
            codec,
            time_base: (1, 1000000), // Default to microseconds
            duration: None,
            bitrate: None,
            params: StreamParams::Unknown,
            extra_data: Vec::new(),
        }
    }

    /// Builder method to set time base
    pub fn with_time_base(mut self, num: u32, den: u32) -> Self {
        self.time_base = (num, den);
        self
    }

    /// Builder method to set duration
    pub fn with_duration(mut self, duration: i64) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Builder method to set bitrate
    pub fn with_bitrate(mut self, bitrate: u64) -> Self {
        self.bitrate = Some(bitrate);
        self
    }

    /// Builder method to set parameters
    pub fn with_params(mut self, params: StreamParams) -> Self {
        self.params = params;
        self
    }

    /// Builder method to set extra data
    pub fn with_extra_data(mut self, data: Vec<u8>) -> Self {
        self.extra_data = data;
        self
    }
}

/// Stream-specific parameters
#[derive(Debug, Clone)]
pub enum StreamParams {
    Video(VideoStreamParams),
    Audio(AudioStreamParams),
    Unknown,
}

/// Video stream parameters
#[derive(Debug, Clone)]
pub struct VideoStreamParams {
    /// Width in pixels
    pub width: usize,

    /// Height in pixels
    pub height: usize,

    /// Pixel format (as stored in container/codec)
    pub pixel_format: PixelFormat,

    /// Frame rate (rational number: num/den)
    pub frame_rate: (u32, u32),

    /// Color space
    pub color_space: ColorSpace,

    /// Color range
    pub color_range: ColorRange,

    /// Sample aspect ratio (width:height)
    pub sample_aspect_ratio: (u32, u32),

    /// Bit depth
    pub bit_depth: u8,
}

impl VideoStreamParams {
    /// Creates new video stream parameters
    pub fn new(width: usize, height: usize, pixel_format: PixelFormat) -> Self {
        Self {
            width,
            height,
            pixel_format,
            frame_rate: (25, 1), // Default 25 fps
            color_space: ColorSpace::Unknown,
            color_range: ColorRange::Unknown,
            sample_aspect_ratio: (1, 1),
            bit_depth: pixel_format.bit_depth(),
        }
    }

    /// Builder method to set frame rate
    pub fn with_frame_rate(mut self, num: u32, den: u32) -> Self {
        self.frame_rate = (num, den);
        self
    }

    /// Builder method to set color space
    pub fn with_color_space(mut self, color_space: ColorSpace) -> Self {
        self.color_space = color_space;
        self
    }

    /// Builder method to set color range
    pub fn with_color_range(mut self, color_range: ColorRange) -> Self {
        self.color_range = color_range;
        self
    }
}

/// Audio stream parameters
#[derive(Debug, Clone)]
pub struct AudioStreamParams {
    /// Sample rate in Hz
    pub sample_rate: u32,

    /// Number of channels
    pub channels: usize,

    /// Sample format
    pub sample_format: SampleFormat,

    /// Channel layout description
    pub channel_layout: String,

    /// Bits per sample
    pub bits_per_sample: usize,

    /// Frame size (samples per channel per frame)
    pub frame_size: Option<usize>,
}

impl AudioStreamParams {
    /// Creates new audio stream parameters
    pub fn new(sample_rate: u32, channels: usize, sample_format: SampleFormat) -> Self {
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
            sample_format,
            channel_layout,
            bits_per_sample: sample_format.bytes_per_sample() * 8,
            frame_size: None,
        }
    }

    /// Builder method to set frame size
    pub fn with_frame_size(mut self, frame_size: usize) -> Self {
        self.frame_size = Some(frame_size);
        self
    }

    /// Builder method to set channel layout
    pub fn with_channel_layout(mut self, layout: String) -> Self {
        self.channel_layout = layout;
        self
    }
}

/// Container format information
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    /// Format name (e.g., "mp4", "mkv", "webm")
    pub format_name: String,

    /// Total duration in microseconds
    pub duration: Option<i64>,

    /// Total bitrate in bits per second
    pub bitrate: Option<u64>,

    /// Container metadata
    pub metadata: Metadata,
}

/// Metadata key-value pairs
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    entries: Vec<(String, String)>,
}

impl Metadata {
    /// Creates a new empty metadata collection
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Adds a metadata entry
    pub fn insert(&mut self, key: String, value: String) {
        self.entries.push((key, value));
    }

    /// Gets a metadata value by key
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Returns all metadata entries
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// Returns an iterator over metadata entries
    pub fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.entries.iter()
    }
}
