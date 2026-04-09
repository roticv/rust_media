//! H.264/AVC video decoder implementation using Apple VideoToolbox
//!
//! Provides hardware-accelerated H.264 decoding on macOS using Apple's VideoToolbox framework.
//!
//! # Platform Support
//!
//! This decoder is only available on macOS. It requires the `videotoolbox` feature flag.
//!
//! # Profile Support
//!
//! VideoToolbox supports all H.264 profiles including:
//! - Baseline Profile
//! - Main Profile
//! - High Profile (most common for commercial content)
//! - High 10 Profile (10-bit)
//!
//! This makes it suitable for decoding commercial H.264 content that OpenH264 cannot handle.
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_media_core::{StreamInfo, MediaType, VideoStreamParams, PixelFormat};
//! use rust_media_codec::VideoToolboxH264Decoder;
//!
//! let video_params = VideoStreamParams::new(1920, 1080, PixelFormat::YUV420P);
//! let mut stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
//!     .with_time_base(1, 90000)
//!     .with_params(rust_media_core::StreamParams::Video(video_params));
//! // extra_data should contain AVCDecoderConfigurationRecord from MP4
//!
//! let mut decoder = VideoToolboxH264Decoder::new(stream_info)?;
//! // ... decode packets using send_packet() and receive_frame()
//! ```

use objc2::rc::Retained;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained};
use objc2_core_media::{
    CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime,
    CMTimeFlags, CMVideoFormatDescription,
};
use objc2_core_video::{kCVPixelBufferPixelFormatTypeKey, CVPixelBuffer};
use objc2_video_toolbox::VTDecompressionSession;
use rust_media_core::{
    Decoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};
use std::collections::VecDeque;
use std::ptr;
use std::sync::{Arc, Mutex};

/// H.264/AVC video decoder using Apple VideoToolbox (hardware accelerated)
///
/// Decodes H.264-compressed video packets into raw YUV frames using macOS hardware acceleration.
///
/// # Advantages over OpenH264
///
/// - Supports all H.264 profiles (Baseline, Main, High, High 10)
/// - Hardware accelerated - significantly faster than software decoding
/// - Lower CPU usage - ideal for real-time applications
///
/// # Notes
///
/// - macOS only (requires `videotoolbox` feature)
/// - Input packets should be in AVCC format (from MP4) or Annex B format
/// - Output is YUV420P (NV12 internally, converted to I420)
pub struct VideoToolboxH264Decoder {
    stream_info: StreamInfo,
    format_description: Option<Retained<CMVideoFormatDescription>>,
    session: Option<Retained<VTDecompressionSession>>,
    buffered_frames: Arc<Mutex<VecDeque<Frame>>>,
    flushed: bool,
    /// NAL unit length size in bytes (1, 2, or 4) from AVCDecoderConfigurationRecord
    #[allow(dead_code)]
    nal_length_size: usize,
    width: usize,
    height: usize,
    /// Callback context - must be kept alive for session lifetime
    #[allow(dead_code)]
    callback_context: Option<Box<DecompressionContext>>,
}

impl VideoToolboxH264Decoder {
    /// Creates a new VideoToolbox H.264 decoder from stream information
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream information (codec must be "h264" or "avc").
    ///   `extra_data` should contain AVCDecoderConfigurationRecord
    ///
    /// # Returns
    ///
    /// A new VideoToolboxH264Decoder or an error if initialization fails
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec
        if stream_info.codec != "h264" && stream_info.codec != "avc" {
            return Err(Error::Unsupported(format!(
                "Expected h264/avc codec, got {}",
                stream_info.codec
            )));
        }

        // Extract dimensions from stream info
        let (width, height) = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => (params.width, params.height),
            _ => {
                return Err(Error::InvalidData(
                    "Expected video stream params".to_string(),
                ))
            }
        };

        // Parse AVCDecoderConfigurationRecord if present
        let nal_length_size = if !stream_info.extra_data.is_empty() {
            parse_nal_length_size(&stream_info.extra_data)?
        } else {
            4 // Default to 4-byte length prefix
        };

        let mut decoder = Self {
            stream_info,
            format_description: None,
            session: None,
            buffered_frames: Arc::new(Mutex::new(VecDeque::new())),
            flushed: false,
            nal_length_size,
            width,
            height,
            callback_context: None,
        };

        // Initialize the decompression session if we have extra_data
        if !decoder.stream_info.extra_data.is_empty() {
            decoder.initialize_session()?;
        }

        Ok(decoder)
    }

    /// Initialize the VTDecompressionSession with the format description
    fn initialize_session(&mut self) -> Result<()> {
        // Create format description from AVCDecoderConfigurationRecord
        let format_desc = create_format_description(&self.stream_info.extra_data)?;
        self.format_description = Some(format_desc.clone());

        // H.264 decoder always outputs 8-bit (NV12) for now. High 10 profile
        // is rare and not part of the current 10-bit story.
        let (session, ctx) = create_decompression_session(
            &format_desc,
            self.buffered_frames.clone(),
            self.width,
            self.height,
            None,
        )?;
        self.session = Some(session);
        self.callback_context = Some(ctx);

        Ok(())
    }

    /// Converts AVCC format data to a CMSampleBuffer for decoding
    fn create_sample_buffer(
        &self,
        data: &[u8],
        pts: Option<i64>,
    ) -> Result<Retained<CMSampleBuffer>> {
        let format_desc = self
            .format_description
            .as_ref()
            .ok_or_else(|| Error::InvalidData("Format description not initialized".to_string()))?;

        // Create block buffer from packet data
        let block_buffer = create_block_buffer(data)?;

        // Create timing info
        let timing = CMSampleTimingInfo {
            duration: CMTime {
                value: 1,
                timescale: 30, // Will be overridden by actual timing
                flags: CMTimeFlags(1), // kCMTimeFlags_Valid
                epoch: 0,
            },
            presentationTimeStamp: CMTime {
                value: pts.unwrap_or(0),
                timescale: self.stream_info.time_base.1 as i32,
                flags: CMTimeFlags(1), // kCMTimeFlags_Valid
                epoch: 0,
            },
            decodeTimeStamp: CMTime {
                value: 0,
                timescale: 0,
                flags: CMTimeFlags(0), // kCMTimeFlags_Invalid
                epoch: 0,
            },
        };

        // Create sample buffer
        create_sample_buffer_from_block_buffer(&block_buffer, format_desc, &timing, data.len())
    }
}

