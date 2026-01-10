//! Packet structure for container-level compressed media data

use crate::types::{MediaType, PacketFlags};

/// A packet containing compressed/encoded media data at the container level.
///
/// Packets are the unit of data that is read from demuxers and written to muxers.
/// They contain compressed data (e.g., H.264 NAL units, AAC frames) along with
/// timing and metadata information.
#[derive(Debug, Clone)]
pub struct Packet {
    /// The compressed data
    data: Vec<u8>,

    /// Presentation timestamp in time_base units
    pts: Option<i64>,

    /// Decoding timestamp in time_base units
    dts: Option<i64>,

    /// Duration in time_base units
    duration: Option<i64>,

    /// Stream index this packet belongs to
    stream_index: usize,

    /// Type of media in this packet
    media_type: MediaType,

    /// Packet flags (keyframe, corrupt, etc.)
    flags: PacketFlags,

    /// Byte position in the stream (-1 if unknown)
    position: i64,
}

impl Packet {
    /// Creates a new packet with the given data
    pub fn new(data: Vec<u8>, stream_index: usize, media_type: MediaType) -> Self {
        Self {
            data,
            pts: None,
            dts: None,
            duration: None,
            stream_index,
            media_type,
            flags: PacketFlags::new(),
            position: -1,
        }
    }

    /// Creates an empty packet
    pub fn empty(stream_index: usize, media_type: MediaType) -> Self {
        Self::new(Vec::new(), stream_index, media_type)
    }

    /// Returns a reference to the packet data
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable reference to the packet data
    pub fn data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.data
    }

    /// Consumes the packet and returns the data
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Returns the size of the packet data in bytes
    pub fn size(&self) -> usize {
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

    /// Returns the decoding timestamp
    pub fn dts(&self) -> Option<i64> {
        self.dts
    }

    /// Sets the decoding timestamp
    pub fn set_dts(&mut self, dts: Option<i64>) {
        self.dts = dts;
    }

    /// Builder method to set decoding timestamp
    pub fn with_dts(mut self, dts: i64) -> Self {
        self.dts = Some(dts);
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

    /// Returns the stream index
    pub fn stream_index(&self) -> usize {
        self.stream_index
    }

    /// Sets the stream index
    pub fn set_stream_index(&mut self, index: usize) {
        self.stream_index = index;
    }

    /// Returns the media type
    pub fn media_type(&self) -> MediaType {
        self.media_type
    }

    /// Sets the media type
    pub fn set_media_type(&mut self, media_type: MediaType) {
        self.media_type = media_type;
    }

    /// Returns a reference to the packet flags
    pub fn flags(&self) -> &PacketFlags {
        &self.flags
    }

    /// Returns a mutable reference to the packet flags
    pub fn flags_mut(&mut self) -> &mut PacketFlags {
        &mut self.flags
    }

    /// Builder method to set flags
    pub fn with_flags(mut self, flags: PacketFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Returns whether this is a keyframe packet
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

    /// Returns the byte position in the stream
    pub fn position(&self) -> i64 {
        self.position
    }

    /// Sets the byte position in the stream
    pub fn set_position(&mut self, position: i64) {
        self.position = position;
    }

    /// Builder method to set position
    pub fn with_position(mut self, position: i64) -> Self {
        self.position = position;
        self
    }

    /// Clears the packet data and resets timestamps
    pub fn clear(&mut self) {
        self.data.clear();
        self.pts = None;
        self.dts = None;
        self.duration = None;
        self.flags = PacketFlags::new();
        self.position = -1;
    }
}

impl Default for Packet {
    fn default() -> Self {
        Self::empty(0, MediaType::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_creation() {
        let data = vec![0x00, 0x01, 0x02, 0x03];
        let packet = Packet::new(data.clone(), 0, MediaType::Video);

        assert_eq!(packet.data(), &data[..]);
        assert_eq!(packet.size(), 4);
        assert_eq!(packet.stream_index(), 0);
        assert_eq!(packet.media_type(), MediaType::Video);
        assert_eq!(packet.pts(), None);
        assert_eq!(packet.dts(), None);
    }

    #[test]
    fn test_packet_builder() {
        let packet = Packet::new(vec![0xFF], 1, MediaType::Audio)
            .with_pts(1000)
            .with_dts(900)
            .with_duration(100)
            .with_keyframe()
            .with_position(12345);

        assert_eq!(packet.pts(), Some(1000));
        assert_eq!(packet.dts(), Some(900));
        assert_eq!(packet.duration(), Some(100));
        assert!(packet.is_keyframe());
        assert_eq!(packet.position(), 12345);
    }

    #[test]
    fn test_packet_flags() {
        let mut packet = Packet::empty(0, MediaType::Video);
        assert!(!packet.is_keyframe());

        packet.set_keyframe(true);
        assert!(packet.is_keyframe());

        packet.flags_mut().set_corrupt(true);
        assert!(packet.flags().is_corrupt());
    }

    #[test]
    fn test_packet_clear() {
        let mut packet = Packet::new(vec![1, 2, 3], 0, MediaType::Video)
            .with_pts(100)
            .with_keyframe();

        packet.clear();

        assert_eq!(packet.size(), 0);
        assert_eq!(packet.pts(), None);
        assert!(!packet.is_keyframe());
    }
}
