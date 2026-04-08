//! Frame reorder buffer for presentation-order output
//!
//! When decoding codecs with B-frames (e.g., H.264), frames are produced in
//! decode order which may differ from presentation (PTS) order. Encoders
//! typically require frames in monotonically increasing PTS order.
//!
//! `FrameReorderBuffer` buffers decoded frames and outputs them sorted by PTS.

use crate::Frame;
use std::collections::BinaryHeap;
use std::cmp::Ordering;

/// Default maximum number of frames to buffer before forcing output.
const DEFAULT_MAX_BUFFER_SIZE: usize = 16;

/// Wrapper to order frames by PTS (min-heap: lowest PTS first).
struct PtsFrame(Frame);

impl PartialEq for PtsFrame {
    fn eq(&self, other: &Self) -> bool {
        self.0.pts() == other.0.pts()
    }
}

impl Eq for PtsFrame {}

impl PartialOrd for PtsFrame {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PtsFrame {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap (BinaryHeap is a max-heap)
        other.0.pts().cmp(&self.0.pts())
    }
}

/// A buffer that reorders decoded frames into presentation (PTS) order.
///
/// Frames are pushed in decode order and popped in PTS order. The buffer
/// holds up to `max_buffer_size` frames before forcing output of the
/// lowest-PTS frame.
///
/// Frames with `None` PTS are assigned a synthetic PTS one unit beyond the
/// highest PTS seen so far, preserving their relative decode order while
/// placing them after all explicitly-timestamped frames.
///
/// # Usage
///
/// ```rust,ignore
/// let mut reorder = FrameReorderBuffer::new();
///
/// // Push decoded frames (may be out of PTS order)
/// for frame in decoded_frames {
///     reorder.push(frame);
///     while let Some(ordered_frame) = reorder.pop_ready() {
///         encoder.send_frame(&ordered_frame)?;
///     }
/// }
///
/// // Flush remaining frames at end of stream
/// while let Some(frame) = reorder.flush_next() {
///     encoder.send_frame(&frame)?;
/// }
/// ```
pub struct FrameReorderBuffer {
    heap: BinaryHeap<PtsFrame>,
    max_buffer_size: usize,
    /// Tracks the highest PTS seen, for synthesizing PTS on None-PTS frames.
    max_pts: i64,
}

impl FrameReorderBuffer {
    /// Creates a new reorder buffer with the default max buffer size (16).
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            max_buffer_size: DEFAULT_MAX_BUFFER_SIZE,
            max_pts: 0,
        }
    }

    /// Creates a new reorder buffer with a custom max buffer size.
    pub fn with_max_buffer_size(max_buffer_size: usize) -> Self {
        Self {
            heap: BinaryHeap::new(),
            max_buffer_size: max_buffer_size.max(1),
            max_pts: 0,
        }
    }

    /// Push a decoded frame into the buffer.
    ///
    /// If the frame has no PTS, a synthetic PTS is assigned beyond the
    /// highest PTS seen so far.
    pub fn push(&mut self, mut frame: Frame) {
        match frame.pts() {
            Some(pts) => {
                if pts > self.max_pts {
                    self.max_pts = pts;
                }
            }
            None => {
                // Assign synthetic PTS after all known frames
                self.max_pts += 1;
                frame.set_pts(Some(self.max_pts));
            }
        }
        self.heap.push(PtsFrame(frame));
    }

    /// Pop a frame if the buffer is full (ready to guarantee ordering).
    ///
    /// Returns the frame with the lowest PTS when the buffer has reached
    /// its max size, meaning enough frames have been buffered to be
    /// confident about ordering. Returns `None` if more frames are needed.
    pub fn pop_ready(&mut self) -> Option<Frame> {
        if self.heap.len() > self.max_buffer_size {
            self.heap.pop().map(|pf| pf.0)
        } else {
            None
        }
    }

    /// Pop the next frame in PTS order during flush.
    ///
    /// Call repeatedly at end-of-stream to drain all buffered frames.
    pub fn flush_next(&mut self) -> Option<Frame> {
        self.heap.pop().map(|pf| pf.0)
    }

    /// Returns the number of buffered frames.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns true if no frames are buffered.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

impl Default for FrameReorderBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;

    fn make_frame(pts: i64) -> Frame {
        let mut f = Frame::new_video(16, 16, PixelFormat::YUV420P);
        f.set_pts(Some(pts));
        f
    }

    #[test]
    fn test_reorder_basic() {
        let mut buf = FrameReorderBuffer::with_max_buffer_size(3);

        // Push frames out of order (decode order with B-frames)
        buf.push(make_frame(0)); // I
        buf.push(make_frame(3)); // P
        buf.push(make_frame(1)); // B
        assert!(buf.pop_ready().is_none()); // buffer size == max, not exceeded

        buf.push(make_frame(2)); // B - now 4 > 3, one frame ready
        let f = buf.pop_ready().unwrap();
        assert_eq!(f.pts(), Some(0));
        assert!(buf.pop_ready().is_none()); // back to 3, not exceeded

        // Flush remaining in PTS order
        assert_eq!(buf.flush_next().unwrap().pts(), Some(1));
        assert_eq!(buf.flush_next().unwrap().pts(), Some(2));
        assert_eq!(buf.flush_next().unwrap().pts(), Some(3));
        assert!(buf.flush_next().is_none());
    }

    #[test]
    fn test_flush_outputs_in_order() {
        let mut buf = FrameReorderBuffer::with_max_buffer_size(8);

        buf.push(make_frame(5));
        buf.push(make_frame(2));
        buf.push(make_frame(3));
        buf.push(make_frame(0));
        buf.push(make_frame(4));
        buf.push(make_frame(1));

        let mut pts_order = Vec::new();
        while let Some(f) = buf.flush_next() {
            pts_order.push(f.pts().unwrap());
        }
        assert_eq!(pts_order, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_empty_buffer() {
        let mut buf = FrameReorderBuffer::new();
        assert!(buf.pop_ready().is_none());
        assert!(buf.flush_next().is_none());
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_monotonic_pts_passes_through() {
        // When frames are already in order, they still come out correctly
        let mut buf = FrameReorderBuffer::with_max_buffer_size(2);

        buf.push(make_frame(0));
        buf.push(make_frame(1));
        assert!(buf.pop_ready().is_none());

        buf.push(make_frame(2));
        assert_eq!(buf.pop_ready().unwrap().pts(), Some(0));

        buf.push(make_frame(3));
        assert_eq!(buf.pop_ready().unwrap().pts(), Some(1));

        // Flush rest
        assert_eq!(buf.flush_next().unwrap().pts(), Some(2));
        assert_eq!(buf.flush_next().unwrap().pts(), Some(3));
        assert!(buf.flush_next().is_none());
    }
}