/// Parse NAL length size from AVCDecoderConfigurationRecord
fn parse_nal_length_size(data: &[u8]) -> Result<usize> {
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
    Ok((length_size_minus_one + 1) as usize)
}

/// Create a CMVideoFormatDescription from AVCDecoderConfigurationRecord
fn create_format_description(avcc_data: &[u8]) -> Result<Retained<CMVideoFormatDescription>> {
    unsafe {
        let mut format_desc: *mut CMFormatDescription = ptr::null_mut();

        // The avcC data contains the full configuration
        // We need to extract SPS and PPS to create the format description
        let (sps_list, pps_list) = parse_avcc_parameter_sets(avcc_data)?;

        if sps_list.is_empty() {
            return Err(Error::InvalidData("No SPS found in avcC".to_string()));
        }

        // Create pointers for parameter sets
        let mut param_set_pointers: Vec<*const u8> = Vec::new();
        let mut param_set_sizes: Vec<usize> = Vec::new();

        for sps in &sps_list {
            param_set_pointers.push(sps.as_ptr());
            param_set_sizes.push(sps.len());
        }

        for pps in &pps_list {
            param_set_pointers.push(pps.as_ptr());
            param_set_sizes.push(pps.len());
        }

        let nal_unit_header_length = ((avcc_data[4] & 0x03) + 1) as i32;

        let status = CMVideoFormatDescriptionCreateFromH264ParameterSets(
            ptr::null(), // allocator
            param_set_pointers.len(),
            param_set_pointers.as_ptr(),
            param_set_sizes.as_ptr(),
            nal_unit_header_length,
            &mut format_desc,
        );

        if status != 0 {
            return Err(Error::Decode(format!(
                "Failed to create format description: {}",
                status
            )));
        }

        if format_desc.is_null() {
            return Err(Error::Decode(
                "Format description is null".to_string(),
            ));
        }

        // Safety: format_desc is non-null and we own it
        Ok(Retained::from_raw(format_desc as *mut CMVideoFormatDescription).unwrap())
    }
}

/// (SPS list, PPS list) parsed from an AVCDecoderConfigurationRecord
type AvcParameterSets = (Vec<Vec<u8>>, Vec<Vec<u8>>);

/// Parse SPS and PPS from AVCDecoderConfigurationRecord
fn parse_avcc_parameter_sets(data: &[u8]) -> Result<AvcParameterSets> {
    if data.len() < 7 {
        return Err(Error::InvalidData(
            "AVCDecoderConfigurationRecord too short".to_string(),
        ));
    }

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
        return Ok((sps_list, Vec::new()));
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

    Ok((sps_list, pps_list))
}

/// Callback context passed to VideoToolbox decompression callback
struct DecompressionContext {
    output_frames: Arc<Mutex<VecDeque<Frame>>>,
    width: usize,
    height: usize,
}

/// C callback function for VTDecompressionSession
#[allow(non_snake_case)]
extern "C" fn decompression_output_callback(
    decompressionOutputRefCon: *mut std::ffi::c_void,
    _sourceFrameRefCon: *mut std::ffi::c_void,
    status: i32,
    _infoFlags: u32,
    imageBuffer: *mut CVPixelBuffer,
    presentationTimeStamp: CMTime,
    _presentationDuration: CMTime,
) {
    if status != 0 {
        eprintln!("VideoToolbox decode error: {}", status);
        return;
    }

    if imageBuffer.is_null() {
        return;
    }

    // Get context
    let ctx = unsafe { &*(decompressionOutputRefCon as *const DecompressionContext) };

    // Convert pixel buffer to frame
    let pts = if presentationTimeStamp.flags.0 & 1 != 0 {
        Some(presentationTimeStamp.value)
    } else {
        None
    };

    match pixel_buffer_to_frame(
        unsafe { &*(imageBuffer as *const CVPixelBuffer) },
        pts,
        ctx.width,
        ctx.height,
    ) {
        Ok(frame) => {
            if let Ok(mut frames) = ctx.output_frames.lock() {
                frames.push_back(frame);
            }
        }
        Err(e) => {
            eprintln!("Failed to convert pixel buffer to frame: {}", e);
        }
    }
}

/// VTDecompressionOutputCallbackRecord structure
#[repr(C)]
#[allow(non_snake_case)]
struct VTDecompressionOutputCallbackRecord {
    decompressionOutputCallback: extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        i32,
        u32,
        *mut CVPixelBuffer,
        CMTime,
        CMTime,
    ),
    decompressionOutputRefCon: *mut std::ffi::c_void,
}

