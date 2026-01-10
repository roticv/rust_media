//! Common types and enums used throughout the media processing framework

/// Type of media data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    /// Video data
    Video,
    /// Audio data
    Audio,
    /// Subtitle data
    Subtitle,
    /// Generic data
    Data,
    /// Unknown type
    Unknown,
}

/// Pixel format for video frames
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// YUV 4:2:0 planar, 8-bit
    YUV420P,
    /// YUV 4:2:0 planar, 10-bit
    YUV420P10LE,
    /// YUV 4:2:2 planar, 8-bit
    YUV422P,
    /// YUV 4:2:2 planar, 10-bit
    YUV422P10LE,
    /// YUV 4:4:4 planar, 8-bit
    YUV444P,
    /// YUV 4:4:4 planar, 10-bit
    YUV444P10LE,
    /// RGB 24-bit
    RGB24,
    /// RGBA 32-bit
    RGBA,
    /// BGR 24-bit
    BGR24,
    /// BGRA 32-bit
    BGRA,
    /// Grayscale 8-bit
    GRAY8,
    /// Grayscale 10-bit
    GRAY10LE,
    /// NV12 (Y plane + interleaved UV)
    NV12,
    /// Unknown format
    Unknown,
}

impl PixelFormat {
    /// Returns the bit depth of the pixel format
    pub fn bit_depth(&self) -> u8 {
        match self {
            PixelFormat::YUV420P | PixelFormat::YUV422P | PixelFormat::YUV444P |
            PixelFormat::RGB24 | PixelFormat::RGBA | PixelFormat::BGR24 |
            PixelFormat::BGRA | PixelFormat::GRAY8 | PixelFormat::NV12 => 8,
            PixelFormat::YUV420P10LE | PixelFormat::YUV422P10LE |
            PixelFormat::YUV444P10LE | PixelFormat::GRAY10LE => 10,
            PixelFormat::Unknown => 0,
        }
    }

    /// Returns whether this is a planar format
    pub fn is_planar(&self) -> bool {
        matches!(self,
            PixelFormat::YUV420P | PixelFormat::YUV420P10LE |
            PixelFormat::YUV422P | PixelFormat::YUV422P10LE |
            PixelFormat::YUV444P | PixelFormat::YUV444P10LE |
            PixelFormat::GRAY8 | PixelFormat::GRAY10LE
        )
    }
}

/// Sample format for audio frames
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// Unsigned 8-bit
    U8,
    /// Signed 16-bit
    S16,
    /// Signed 32-bit
    S32,
    /// 32-bit float
    F32,
    /// 64-bit float
    F64,
    /// Unsigned 8-bit planar
    U8P,
    /// Signed 16-bit planar
    S16P,
    /// Signed 32-bit planar
    S32P,
    /// 32-bit float planar
    F32P,
    /// 64-bit float planar
    F64P,
    /// Unknown format
    Unknown,
}

impl SampleFormat {
    /// Returns the number of bytes per sample
    pub fn bytes_per_sample(&self) -> usize {
        match self {
            SampleFormat::U8 | SampleFormat::U8P => 1,
            SampleFormat::S16 | SampleFormat::S16P => 2,
            SampleFormat::S32 | SampleFormat::S32P |
            SampleFormat::F32 | SampleFormat::F32P => 4,
            SampleFormat::F64 | SampleFormat::F64P => 8,
            SampleFormat::Unknown => 0,
        }
    }

    /// Returns whether this is a planar format
    pub fn is_planar(&self) -> bool {
        matches!(self,
            SampleFormat::U8P | SampleFormat::S16P | SampleFormat::S32P |
            SampleFormat::F32P | SampleFormat::F64P
        )
    }
}

/// Color space specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// RGB color space
    RGB,
    /// BT.709 (HDTV)
    BT709,
    /// BT.601 (SDTV)
    BT601,
    /// BT.2020 (UHDTV)
    BT2020,
    /// SMPTE 240M
    SMPTE240M,
    /// Unknown color space
    Unknown,
}

/// Color range specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRange {
    /// Limited/TV range (16-235 for 8-bit)
    Limited,
    /// Full/PC range (0-255 for 8-bit)
    Full,
    /// Unknown range
    Unknown,
}

/// Packet flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFlags {
    flags: u32,
}

impl PacketFlags {
    pub const KEYFRAME: u32 = 1 << 0;
    pub const CORRUPT: u32 = 1 << 1;
    pub const DISCARD: u32 = 1 << 2;

    pub fn new() -> Self {
        Self { flags: 0 }
    }

    pub fn with_keyframe(mut self) -> Self {
        self.flags |= Self::KEYFRAME;
        self
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & Self::KEYFRAME != 0
    }

    pub fn set_keyframe(&mut self, value: bool) {
        if value {
            self.flags |= Self::KEYFRAME;
        } else {
            self.flags &= !Self::KEYFRAME;
        }
    }

    pub fn is_corrupt(&self) -> bool {
        self.flags & Self::CORRUPT != 0
    }

    pub fn set_corrupt(&mut self, value: bool) {
        if value {
            self.flags |= Self::CORRUPT;
        } else {
            self.flags &= !Self::CORRUPT;
        }
    }

    pub fn is_discard(&self) -> bool {
        self.flags & Self::DISCARD != 0
    }

    pub fn set_discard(&mut self, value: bool) {
        if value {
            self.flags |= Self::DISCARD;
        } else {
            self.flags &= !Self::DISCARD;
        }
    }
}

impl Default for PacketFlags {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFlags {
    flags: u32,
}

impl FrameFlags {
    pub const KEYFRAME: u32 = 1 << 0;
    pub const INTERLACED: u32 = 1 << 1;
    pub const TOP_FIELD_FIRST: u32 = 1 << 2;

    pub fn new() -> Self {
        Self { flags: 0 }
    }

    pub fn with_keyframe(mut self) -> Self {
        self.flags |= Self::KEYFRAME;
        self
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & Self::KEYFRAME != 0
    }

    pub fn set_keyframe(&mut self, value: bool) {
        if value {
            self.flags |= Self::KEYFRAME;
        } else {
            self.flags &= !Self::KEYFRAME;
        }
    }

    pub fn is_interlaced(&self) -> bool {
        self.flags & Self::INTERLACED != 0
    }

    pub fn set_interlaced(&mut self, value: bool) {
        if value {
            self.flags |= Self::INTERLACED;
        } else {
            self.flags &= !Self::INTERLACED;
        }
    }

    pub fn is_top_field_first(&self) -> bool {
        self.flags & Self::TOP_FIELD_FIRST != 0
    }

    pub fn set_top_field_first(&mut self, value: bool) {
        if value {
            self.flags |= Self::TOP_FIELD_FIRST;
        } else {
            self.flags &= !Self::TOP_FIELD_FIRST;
        }
    }
}

impl Default for FrameFlags {
    fn default() -> Self {
        Self::new()
    }
}