/// Create a VTDecompressionSession with proper output callback.
///
/// `output_pixel_format`, when set, forces VideoToolbox to deliver pixel buffers
/// in that exact format via `kCVPixelBufferPixelFormatTypeKey`. We use this to
/// pin 10-bit HEVC output to the documented `'x420'` (P010) format instead of
/// whatever undocumented internal format VideoToolbox might otherwise pick.
fn create_decompression_session(
    format_desc: &CMVideoFormatDescription,
    output_frames: Arc<Mutex<VecDeque<Frame>>>,
    width: usize,
    height: usize,
    output_pixel_format: Option<u32>,
) -> Result<(Retained<VTDecompressionSession>, Box<DecompressionContext>)> {
    unsafe {
        let mut session: *mut VTDecompressionSession = ptr::null_mut();

        // Create context for callback (must be kept alive for session lifetime)
        let ctx = Box::new(DecompressionContext {
            output_frames,
            width,
            height,
        });

        // Create callback record
        let callback_record = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: decompression_output_callback,
            decompressionOutputRefCon: &*ctx as *const _ as *mut std::ffi::c_void,
        };

        // Build destination image buffer attributes if a pixel format is requested.
        // The CFDictionary (and the CFNumber it borrows) must outlive the call
        // to VTDecompressionSessionCreate.
        let fmt_number = output_pixel_format.map(|fmt| CFNumber::new_i32(fmt as i32));
        let dest_attrs = fmt_number.as_ref().map(|num| {
            let key: &objc2_core_foundation::CFType = kCVPixelBufferPixelFormatTypeKey;
            let value: &objc2_core_foundation::CFType = num;
            CFDictionary::<objc2_core_foundation::CFType, objc2_core_foundation::CFType>::from_slices(
                &[key],
                &[value],
            )
        });

        let dest_attrs_ptr: *const std::ffi::c_void = match &dest_attrs {
            Some(d) => CFRetained::as_ptr(d).as_ptr() as *const std::ffi::c_void,
            None => ptr::null(),
        };

        let status = VTDecompressionSessionCreate(
            ptr::null(),                                     // allocator
            format_desc as *const CMFormatDescription,
            ptr::null(),                                     // videoDecoderSpecification
            dest_attrs_ptr,                                  // destinationImageBufferAttributes
            &callback_record as *const _ as *const std::ffi::c_void,
            &mut session,
        );

        if status != 0 {
            return Err(Error::Decode(format!(
                "Failed to create decompression session: {}",
                status
            )));
        }

        if session.is_null() {
            return Err(Error::Decode(
                "Decompression session is null".to_string(),
            ));
        }

        Ok((Retained::from_raw(session).unwrap(), ctx))
    }
}

/// Create a CMBlockBuffer from packet data
fn create_block_buffer(data: &[u8]) -> Result<Retained<CMBlockBuffer>> {
    unsafe {
        let mut block_buffer: *mut CMBlockBuffer = ptr::null_mut();

        let status = CMBlockBufferCreateWithMemoryBlock(
            ptr::null(),           // allocator
            ptr::null_mut(),       // memoryBlock (NULL = allocate)
            data.len(),            // blockLength
            ptr::null(),           // blockAllocator
            ptr::null(),           // customBlockSource
            0,                     // offsetToData
            data.len(),            // dataLength
            0,                     // flags
            &mut block_buffer,
        );

        if status != 0 {
            return Err(Error::Decode(format!(
                "Failed to create block buffer: {}",
                status
            )));
        }

        // Copy data into block buffer
        let status = CMBlockBufferReplaceDataBytes(
            data.as_ptr() as *const _,
            block_buffer,
            0,
            data.len(),
        );

        if status != 0 {
            return Err(Error::Decode(format!(
                "Failed to copy data to block buffer: {}",
                status
            )));
        }

        Ok(Retained::from_raw(block_buffer).unwrap())
    }
}

/// Create a CMSampleBuffer from a block buffer
fn create_sample_buffer_from_block_buffer(
    block_buffer: &CMBlockBuffer,
    format_desc: &CMVideoFormatDescription,
    timing: &CMSampleTimingInfo,
    data_length: usize,
) -> Result<Retained<CMSampleBuffer>> {
    unsafe {
        let mut sample_buffer: *mut CMSampleBuffer = ptr::null_mut();
        let sample_size = data_length;

        let status = CMSampleBufferCreateReady(
            ptr::null(),                                     // allocator
            block_buffer as *const _ as *mut CMBlockBuffer,
            format_desc as *const _ as *mut CMFormatDescription,
            1,                                               // numSamples
            1,                                               // numSampleTimingEntries
            timing,                                          // sampleTimingArray
            1,                                               // numSampleSizeEntries
            &sample_size,                                    // sampleSizeArray
            &mut sample_buffer,
        );

        if status != 0 {
            return Err(Error::Decode(format!(
                "Failed to create sample buffer: {}",
                status
            )));
        }

        Ok(Retained::from_raw(sample_buffer).unwrap())
    }
}

// CVPixelBuffer pixel format codes (FourCC).
//
// Each constant corresponds to a `kCVPixelFormatType_*` value from CoreVideo.
// VideoToolbox picks one of these based on the source bit depth and our
// (currently empty) destination buffer attributes.
const PF_420V: u32 = 0x34323076; // '420v' — NV12, 8-bit, video range
const PF_420F: u32 = 0x34323066; // '420f' — I420, 8-bit, full range
const PF_X420: u32 = 0x78343230; // 'x420' — P010, 10-bit MSB-aligned, video range
const PF_XF20: u32 = 0x78663230; // 'xf20' — P010, 10-bit MSB-aligned, full range

/// Convert CVPixelBuffer to Frame
fn pixel_buffer_to_frame(
    pixel_buffer: &CVPixelBuffer,
    pts: Option<i64>,
    expected_width: usize,
    expected_height: usize,
) -> Result<Frame> {
    unsafe {
        // Lock the pixel buffer for reading
        let lock_status = CVPixelBufferLockBaseAddress(pixel_buffer as *const _ as *mut _, 1); // kCVPixelBufferLock_ReadOnly
        if lock_status != 0 {
            return Err(Error::Decode(format!(
                "Failed to lock pixel buffer: {}",
                lock_status
            )));
        }

        let width = CVPixelBufferGetWidth(pixel_buffer as *const _ as *mut _);
        let height = CVPixelBufferGetHeight(pixel_buffer as *const _ as *mut _);
        let pixel_format = CVPixelBufferGetPixelFormatType(pixel_buffer as *const _ as *mut _);

        // Use actual dimensions from pixel buffer, but log if different from expected
        if width != expected_width || height != expected_height {
            eprintln!(
                "Warning: Pixel buffer dimensions {}x{} differ from expected {}x{}",
                width, height, expected_width, expected_height
            );
        }

        // Choose output PixelFormat based on the source pixel buffer format.
        let output_format = match pixel_format {
            PF_420V | PF_420F => PixelFormat::YUV420P,
            PF_X420 | PF_XF20 => PixelFormat::YUV420P10LE,
            other => {
                CVPixelBufferUnlockBaseAddress(pixel_buffer as *const _ as *mut _, 1);
                return Err(Error::Unsupported(format!(
                    "Unsupported pixel format: 0x{:08x}",
                    other
                )));
            }
        };

        let mut frame = Frame::new_video(width, height, output_format);
        frame.set_pts(pts);

        let result = match pixel_format {
            PF_420V => convert_nv12_to_yuv420p(pixel_buffer, &mut frame, width, height),
            PF_420F => convert_i420_to_yuv420p(pixel_buffer, &mut frame, width, height),
            PF_X420 | PF_XF20 => {
                convert_p010_to_yuv420p10le(pixel_buffer, &mut frame, width, height)
            }
            _ => unreachable!("format dispatch handled above"),
        };

        // Always unlock, even on conversion failure
        CVPixelBufferUnlockBaseAddress(pixel_buffer as *const _ as *mut _, 1);
        result?;

        Ok(frame)
    }
}

/// NV12 (8-bit bi-planar Y + interleaved UV) → YUV420P (planar I420).
unsafe fn convert_nv12_to_yuv420p(
    pixel_buffer: &CVPixelBuffer,
    frame: &mut Frame,
    width: usize,
    height: usize,
) -> Result<()> {
    let y_plane = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 0);
    let uv_plane = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 1);
    let y_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 0);
    let uv_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 1);

    // Copy Y plane
    if let Some(y_frame_plane) = frame.plane_mut(0) {
        for row in 0..height {
            let src = y_plane.add(row * y_stride);
            let dst_start = row * width;
            std::ptr::copy_nonoverlapping(
                src as *const u8,
                y_frame_plane[dst_start..].as_mut_ptr(),
                width,
            );
        }
    }

    // Deinterleave UV → planar U, V
    let uv_height = height / 2;
    let uv_width = width / 2;

    let mut u_data = vec![0u8; uv_width * uv_height];
    let mut v_data = vec![0u8; uv_width * uv_height];

    for row in 0..uv_height {
        let src_row = uv_plane.add(row * uv_stride) as *const u8;
        for col in 0..uv_width {
            let idx = row * uv_width + col;
            u_data[idx] = *src_row.add(col * 2);
            v_data[idx] = *src_row.add(col * 2 + 1);
        }
    }

    if let Some(u_plane_dst) = frame.plane_mut(1) {
        u_plane_dst[..u_data.len()].copy_from_slice(&u_data);
    }
    if let Some(v_plane_dst) = frame.plane_mut(2) {
        v_plane_dst[..v_data.len()].copy_from_slice(&v_data);
    }

    Ok(())
}

/// I420 (8-bit planar Y, U, V) → YUV420P (already the same layout, just copy).
unsafe fn convert_i420_to_yuv420p(
    pixel_buffer: &CVPixelBuffer,
    frame: &mut Frame,
    width: usize,
    height: usize,
) -> Result<()> {
    let y_plane_ptr = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 0);
    let u_plane_ptr = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 1);
    let v_plane_ptr = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 2);
    let y_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 0);
    let u_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 1);
    let v_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 2);

    if let Some(y_frame_plane) = frame.plane_mut(0) {
        for row in 0..height {
            let src = y_plane_ptr.add(row * y_stride);
            let dst_start = row * width;
            std::ptr::copy_nonoverlapping(
                src as *const u8,
                y_frame_plane[dst_start..].as_mut_ptr(),
                width,
            );
        }
    }

    let uv_height = height / 2;
    let uv_width = width / 2;
    if let Some(u_frame_plane) = frame.plane_mut(1) {
        for row in 0..uv_height {
            let src = u_plane_ptr.add(row * u_stride);
            let dst_start = row * uv_width;
            std::ptr::copy_nonoverlapping(
                src as *const u8,
                u_frame_plane[dst_start..].as_mut_ptr(),
                uv_width,
            );
        }
    }

    if let Some(v_frame_plane) = frame.plane_mut(2) {
        for row in 0..uv_height {
            let src = v_plane_ptr.add(row * v_stride);
            let dst_start = row * uv_width;
            std::ptr::copy_nonoverlapping(
                src as *const u8,
                v_frame_plane[dst_start..].as_mut_ptr(),
                uv_width,
            );
        }
    }

    Ok(())
}

/// P010 (10-bit bi-planar, MSB-aligned) → YUV420P10LE (planar, value in LSBs).
///
/// P010 layout:
/// - Plane 0 (Y): one u16 per luma sample
/// - Plane 1 (UV): two u16s per chroma site, interleaved (U, V)
///
/// Each 16-bit container stores the 10-bit value in the **upper 10 bits**
/// (i.e. shifted left by 6). Our `YUV420P10LE` format expects the 10-bit
/// value in the lower 10 bits, so we right-shift by 6 and clamp to 10 bits.
unsafe fn convert_p010_to_yuv420p10le(
    pixel_buffer: &CVPixelBuffer,
    frame: &mut Frame,
    width: usize,
    height: usize,
) -> Result<()> {
    let y_plane = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 0);
    let uv_plane = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer as *const _ as *mut _, 1);
    let y_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 0);
    let uv_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer as *const _ as *mut _, 1);

    // --- Y plane ---
    if let Some(y_frame_plane) = frame.plane_mut(0) {
        for row in 0..height {
            let src_row = y_plane.add(row * y_stride) as *const u8;
            let dst_off = row * width * 2;
            for col in 0..width {
                // Read u16 from source (P010 is little-endian on Apple platforms)
                let lo = *src_row.add(col * 2);
                let hi = *src_row.add(col * 2 + 1);
                let p010 = u16::from_le_bytes([lo, hi]);
                // 10-bit value lives in the upper 10 bits → shift right by 6
                let val10 = (p010 >> 6) & 0x3FF;
                // Write little-endian u16 into the YUV420P10LE plane
                let bytes = val10.to_le_bytes();
                y_frame_plane[dst_off + col * 2] = bytes[0];
                y_frame_plane[dst_off + col * 2 + 1] = bytes[1];
            }
        }
    }

    // --- UV plane → split into planar U, V ---
    let uv_height = height / 2;
    let uv_width = width / 2;

    // Stage into temporary buffers to avoid borrowing two frame planes at once.
    let mut u_data = vec![0u8; uv_width * uv_height * 2];
    let mut v_data = vec![0u8; uv_width * uv_height * 2];

    for row in 0..uv_height {
        let src_row = uv_plane.add(row * uv_stride) as *const u8;
        for col in 0..uv_width {
            // P010 UV plane: each chroma sample pair is 4 bytes (u16 U, u16 V)
            let pair = src_row.add(col * 4);
            let u_lo = *pair;
            let u_hi = *pair.add(1);
            let v_lo = *pair.add(2);
            let v_hi = *pair.add(3);

            let u10 = (u16::from_le_bytes([u_lo, u_hi]) >> 6) & 0x3FF;
            let v10 = (u16::from_le_bytes([v_lo, v_hi]) >> 6) & 0x3FF;

            let dst_idx = (row * uv_width + col) * 2;
            let u_bytes = u10.to_le_bytes();
            let v_bytes = v10.to_le_bytes();
            u_data[dst_idx] = u_bytes[0];
            u_data[dst_idx + 1] = u_bytes[1];
            v_data[dst_idx] = v_bytes[0];
            v_data[dst_idx + 1] = v_bytes[1];
        }
    }

    if let Some(u_plane_dst) = frame.plane_mut(1) {
        u_plane_dst[..u_data.len()].copy_from_slice(&u_data);
    }
    if let Some(v_plane_dst) = frame.plane_mut(2) {
        v_plane_dst[..v_data.len()].copy_from_slice(&v_data);
    }

    Ok(())
}

impl Decoder for VideoToolboxH264Decoder {
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

        // Initialize session on first packet if not already done
        if self.session.is_none() {
            if self.stream_info.extra_data.is_empty() {
                return Err(Error::InvalidData(
                    "No AVCDecoderConfigurationRecord available".to_string(),
                ));
            }
            self.initialize_session()?;
        }

        let session = self.session.as_ref().unwrap();
        let pts = packet.pts();
        let data = packet.data();

        // Create sample buffer
        let sample_buffer = self.create_sample_buffer(data, pts)?;

        // Decode the frame (callback will receive the output)
        unsafe {
            let mut info_flags: u32 = 0;
            let session_ptr: *mut VTDecompressionSession = &**session as *const _ as *mut _;

            let status = VTDecompressionSessionDecodeFrame(
                session_ptr,
                &*sample_buffer as *const _ as *mut CMSampleBuffer,
                0, // decodeFlags
                ptr::null_mut(), // sourceFrameRefCon
                &mut info_flags,
            );

            if status != 0 && status != -12909 {
                // -12909 = noErr but no frame output yet
                return Err(Error::Decode(format!(
                    "VTDecompressionSessionDecodeFrame failed: {}",
                    status
                )));
            }

            // Wait for callback to complete - ignore errors as the callback handles frame delivery
            let _ = VTDecompressionSessionWaitForAsynchronousFrames(session_ptr);
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        let mut frames = self.buffered_frames.lock().unwrap();
        if let Some(frame) = frames.pop_front() {
            Ok(frame)
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        if let Some(session) = &self.session {
            unsafe {
                let session_ptr: *mut VTDecompressionSession = &**session as *const _ as *mut _;
                // Finish any delayed frames - ignore errors
                let _ = VTDecompressionSessionFinishDelayedFrames(session_ptr);
                let _ = VTDecompressionSessionWaitForAsynchronousFrames(session_ptr);
            }
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // Invalidate current session
        if let Some(session) = self.session.take() {
            unsafe {
                let session_ptr: *const VTDecompressionSession = &*session;
                VTDecompressionSessionInvalidate(session_ptr as *mut _);
            }
        }

        // Drop callback context after session is invalidated
        self.callback_context = None;
        self.format_description = None;
        self.buffered_frames.lock().unwrap().clear();
        self.flushed = false;

        // Reinitialize if we have extra_data
        if !self.stream_info.extra_data.is_empty() {
            self.initialize_session()?;
        }

        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

// ============================================================================
// HEVC/H.265 VideoToolbox Decoder
// ============================================================================

/// H.265/HEVC video decoder using Apple VideoToolbox (hardware accelerated)
///
/// Decodes HEVC-compressed video packets into raw YUV frames using macOS hardware acceleration.
///
/// # Profile Support
///
/// VideoToolbox supports all HEVC profiles including:
/// - Main Profile (8-bit)
/// - Main 10 Profile (10-bit, HDR)
/// - Main Still Picture
///
/// # Notes
///
/// - macOS only (requires `videotoolbox` feature)
/// - Input packets should be in hvcC format (from MP4)
/// - Output is YUV420P (NV12 internally, converted to I420)
pub struct VideoToolboxHevcDecoder {
    stream_info: StreamInfo,
    format_description: Option<Retained<CMVideoFormatDescription>>,
    session: Option<Retained<VTDecompressionSession>>,
    buffered_frames: Arc<Mutex<VecDeque<Frame>>>,
    flushed: bool,
    #[allow(dead_code)]
    nal_length_size: usize,
    width: usize,
    height: usize,
    #[allow(dead_code)]
    callback_context: Option<Box<DecompressionContext>>,
}

impl VideoToolboxHevcDecoder {
    /// Creates a new VideoToolbox HEVC decoder from stream information
    ///
    /// `extra_data` should contain HEVCDecoderConfigurationRecord (hvcC)
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "hevc" && stream_info.codec != "h265" && stream_info.codec != "hvc1" && stream_info.codec != "hev1" {
            return Err(Error::Unsupported(format!(
                "Expected hevc/h265 codec, got {}",
                stream_info.codec
            )));
        }

        let (width, height) = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => (params.width, params.height),
            _ => {
                return Err(Error::InvalidData(
                    "Expected video stream params".to_string(),
                ))
            }
        };

        let nal_length_size = if stream_info.extra_data.len() >= 23 {
            // HEVCDecoderConfigurationRecord: lengthSizeMinusOne is bottom 2 bits of byte 21
            ((stream_info.extra_data[21] & 0x03) + 1) as usize
        } else {
            4
        };

        let mut decoder = Self {
            stream_info,
            format_description: None,
            session: None,
            buffered_frames: Arc::new(Mutex::new(VecDeque::new())),
            flushed: false,
            nal_length_size,
            width,
            height,
            callback_context: None,
        };

        if !decoder.stream_info.extra_data.is_empty() {
            decoder.initialize_session()?;
        }

        Ok(decoder)
    }

    fn initialize_session(&mut self) -> Result<()> {
        let format_desc = create_hevc_format_description(&self.stream_info.extra_data)?;
        self.format_description = Some(format_desc.clone());

        // For 10-bit HEVC, explicitly request the documented 'x420' (P010)
        // pixel format. Otherwise VideoToolbox may pick an undocumented
        // internal format like 'p420' that we don't know how to interpret.
        let bit_depth = parse_hvcc_bit_depth(&self.stream_info.extra_data).unwrap_or(8);
        let output_pixel_format = if bit_depth >= 10 {
            Some(PF_X420) // 'x420' = kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange
        } else {
            None
        };

        let (session, ctx) = create_decompression_session(
            &format_desc,
            self.buffered_frames.clone(),
            self.width,
            self.height,
            output_pixel_format,
        )?;
        self.session = Some(session);
        self.callback_context = Some(ctx);

        Ok(())
    }

    fn create_sample_buffer(
        &self,
        data: &[u8],
        pts: Option<i64>,
    ) -> Result<Retained<CMSampleBuffer>> {
        let format_desc = self
            .format_description
            .as_ref()
            .ok_or_else(|| Error::InvalidData("Format description not initialized".to_string()))?;

        let block_buffer = create_block_buffer(data)?;

        let timing = CMSampleTimingInfo {
            duration: CMTime {
                value: 1,
                timescale: 30,
                flags: CMTimeFlags(1),
                epoch: 0,
            },
            presentationTimeStamp: CMTime {
                value: pts.unwrap_or(0),
                timescale: self.stream_info.time_base.1 as i32,
                flags: CMTimeFlags(1),
                epoch: 0,
            },
            decodeTimeStamp: CMTime {
                value: 0,
                timescale: 0,
                flags: CMTimeFlags(0),
                epoch: 0,
            },
        };

        create_sample_buffer_from_block_buffer(&block_buffer, format_desc, &timing, data.len())
    }
}

/// Create a CMVideoFormatDescription from HEVCDecoderConfigurationRecord
fn create_hevc_format_description(hvcc_data: &[u8]) -> Result<Retained<CMVideoFormatDescription>> {
    unsafe {
        let param_sets = parse_hvcc_parameter_sets(hvcc_data)?;

        if param_sets.is_empty() {
            return Err(Error::InvalidData("No parameter sets found in hvcC".to_string()));
        }

        let mut param_set_pointers: Vec<*const u8> = Vec::new();
        let mut param_set_sizes: Vec<usize> = Vec::new();

        for ps in &param_sets {
            param_set_pointers.push(ps.as_ptr());
            param_set_sizes.push(ps.len());
        }

        let nal_unit_header_length = ((hvcc_data[21] & 0x03) + 1) as i32;

        let mut format_desc: *mut CMFormatDescription = ptr::null_mut();

        let status = CMVideoFormatDescriptionCreateFromHEVCParameterSets(
            ptr::null(),
            param_set_pointers.len(),
            param_set_pointers.as_ptr(),
            param_set_sizes.as_ptr(),
            nal_unit_header_length,
            ptr::null(), // extensions
            &mut format_desc,
        );

        if status != 0 {
            return Err(Error::Decode(format!(
                "Failed to create HEVC format description: {}",
                status
            )));
        }

        if format_desc.is_null() {
            return Err(Error::Decode(
                "HEVC format description is null".to_string(),
            ));
        }

        Ok(Retained::from_raw(format_desc as *mut CMVideoFormatDescription).unwrap())
    }
}

/// Extract `bit_depth_luma_minus_8` from an HEVCDecoderConfigurationRecord (hvcC).
///
/// Per ISO/IEC 14496-15, byte 17 of the hvcC record contains
/// `reserved (5 bits) | bitDepthLumaMinus8 (3 bits)`. Returns the actual luma
/// bit depth (8, 10, or 12), or `None` if the record is too short.
fn parse_hvcc_bit_depth(data: &[u8]) -> Option<u8> {
    if data.len() < 18 {
        return None;
    }
    let bit_depth_luma_minus_8 = data[17] & 0x07;
    Some(8 + bit_depth_luma_minus_8)
}

/// Parse VPS, SPS, and PPS from HEVCDecoderConfigurationRecord
///
/// hvcC structure (ISO 14496-15 section 8.3.3.1.2):
/// - bytes 0-21: configuration fields
/// - byte 22: numOfArrays
/// - For each array:
///   - byte 0: array_completeness (1 bit) + reserved (1 bit) + NAL_unit_type (6 bits)
///   - bytes 1-2: numNalus (u16)
///   - For each NAL:
///     - bytes 0-1: nalUnitLength (u16)
///     - nalUnitLength bytes: NAL unit data
fn parse_hvcc_parameter_sets(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    if data.len() < 23 {
        return Err(Error::InvalidData(
            "HEVCDecoderConfigurationRecord too short".to_string(),
        ));
    }

    let num_arrays = data[22] as usize;
    let mut pos = 23;
    let mut param_sets = Vec::new();

    for _ in 0..num_arrays {
        if pos + 3 > data.len() {
            break;
        }

        // Skip NAL type byte
        pos += 1;

        let num_nalus = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        for _ in 0..num_nalus {
            if pos + 2 > data.len() {
                break;
            }
            let nal_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;

            if pos + nal_length > data.len() {
                break;
            }
            param_sets.push(data[pos..pos + nal_length].to_vec());
            pos += nal_length;
        }
    }

    Ok(param_sets)
}

impl Decoder for VideoToolboxHevcDecoder {
    fn codec(&self) -> &str {
        "hevc"
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

        if self.session.is_none() {
            if self.stream_info.extra_data.is_empty() {
                return Err(Error::InvalidData(
                    "No HEVCDecoderConfigurationRecord available".to_string(),
                ));
            }
            self.initialize_session()?;
        }

        let session = self.session.as_ref().unwrap();
        let pts = packet.pts();
        let data = packet.data();

        let sample_buffer = self.create_sample_buffer(data, pts)?;

        unsafe {
            let mut info_flags: u32 = 0;
            let session_ptr: *mut VTDecompressionSession = &**session as *const _ as *mut _;

            let status = VTDecompressionSessionDecodeFrame(
                session_ptr,
                &*sample_buffer as *const _ as *mut CMSampleBuffer,
                0,
                ptr::null_mut(),
                &mut info_flags,
            );

            if status != 0 && status != -12909 {
                return Err(Error::Decode(format!(
                    "VTDecompressionSessionDecodeFrame failed: {}",
                    status
                )));
            }

            let _ = VTDecompressionSessionWaitForAsynchronousFrames(session_ptr);
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        let mut frames = self.buffered_frames.lock().unwrap();
        if let Some(frame) = frames.pop_front() {
            Ok(frame)
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        if let Some(session) = &self.session {
            unsafe {
                let session_ptr: *mut VTDecompressionSession = &**session as *const _ as *mut _;
                let _ = VTDecompressionSessionFinishDelayedFrames(session_ptr);
                let _ = VTDecompressionSessionWaitForAsynchronousFrames(session_ptr);
            }
        }
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        if let Some(session) = self.session.take() {
            unsafe {
                let session_ptr: *const VTDecompressionSession = &*session;
                VTDecompressionSessionInvalidate(session_ptr as *mut _);
            }
        }
        self.callback_context = None;
        self.format_description = None;
        self.buffered_frames.lock().unwrap().clear();
        self.flushed = false;

        if !self.stream_info.extra_data.is_empty() {
            self.initialize_session()?;
        }

        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

// External C functions from VideoToolbox and CoreMedia.
// Multiple #[link] attributes are intentional — each declares a separate
// framework dependency. clippy::duplicated_attributes is a false positive here.
#[allow(clippy::duplicated_attributes)]
#[link(name = "VideoToolbox", kind = "framework")]
#[link(name = "CoreMedia", kind = "framework")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: *const std::ffi::c_void,
        parameterSetCount: usize,
        parameterSetPointers: *const *const u8,
        parameterSetSizes: *const usize,
        nalUnitHeaderLength: i32,
        formatDescriptionOut: *mut *mut CMFormatDescription,
    ) -> i32;

    fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        allocator: *const std::ffi::c_void,
        parameterSetCount: usize,
        parameterSetPointers: *const *const u8,
        parameterSetSizes: *const usize,
        nalUnitHeaderLength: i32,
        extensions: *const std::ffi::c_void,
        formatDescriptionOut: *mut *mut CMFormatDescription,
    ) -> i32;

    fn VTDecompressionSessionCreate(
        allocator: *const std::ffi::c_void,
        videoFormatDescription: *const CMFormatDescription,
        videoDecoderSpecification: *const std::ffi::c_void,
        destinationImageBufferAttributes: *const std::ffi::c_void,
        outputCallback: *const std::ffi::c_void,
        decompressionSessionOut: *mut *mut VTDecompressionSession,
    ) -> i32;

    fn VTDecompressionSessionDecodeFrame(
        session: *mut VTDecompressionSession,
        sampleBuffer: *mut CMSampleBuffer,
        decodeFlags: u32,
        sourceFrameRefCon: *mut std::ffi::c_void,
        infoFlagsOut: *mut u32,
    ) -> i32;

    fn VTDecompressionSessionWaitForAsynchronousFrames(
        session: *mut VTDecompressionSession,
    ) -> i32;

    fn VTDecompressionSessionFinishDelayedFrames(
        session: *mut VTDecompressionSession,
    ) -> i32;

    fn VTDecompressionSessionInvalidate(
        session: *mut VTDecompressionSession,
    );

    fn CMBlockBufferCreateWithMemoryBlock(
        allocator: *const std::ffi::c_void,
        memoryBlock: *mut std::ffi::c_void,
        blockLength: usize,
        blockAllocator: *const std::ffi::c_void,
        customBlockSource: *const std::ffi::c_void,
        offsetToData: usize,
        dataLength: usize,
        flags: u32,
        blockBufferOut: *mut *mut CMBlockBuffer,
    ) -> i32;

    fn CMBlockBufferReplaceDataBytes(
        sourceBytes: *const std::ffi::c_void,
        destinationBuffer: *mut CMBlockBuffer,
        offsetIntoDestination: usize,
        dataLength: usize,
    ) -> i32;

    fn CMSampleBufferCreateReady(
        allocator: *const std::ffi::c_void,
        dataBuffer: *mut CMBlockBuffer,
        formatDescription: *mut CMFormatDescription,
        numSamples: i32,
        numSampleTimingEntries: i32,
        sampleTimingArray: *const CMSampleTimingInfo,
        numSampleSizeEntries: i32,
        sampleSizeArray: *const usize,
        sampleBufferOut: *mut *mut CMSampleBuffer,
    ) -> i32;

    fn CVPixelBufferLockBaseAddress(
        pixelBuffer: *mut CVPixelBuffer,
        lockFlags: u64,
    ) -> i32;

    fn CVPixelBufferUnlockBaseAddress(
        pixelBuffer: *mut CVPixelBuffer,
        unlockFlags: u64,
    ) -> i32;

    fn CVPixelBufferGetWidth(pixelBuffer: *mut CVPixelBuffer) -> usize;
    fn CVPixelBufferGetHeight(pixelBuffer: *mut CVPixelBuffer) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixelBuffer: *mut CVPixelBuffer) -> u32;
    fn CVPixelBufferGetBaseAddressOfPlane(pixelBuffer: *mut CVPixelBuffer, planeIndex: usize) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer: *mut CVPixelBuffer, planeIndex: usize) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::VideoStreamParams;

    #[test]
    fn test_videotoolbox_decoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        // Without extra_data, decoder should still create but session won't initialize
        let decoder = VideoToolboxH264Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_videotoolbox_decoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = VideoToolboxH264Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_parse_nal_length_size() {
        // Example AVCDecoderConfigurationRecord with 4-byte NAL length
        let avcc_data = [
            0x01, // configurationVersion
            0x64, // AVCProfileIndication (High profile)
            0x00, // profile_compatibility
            0x1F, // AVCLevelIndication (3.1)
            0xFF, // reserved + lengthSizeMinusOne (3 = 4-byte lengths)
            0xE1, // reserved + numOfSequenceParameterSets (1)
            0x00, 0x07, // SPS length
        ];

        let result = parse_nal_length_size(&avcc_data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 4);
    }
}
