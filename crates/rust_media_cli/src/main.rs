//! rust_media CLI - Command-line media processing tool
//!
//! A Rust-based media conversion and processing tool (FFmpeg equivalent)

use clap::{Parser, Subcommand, ValueEnum};
use rust_media_filter::audio::{AudioResampler, VolumeFilter};
use rust_media_filter::video::ssim::{calculate_frame_ssim, ssim_to_db};
use rust_media_filter::video::{CropFilter, ScaleFilter};
use rust_media_filter::{Filter, FilterGraph};
use rust_media::{
    AudioStreamParams, Decoder, Demuxer, Encoder, Frame, FrameReorderBuffer, MediaType, Muxer,
    Packet, PixelFormat, SampleFormat, StreamInfo, StreamParams, VideoStreamParams,
};
use rust_media_format::detect::ContainerFormat;
use rust_media_format::mp4::{Mp4Demuxer, Mp4Muxer};
use rust_media_format::wav::{WavDemuxer, WavMuxer};
use rust_media_format::webm::{WebmDemuxer, WebmMuxer};
use serde::Serialize;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rust_media")]
#[command(author, version, about = "A Rust-based media processing framework", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
// `Transform` carries many user-facing options (clap arg fields), which
// makes it legitimately larger than `Info`. Boxing the fields would push
// indirection through every match arm for no real gain.
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Analyze media files and display information (similar to ffprobe)
    Info {
        /// Input media file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Show packet information
        #[arg(short = 'p', long)]
        show_packets: bool,

        /// Show frame information (requires decoding)
        #[arg(short = 'f', long)]
        show_frames: bool,

        /// Limit number of packets/frames to show
        #[arg(short = 'n', long, default_value = "0")]
        count: usize,

        /// Select stream by index (default: all streams)
        #[arg(short = 's', long)]
        stream: Option<usize>,

        /// Output format
        #[arg(short = 'o', long, value_enum, default_value = "text")]
        output_format: OutputFormat,
    },

    /// Transform (transcode) media files (similar to ffmpeg)
    Transform {
        /// Input media file(s) - use multiple -i flags for multi-input filters (e.g., ssim)
        #[arg(short = 'i', long = "input", required = true, num_args = 1)]
        inputs: Vec<PathBuf>,

        /// Output media file
        #[arg(value_name = "OUTPUT")]
        output: PathBuf,

        /// Video codec (vp8, vp9, h264, copy, none)
        #[arg(short = 'v', long, default_value = "copy")]
        video_codec: String,

        /// Audio codec (opus, aac, pcm, copy, none)
        #[arg(short = 'a', long, default_value = "copy")]
        audio_codec: String,

        /// Video bitrate in kbps (e.g., 1000 for 1 Mbps)
        #[arg(long, default_value = "1000")]
        video_bitrate: u64,

        /// Audio bitrate in kbps (e.g., 128 for 128 kbps)
        #[arg(long, default_value = "128")]
        audio_bitrate: u64,

        // ----- Codec tuning (ffmpeg-compatible names) -----
        // These apply to AV1 (rav1e) today. Other encoders silently ignore
        // them until their builders gain matching options.
        /// GOP size — maximum frames between keyframes (ffmpeg -g).
        ///
        /// AV1: defaults to 240. Larger values improve compression at the
        /// cost of seek granularity.
        #[arg(short = 'g', long = "gop-size", visible_alias = "g")]
        gop_size: Option<u64>,

        /// Minimum frames between keyframes (ffmpeg -keyint_min).
        ///
        /// AV1: defaults to 12.
        #[arg(long = "keyint-min", visible_alias = "keyint_min")]
        keyint_min: Option<u64>,

        /// Constant-quality quantizer for AV1 (ffmpeg librav1e -qp).
        ///
        /// Range 0-255 — lower is higher quality. Setting `--qp` puts the
        /// encoder into constant-quality mode and overrides --video-bitrate.
        /// rav1e's default is 100.
        #[arg(long = "qp")]
        qp: Option<u8>,

        /// Speed preset for AV1 (ffmpeg librav1e -speed).
        ///
        /// Range 0-10 — lower is slower and higher quality. rav1e's default
        /// is 6 (balanced). Use 8-10 for fast previews.
        #[arg(long = "speed")]
        speed: Option<u8>,

        /// Number of tile columns (ffmpeg -tile-columns). Must be a power of two.
        #[arg(long = "tile-columns", visible_alias = "tile_columns")]
        tile_columns: Option<usize>,

        /// Number of tile rows (ffmpeg -tile-rows). Must be a power of two.
        #[arg(long = "tile-rows", visible_alias = "tile_rows")]
        tile_rows: Option<usize>,

        /// Total tile count for AV1 (ffmpeg librav1e -tiles).
        ///
        /// When set, overrides --tile-columns and --tile-rows.
        #[arg(long = "tiles")]
        tiles: Option<usize>,

        /// Select video stream by index (default: first video stream)
        #[arg(long)]
        video_stream: Option<usize>,

        /// Select audio stream by index (default: first audio stream)
        #[arg(long)]
        audio_stream: Option<usize>,

        /// Disable video output
        #[arg(long)]
        no_video: bool,

        /// Disable audio output
        #[arg(long)]
        no_audio: bool,

        /// Show progress during transcoding
        #[arg(long)]
        progress: bool,

        /// Video filter graph (e.g., "ssim")
        ///
        /// Filters are specified as: filter_name=param1=value1:param2=value2
        /// Multiple filters can be chained with commas: filter1,filter2
        ///
        /// Available filters:
        ///   ssim - Compute SSIM between two video inputs
        ///     Input 0 (-i first) is the reference, Input 1 (-i second) is the distorted
        ///     Requires two -i inputs
        ///     Parameters:
        ///       stats_file=<file> - Output stats to file (optional)
        ///       print_per_frame   - Print per-frame SSIM values (optional)
        #[arg(long = "vf", visible_alias = "video-filter")]
        video_filter: Option<String>,

        /// Audio filter graph (e.g., "aresample=48000")
        ///
        /// Filters are specified as: filter_name=param1=value1:param2=value2
        /// Multiple filters can be chained with commas: filter1,filter2
        ///
        /// Available filters:
        ///   aresample=<sample_rate> - Resample audio to target sample rate
        ///     Example: aresample=48000
        #[arg(long = "af", visible_alias = "audio-filter")]
        audio_filter: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    /// Human-readable text output
    Text,
    /// JSON output
    Json,
}

// ============================================================================
// Serializable data structures for JSON output
// ============================================================================

#[derive(Serialize)]
struct MediaInfo {
    format: FormatInfo,
    streams: Vec<StreamInfoJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    packets: Vec<PacketInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    frames: Vec<FrameInfo>,
}

#[derive(Serialize)]
struct FormatInfo {
    filename: String,
    format_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bitrate: Option<u64>,
    nb_streams: usize,
}

#[derive(Serialize)]
struct StreamInfoJson {
    index: usize,
    media_type: String,
    codec: String,
    time_base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bitrate: Option<u64>,
    #[serde(flatten)]
    params: StreamParamsJson,
}

#[derive(Serialize)]
#[serde(untagged)]
enum StreamParamsJson {
    Video {
        width: usize,
        height: usize,
        pixel_format: String,
        frame_rate: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        color_space: Option<String>,
        bit_depth: u8,
    },
    Audio {
        sample_rate: u32,
        channels: usize,
        sample_format: String,
    },
    Unknown {},
}

#[derive(Serialize)]
struct PacketInfo {
    packet_index: usize,
    stream_index: usize,
    media_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pts_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dts_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<i64>,
    size: usize,
    is_keyframe: bool,
}

#[derive(Serialize)]
struct FrameInfo {
    frame_index: usize,
    stream_index: usize,
    media_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pts_time: Option<String>,
    is_keyframe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pict_type: Option<String>,
    #[serde(flatten)]
    params: FrameParamsJson,
}

#[derive(Serialize)]
#[serde(untagged)]
enum FrameParamsJson {
    Video {
        width: usize,
        height: usize,
        pixel_format: String,
    },
    Audio {
        nb_samples: usize,
        channels: usize,
        sample_format: String,
    },
    Unknown {},
}

/// Parsed video filter (alias for filter crate type, used in SSIM context)

// ============================================================================
// SSIM data structures (CLI-specific orchestration, uses rust_media_filter for computation)
// ============================================================================

#[derive(Serialize)]
struct SsimResult {
    reference: String,
    distorted: String,
    width: usize,
    height: usize,
    frame_count: usize,
    ssim_y: f64,
    ssim_u: f64,
    ssim_v: f64,
    ssim_avg: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    frames: Vec<SsimFrameResult>,
}

#[derive(Serialize, Clone)]
struct SsimFrameResult {
    frame_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pts: Option<i64>,
    ssim_y: f64,
    ssim_u: f64,
    ssim_v: f64,
    ssim_avg: f64,
}

/// SSIM filter context - holds state during filtering
///
/// Stream 0 (main input) = reference, Stream 1 (second input) = distorted.
/// The filter internally decodes the distorted input and compares against
/// reference frames received via `process_frame`.
struct SsimFilterContext {
    distorted_path: PathBuf,
    stats_file: Option<PathBuf>,
    print_per_frame: bool,
    // Runtime state - decodes the distorted (stream 1) input
    dist_demuxer: Option<Box<dyn Demuxer>>,
    dist_decoder: Option<Box<dyn DecoderWrapper>>,
    dist_stream_idx: usize,
    dist_frames: std::collections::VecDeque<Frame>,
    dist_eof: bool,
    // Results
    frame_results: Vec<SsimFrameResult>,
    total_ssim_y: f64,
    total_ssim_u: f64,
    total_ssim_v: f64,
    total_ssim_avg: f64,
    frame_count: usize,
    width: usize,
    height: usize,
}

impl SsimFilterContext {
    fn new(distorted_path: &std::path::Path, filter: &Filter) -> Result<Self, Box<dyn std::error::Error>> {
        let stats_file = filter.get_param("stats_file").map(PathBuf::from);
        let print_per_frame = filter.has_flag("print_per_frame");

        Ok(SsimFilterContext {
            distorted_path: distorted_path.to_path_buf(),
            stats_file,
            print_per_frame,
            dist_demuxer: None,
            dist_decoder: None,
            dist_stream_idx: 0,
            dist_frames: std::collections::VecDeque::new(),
            dist_eof: false,
            frame_results: Vec::new(),
            total_ssim_y: 0.0,
            total_ssim_u: 0.0,
            total_ssim_v: 0.0,
            total_ssim_avg: 0.0,
            frame_count: 0,
            width: 0,
            height: 0,
        })
    }

    fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Open distorted (stream 1) demuxer
        let dist_demuxer = open_demuxer_for_ssim(&self.distorted_path)?;
        let dist_streams = dist_demuxer.streams()?;

        // Find video stream
        let dist_video = find_video_stream(&dist_streams, None)
            .ok_or("No video stream found in distorted file")?;

        self.dist_stream_idx = dist_video.index;

        // Get dimensions
        if let StreamParams::Video(params) = &dist_video.params {
            self.width = params.width;
            self.height = params.height;
        }

        // Create decoder
        let dist_decoder = create_decoder_for_stream(dist_video)?;

        self.dist_demuxer = Some(dist_demuxer);
        self.dist_decoder = Some(dist_decoder);

        Ok(())
    }

    fn process_frame(&mut self, reference_frame: &Frame) -> Result<(), Box<dyn std::error::Error>> {
        // Initialize on first frame if needed
        if self.dist_demuxer.is_none() {
            self.initialize()?;
        }

        // Get distorted frame (stream 1)
        let dist_frame = self.get_next_distorted_frame()?;

        // Calculate SSIM (reference vs distorted)
        match calculate_frame_ssim(reference_frame, &dist_frame) {
            Ok((ssim_y, ssim_u, ssim_v, ssim_avg)) => {
                self.total_ssim_y += ssim_y;
                self.total_ssim_u += ssim_u;
                self.total_ssim_v += ssim_v;
                self.total_ssim_avg += ssim_avg;

                let frame_result = SsimFrameResult {
                    frame_index: self.frame_count,
                    pts: reference_frame.pts(),
                    ssim_y,
                    ssim_u,
                    ssim_v,
                    ssim_avg,
                };

                if self.print_per_frame {
                    eprintln!(
                        "[SSIM] Frame {:>5}: Y={:.6} U={:.6} V={:.6} All={:.6} ({:.2} dB)",
                        self.frame_count, ssim_y, ssim_u, ssim_v, ssim_avg, ssim_to_db(ssim_avg)
                    );
                }

                self.frame_results.push(frame_result);
                self.frame_count += 1;
            }
            Err(e) => {
                eprintln!("Warning: Failed to calculate SSIM for frame {}: {}", self.frame_count, e);
            }
        }

        Ok(())
    }

    fn get_next_distorted_frame(&mut self) -> Result<Frame, Box<dyn std::error::Error>> {
        // Try to get a buffered frame first
        if let Some(frame) = self.dist_frames.pop_front() {
            return Ok(frame);
        }

        let demuxer = self.dist_demuxer.as_mut().ok_or("Distorted demuxer not initialized")?;
        let decoder = self.dist_decoder.as_mut().ok_or("Distorted decoder not initialized")?;

        // Decode frames until we get one
        loop {
            if self.dist_eof {
                return Err("Distorted video ended before reference".into());
            }

            // Try to receive a frame
            if let Ok(frame) = decoder.receive_frame() {
                return Ok(frame);
            }

            // Read packets until we get a frame
            match demuxer.read_packet() {
                Ok(packet) => {
                    if packet.stream_index() != self.dist_stream_idx {
                        continue;
                    }
                    let _ = decoder.send_packet(&packet);
                }
                Err(_) => {
                    self.dist_eof = true;
                    let _ = decoder.flush();

                    // Try one more receive after flush
                    match decoder.receive_frame() {
                        Ok(frame) => return Ok(frame),
                        Err(_) => return Err("Distorted video ended before reference".into()),
                    }
                }
            }
        }
    }

    fn finalize(&self) -> SsimResult {
        let avg_ssim_y = if self.frame_count > 0 { self.total_ssim_y / self.frame_count as f64 } else { 0.0 };
        let avg_ssim_u = if self.frame_count > 0 { self.total_ssim_u / self.frame_count as f64 } else { 0.0 };
        let avg_ssim_v = if self.frame_count > 0 { self.total_ssim_v / self.frame_count as f64 } else { 0.0 };
        let avg_ssim_avg = if self.frame_count > 0 { self.total_ssim_avg / self.frame_count as f64 } else { 0.0 };

        SsimResult {
            reference: "input[0]".to_string(),
            distorted: self.distorted_path.display().to_string(),
            width: self.width,
            height: self.height,
            frame_count: self.frame_count,
            ssim_y: avg_ssim_y,
            ssim_u: avg_ssim_u,
            ssim_v: avg_ssim_v,
            ssim_avg: avg_ssim_avg,
            frames: self.frame_results.clone(),
        }
    }

    fn print_summary(&self) {
        let result = self.finalize();
        eprintln!();
        eprintln!("SSIM Summary:");
        eprintln!("  Reference: {}", result.reference);
        eprintln!("  Distorted: {}", result.distorted);
        eprintln!("  Frames:    {}", result.frame_count);
        eprintln!("  Y:   {:.6} ({:.2} dB)", result.ssim_y, ssim_to_db(result.ssim_y));
        eprintln!("  U:   {:.6} ({:.2} dB)", result.ssim_u, ssim_to_db(result.ssim_u));
        eprintln!("  V:   {:.6} ({:.2} dB)", result.ssim_v, ssim_to_db(result.ssim_v));
        eprintln!("  All: {:.6} ({:.2} dB)", result.ssim_avg, ssim_to_db(result.ssim_avg));
    }

    fn write_stats_file(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(stats_path) = &self.stats_file {
            let result = self.finalize();
            let json = serde_json::to_string_pretty(&result)?;
            std::fs::write(stats_path, json)?;
            eprintln!("SSIM stats written to: {}", stats_path.display());
        }
        Ok(())
    }
}

// ============================================================================
// Main entry point
// ============================================================================

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Info {
            input,
            show_packets,
            show_frames,
            count,
            stream,
            output_format,
        } => {
            if let Err(e) =
                run_info(&input, show_packets, show_frames, count, stream, output_format)
            {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Transform {
            inputs,
            output,
            video_codec,
            audio_codec,
            video_bitrate,
            audio_bitrate,
            gop_size,
            keyint_min,
            qp,
            speed,
            tile_columns,
            tile_rows,
            tiles,
            video_stream,
            audio_stream,
            no_video,
            no_audio,
            progress,
            video_filter,
            audio_filter,
        } => {
            let encoder_opts = EncoderOptions {
                gop_size,
                keyint_min,
                qp,
                speed,
                tile_columns,
                tile_rows,
                tiles,
            };
            if let Err(e) = run_transform(
                &inputs,
                &output,
                &video_codec,
                &audio_codec,
                video_bitrate * 1000, // Convert kbps to bps
                audio_bitrate * 1000, // Convert kbps to bps
                video_stream,
                audio_stream,
                no_video,
                no_audio,
                progress,
                video_filter.as_deref(),
                audio_filter.as_deref(),
                &encoder_opts,
            ) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

// ============================================================================
// Transform (transcode) implementation
// ============================================================================

/// Per-codec encoder tuning options gathered from CLI flags.
///
/// The flag names mirror FFmpeg's so users coming from `ffmpeg` can carry
/// muscle memory across (`-g`, `-keyint_min`, `-qp`, `-speed`, `-tile-columns`,
/// `-tile-rows`, `-tiles`). Every field is `Option<_>` — `None` means "fall
/// back to the codec's built-in default".
///
/// Each option is applied to the encoder builder if the codec supports it,
/// and silently ignored otherwise. Today only AV1 (rav1e) consumes any of
/// these; VP8/VP9/H.264 ignore everything until their builders gain matching
/// knobs.
#[derive(Debug, Default, Clone)]
struct EncoderOptions {
    /// `-g` — max keyframe interval in frames (rav1e `max_key_frame_interval`).
    gop_size: Option<u64>,
    /// `-keyint_min` — min keyframe interval in frames.
    keyint_min: Option<u64>,
    /// `-qp` — constant-quality quantizer (rav1e range 0..=255).
    qp: Option<u8>,
    /// `-speed` — speed preset (rav1e range 0..=10).
    speed: Option<u8>,
    /// `-tile-columns` — horizontal tile count.
    tile_columns: Option<usize>,
    /// `-tile-rows` — vertical tile count.
    tile_rows: Option<usize>,
    /// `-tiles` — total tile count (rav1e). Overrides cols/rows when set.
    tiles: Option<usize>,
}

#[allow(clippy::too_many_arguments)]
fn run_transform(
    inputs: &[PathBuf],
    output: &std::path::Path,
    video_codec: &str,
    audio_codec: &str,
    video_bitrate: u64,
    audio_bitrate: u64,
    video_stream_idx: Option<usize>,
    audio_stream_idx: Option<usize>,
    no_video: bool,
    no_audio: bool,
    progress: bool,
    video_filter: Option<&str>,
    audio_filter: Option<&str>,
    encoder_opts: &EncoderOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let input = inputs.first().ok_or("At least one input file is required")?;
    let input_filename = input.to_string_lossy().to_string();
    let output_filename = output.to_string_lossy().to_string();

    // Detect formats
    let (input_demuxer, input_format) = open_demuxer(input)?;
    let input_ext = input_format.name().to_string();

    let output_ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    println!("Input:  {} ({})", input_filename, input_ext);
    println!("Output: {} ({})", output_filename, output_ext);

    // Get streams and duration
    let container = input_demuxer.container_info()?;
    let streams = input_demuxer.streams()?;
    let duration = container.duration;
    drop(input_demuxer); // Close; will reopen in transcode pipeline

    // Find video and audio streams
    let video_stream = if no_video {
        None
    } else {
        video_stream_idx
            .and_then(|idx| streams.get(idx).cloned())
            .or_else(|| {
                streams
                    .iter()
                    .find(|s| s.media_type == MediaType::Video)
                    .cloned()
            })
    };

    let mut audio_stream = if no_audio {
        None
    } else {
        audio_stream_idx
            .and_then(|idx| streams.get(idx).cloned())
            .or_else(|| {
                streams
                    .iter()
                    .find(|s| s.media_type == MediaType::Audio)
                    .cloned()
            })
    };

    // Print stream info
    if let Some(ref vs) = video_stream {
        println!(
            "Video:  Stream #{} ({}) -> {}",
            vs.index,
            vs.codec,
            if video_codec == "copy" {
                vs.codec.clone()
            } else if video_codec == "none" {
                "disabled".to_string()
            } else {
                video_codec.to_string()
            }
        );
    }

    if let Some(ref aus) = audio_stream {
        println!(
            "Audio:  Stream #{} ({}) -> {}",
            aus.index,
            aus.codec,
            if audio_codec == "copy" {
                aus.codec.clone()
            } else if audio_codec == "none" {
                "disabled".to_string()
            } else {
                audio_codec.to_string()
            }
        );
    }

    println!();

    // Parse video filter graph and create filter context
    let mut ssim_filter: Option<SsimFilterContext> = None;
    let mut scale_filter: Option<ScaleFilter> = None;
    let mut crop_filter: Option<CropFilter> = None;

    if let Some(filter_str) = video_filter {
        let filter_graph = FilterGraph::parse(filter_str)
            .map_err(|e| format!("Failed to parse filter graph: {}", e))?;

        // Process filters
        for filter in &filter_graph.filters {
            match filter.name.as_str() {
                "ssim" => {
                    if video_codec == "copy" {
                        return Err("SSIM filter requires video transcoding (cannot use with -v copy)".into());
                    }
                    if inputs.len() < 2 {
                        return Err("SSIM filter requires two inputs: -i <reference> -i <distorted>".into());
                    }
                    println!("Filter: SSIM comparison (reference={}, distorted={})",
                             inputs[0].display(), inputs[1].display());
                    ssim_filter = Some(SsimFilterContext::new(&inputs[1], filter)?);
                }
                "scale" => {
                    if video_codec == "copy" {
                        return Err("scale filter requires video transcoding (cannot use with -v copy)".into());
                    }
                    // Accept both `scale=w=640:h=480` and `scale=640:480` (positional).
                    // Also accept ffmpeg-style aspect-ratio markers: -1 (preserve)
                    // and -2 (preserve, round to even).
                    let (parsed_w, parsed_h) = parse_scale_dimensions(filter)
                        .map_err(|e| format!("scale filter: {}", e))?;

                    // Resolve aspect-ratio markers using the source video dimensions.
                    // The source dimensions reflect any earlier filter in the chain
                    // (e.g., crop) since we've been updating them as we parse.
                    let (src_w, src_h) = video_stream
                        .as_ref()
                        .and_then(|vs| {
                            if let StreamParams::Video(ref vp) = vs.params {
                                // If a crop filter was already parsed, use its target
                                // dimensions as the source for scale (chain order is
                                // crop → scale).
                                if let Some(cf) = crop_filter.as_ref() {
                                    Some((cf.width(), cf.height()))
                                } else {
                                    Some((vp.width, vp.height))
                                }
                            } else {
                                None
                            }
                        })
                        .ok_or("scale filter: no video stream found to resolve dimensions")?;

                    let (w, h) = resolve_scale_dimensions(parsed_w, parsed_h, src_w, src_h)
                        .map_err(|e| format!("scale filter: {}", e))?;
                    if (parsed_w, parsed_h) != (w as i32, h as i32) {
                        println!(
                            "Filter: scale ({}x{} → {}x{}, bilinear)",
                            src_w, src_h, w, h
                        );
                    } else {
                        println!("Filter: scale ({}x{}, bilinear)", w, h);
                    }
                    scale_filter = Some(
                        ScaleFilter::new(w, h)
                            .map_err(|e| format!("scale filter: {}", e))?,
                    );
                }
                "crop" => {
                    if video_codec == "copy" {
                        return Err("crop filter requires video transcoding (cannot use with -v copy)".into());
                    }
                    // Accept `crop=W:H:X:Y`, `crop=W:H` (centered), or
                    // `crop=w=W:h=H:x=X:y=Y`
                    let (w, h, x, y) = parse_crop_params(filter)
                        .map_err(|e| format!("crop filter: {}", e))?;
                    match (x, y) {
                        (Some(x), Some(y)) => println!("Filter: crop ({}x{} at {},{})", w, h, x, y),
                        _ => println!("Filter: crop ({}x{}, centered)", w, h),
                    }
                    crop_filter = Some(
                        CropFilter::new(w, h, x, y)
                            .map_err(|e| format!("crop filter: {}", e))?,
                    );
                }
                _ => {
                    return Err(format!("Unknown video filter: {}", filter.name).into());
                }
            }
        }
    }

    // Parse audio filter graph
    let mut audio_resampler: Option<AudioResampler> = None;
    let mut volume_filter: Option<VolumeFilter> = None;

    if let Some(filter_str) = audio_filter {
        let filter_graph = FilterGraph::parse(filter_str)
            .map_err(|e| format!("Failed to parse audio filter graph: {}", e))?;

        for filter in &filter_graph.filters {
            match filter.name.as_str() {
                "volume" => {
                    // volume=2.0 or volume=0.5 or volume=6dB
                    let val_str = filter
                        .get_param("volume")
                        .or_else(|| {
                            filter.params.keys()
                                .find(|k| k.as_str() != "true" && k.as_str() != "1")
                                .map(|k| k.as_str())
                        })
                        .ok_or("volume filter requires a value (e.g., volume=0.5 or volume=6dB)")?;

                    let vf = if let Some(db_str) = val_str.strip_suffix("dB") {
                        let db: f64 = db_str
                            .parse()
                            .map_err(|_| format!("Invalid dB value: {}", val_str))?;
                        println!("Filter: volume ({:.1} dB, {:.3}x)", db, 10.0f64.powf(db / 20.0));
                        VolumeFilter::from_db(db)
                    } else {
                        let gain: f64 = val_str
                            .parse()
                            .map_err(|_| format!("Invalid volume value: {}", val_str))?;
                        println!("Filter: volume ({:.3}x, {:.1} dB)", gain, 20.0 * gain.max(f64::EPSILON).log10());
                        VolumeFilter::new(gain)
                    };
                    volume_filter = Some(vf);
                }
                "aresample" => {
                    // aresample=48000 or aresample=sample_rate=48000
                    let rate_str = filter
                        .get_param("sample_rate")
                        .or_else(|| {
                            // Handle positional arg: "aresample=48000" parses as key="48000" value="true"
                            filter.params.keys()
                                .find(|k| k.parse::<u32>().is_ok())
                                .map(|k| k.as_str())
                        })
                        .ok_or("aresample filter requires a sample rate (e.g., aresample=48000)")?;
                    let target_rate: u32 = rate_str
                        .parse()
                        .map_err(|_| format!("Invalid sample rate: {}", rate_str))?;
                    println!("Filter: aresample (target {} Hz)", target_rate);
                    audio_resampler = Some(AudioResampler::new(target_rate));

                    // Update audio stream sample rate for encoder
                    if let Some(ref mut aus) = audio_stream {
                        if let StreamParams::Audio(ref mut ap) = aus.params {
                            ap.sample_rate = target_rate;
                        }
                        aus.time_base = (1, target_rate);
                    }
                }
                _ => {
                    return Err(format!("Unknown audio filter: {}", filter.name).into());
                }
            }
        }
    }

    // Auto-resample if the encoder doesn't support the input sample rate
    if audio_resampler.is_none() && audio_codec != "copy" && audio_codec != "none" {
        if let Some(ref mut aus) = audio_stream {
            if let StreamParams::Audio(ref ap) = aus.params {
                if let Some(target_rate) = nearest_supported_sample_rate(audio_codec, ap.sample_rate) {
                    eprintln!(
                        "Auto-resampling audio: {} Hz -> {} Hz (required by {} encoder)",
                        ap.sample_rate, target_rate, audio_codec
                    );
                    audio_resampler = Some(AudioResampler::new(target_rate));
                    // Update stream info so muxer and encoder get the resampled rate
                    if let StreamParams::Audio(ref mut ap) = aus.params {
                        ap.sample_rate = target_rate;
                    }
                    aus.time_base = (1, target_rate);
                }
            }
        }
    }

    // Perform the actual transcoding
    match output_ext.as_str() {
        "mp4" | "m4a" | "m4v" | "mov" => {
            transcode_to_mp4(
                &input_filename,
                &output_filename,
                video_stream,
                audio_stream,
                video_codec,
                audio_codec,
                video_bitrate,
                audio_bitrate,
                duration,
                progress,
                &mut ssim_filter,
                &scale_filter,
                &crop_filter,
                &mut audio_resampler,
                &volume_filter,
                encoder_opts,
            )?;
        }
        "webm" => {
            transcode_to_webm(
                &input_filename,
                &output_filename,
                video_stream,
                audio_stream,
                video_codec,
                audio_codec,
                video_bitrate,
                audio_bitrate,
                duration,
                progress,
                &mut ssim_filter,
                &scale_filter,
                &crop_filter,
                &mut audio_resampler,
                &volume_filter,
                encoder_opts,
            )?;
        }
        "wav" => {
            transcode_to_wav(
                &input_filename,
                &output_filename,
                audio_stream,
                audio_codec,
                duration,
                progress,
            )?;
        }
        _ => {
            return Err(format!("Unsupported output format: {}", output_ext).into());
        }
    }

    // Print SSIM summary if filter was used
    if let Some(ref filter) = ssim_filter {
        filter.print_summary();
        filter.write_stats_file()?;
    }

    println!("Transcoding complete!");
    Ok(())
}

// ============================================================================
// Transcode to MP4
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn transcode_to_mp4(
    input_filename: &str,
    output_filename: &str,
    video_stream: Option<StreamInfo>,
    audio_stream: Option<StreamInfo>,
    video_codec: &str,
    audio_codec: &str,
    video_bitrate: u64,
    audio_bitrate: u64,
    duration: Option<i64>,
    progress: bool,
    ssim_filter: &mut Option<SsimFilterContext>,
    scale_filter: &Option<ScaleFilter>,
    crop_filter: &Option<CropFilter>,
    audio_resampler: &mut Option<AudioResampler>,
    volume_filter: &Option<VolumeFilter>,
    encoder_opts: &EncoderOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_file = File::create(output_filename)?;
    let mut writer = BufWriter::new(output_file);
    let mut muxer = Mp4Muxer::new(&mut writer);

    // Track output stream indices
    let mut video_out_idx: Option<usize> = None;
    let mut audio_out_idx: Option<usize> = None;
    let mut current_out_idx = 0;

    // Add video stream to muxer
    if let Some(ref vs) = video_stream {
        if video_codec != "none" {
            let out_codec = if video_codec == "copy" {
                vs.codec.clone()
            } else {
                video_codec.to_string()
            };
            let mut out_stream = vs.clone();
            out_stream.index = current_out_idx;
            out_stream.codec = out_codec.clone();
            out_stream.bitrate = Some(video_bitrate);
            // Apply crop dimensions first, then scale (chain order matches the
            // pipeline: crop runs before scale).
            if let Some(cf) = crop_filter {
                if let StreamParams::Video(ref mut vp) = out_stream.params {
                    vp.width = cf.width();
                    vp.height = cf.height();
                }
            }
            if let Some(sf) = scale_filter {
                if let StreamParams::Video(ref mut vp) = out_stream.params {
                    vp.width = sf.target_width();
                    vp.height = sf.target_height();
                }
            }
            // For AV1 (rav1e), pre-build the encoder to extract the av1C
            // sequence header — the MP4 muxer needs it to write the av1C box.
            // The encoder pixel format follows the input bit depth (10-bit for
            // YUV420P10LE, 8-bit otherwise), matching what create_video_encoder
            // will use later.
            if video_codec != "copy" {
                if out_codec == "av1" || out_codec == "av01" {
                    let av1c = build_av1_codec_config(vs, video_bitrate, encoder_opts)?;
                    out_stream.extra_data = av1c;
                    // Reflect 10-bit in the muxer's StreamParams so info readers
                    // see the correct bit_depth field.
                    if let StreamParams::Video(ref mut vp) = out_stream.params {
                        if matches!(
                            vs.params,
                            StreamParams::Video(ref ivp) if ivp.pixel_format == PixelFormat::YUV420P10LE
                        ) {
                            vp.pixel_format = PixelFormat::YUV420P10LE;
                            vp.bit_depth = 10;
                        } else {
                            vp.pixel_format = PixelFormat::YUV420P;
                            vp.bit_depth = 8;
                        }
                    }
                } else if out_codec == "vp9" {
                    let vpcc = build_vp9_codec_config(vs, video_bitrate)?;
                    out_stream.extra_data = vpcc;
                }
                #[cfg(feature = "gpl-x264")]
                if out_codec == "h264" || out_codec == "avc" {
                    let avcc = build_h264_codec_config(vs, video_bitrate)?;
                    out_stream.extra_data = avcc;
                }
            }
            muxer.add_stream(out_stream)?;
            video_out_idx = Some(current_out_idx);
            current_out_idx += 1;
        }
    }

    // Add audio stream to muxer
    if let Some(ref aus) = audio_stream {
        if audio_codec != "none" {
            let out_codec = if audio_codec == "copy" {
                aus.codec.clone()
            } else {
                audio_codec.to_string()
            };
            let mut out_stream = aus.clone();
            out_stream.index = current_out_idx;
            out_stream.codec = out_codec.clone();
            out_stream.bitrate = Some(audio_bitrate);

            // For AAC, we need to set up the encoder and get AudioSpecificConfig
            if out_codec == "aac" && audio_codec != "copy" {
                #[cfg(feature = "fdk-aac")]
                {
                    // Create encoder to get AudioSpecificConfig
                    let encoder = rust_media_codec::FdkAacEncoder::new(out_stream.clone())?;
                    out_stream.extra_data = encoder.audio_specific_config().to_vec();
                }
            }

            muxer.add_stream(out_stream)?;
            audio_out_idx = Some(current_out_idx);
            // current_out_idx += 1;  // Not used after this
        }
    }

    muxer.write_header()?;

    // Create the transcode pipeline
    run_transcode_pipeline(
        input_filename,
        &mut muxer,
        video_stream,
        audio_stream,
        video_out_idx,
        audio_out_idx,
        video_codec,
        audio_codec,
        video_bitrate,
        audio_bitrate,
        duration,
        progress,
        ssim_filter,
        scale_filter,
        crop_filter,
        audio_resampler,
        volume_filter,
        encoder_opts,
    )?;

    muxer.write_trailer()?;
    muxer.flush()?;

    Ok(())
}

// ============================================================================
// Transcode to WebM
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn transcode_to_webm(
    input_filename: &str,
    output_filename: &str,
    video_stream: Option<StreamInfo>,
    audio_stream: Option<StreamInfo>,
    video_codec: &str,
    audio_codec: &str,
    video_bitrate: u64,
    audio_bitrate: u64,
    duration: Option<i64>,
    progress: bool,
    ssim_filter: &mut Option<SsimFilterContext>,
    scale_filter: &Option<ScaleFilter>,
    crop_filter: &Option<CropFilter>,
    audio_resampler: &mut Option<AudioResampler>,
    volume_filter: &Option<VolumeFilter>,
    encoder_opts: &EncoderOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_file = File::create(output_filename)?;
    let writer = BufWriter::new(output_file);
    let mut muxer = WebmMuxer::new(writer);

    // Track output stream indices
    let mut video_out_idx: Option<usize> = None;
    let mut audio_out_idx: Option<usize> = None;
    let mut current_out_idx = 0;

    // Add video stream to muxer
    if let Some(ref vs) = video_stream {
        if video_codec != "none" {
            let out_codec = if video_codec == "copy" {
                vs.codec.clone()
            } else {
                video_codec.to_string()
            };

            // WebM only supports VP8, VP9, AV1 for video
            if !["vp8", "vp9", "av1", "copy"].contains(&out_codec.as_str())
                && video_codec != "copy"
            {
                return Err(format!(
                    "WebM only supports VP8, VP9, AV1 video codecs, not {}",
                    out_codec
                )
                .into());
            }

            let mut out_stream = vs.clone();
            out_stream.index = current_out_idx;
            out_stream.codec = out_codec.clone();
            out_stream.bitrate = Some(video_bitrate);
            // Apply crop dimensions first, then scale (chain order matches the
            // pipeline: crop runs before scale).
            if let Some(cf) = crop_filter {
                if let StreamParams::Video(ref mut vp) = out_stream.params {
                    vp.width = cf.width();
                    vp.height = cf.height();
                }
            }
            if let Some(sf) = scale_filter {
                if let StreamParams::Video(ref mut vp) = out_stream.params {
                    vp.width = sf.target_width();
                    vp.height = sf.target_height();
                }
            }
            // Pre-build codec config records for containers that need them.
            // CodecPrivate for Matroska/WebM uses the same layout as the
            // ISOBMFF config boxes (av1C, vpcC).
            if video_codec != "copy" {
                if out_codec == "av1" || out_codec == "av01" {
                    let av1c = build_av1_codec_config(vs, video_bitrate, encoder_opts)?;
                    out_stream.extra_data = av1c;
                    if let StreamParams::Video(ref mut vp) = out_stream.params {
                        if matches!(
                            vs.params,
                            StreamParams::Video(ref ivp) if ivp.pixel_format == PixelFormat::YUV420P10LE
                        ) {
                            vp.pixel_format = PixelFormat::YUV420P10LE;
                            vp.bit_depth = 10;
                        } else {
                            vp.pixel_format = PixelFormat::YUV420P;
                            vp.bit_depth = 8;
                        }
                    }
                } else if out_codec == "vp9" {
                    let vpcc = build_vp9_codec_config(vs, video_bitrate)?;
                    out_stream.extra_data = vpcc;
                }
            }
            muxer.add_stream(out_stream)?;
            video_out_idx = Some(current_out_idx);
            current_out_idx += 1;
        }
    }

    // Add audio stream to muxer
    if let Some(ref aus) = audio_stream {
        if audio_codec != "none" {
            let out_codec = if audio_codec == "copy" {
                aus.codec.clone()
            } else {
                audio_codec.to_string()
            };

            // WebM only supports Opus and Vorbis for audio
            if !["opus", "vorbis", "copy"].contains(&out_codec.as_str()) && audio_codec != "copy" {
                return Err(format!(
                    "WebM only supports Opus and Vorbis audio codecs, not {}",
                    out_codec
                )
                .into());
            }

            let mut out_stream = aus.clone();
            out_stream.index = current_out_idx;
            out_stream.codec = out_codec;
            out_stream.bitrate = Some(audio_bitrate);
            muxer.add_stream(out_stream)?;
            audio_out_idx = Some(current_out_idx);
            // current_out_idx += 1;  // Not used after this
        }
    }

    muxer.write_header()?;

    // Create the transcode pipeline
    run_transcode_pipeline(
        input_filename,
        &mut muxer,
        video_stream,
        audio_stream,
        video_out_idx,
        audio_out_idx,
        video_codec,
        audio_codec,
        video_bitrate,
        audio_bitrate,
        duration,
        progress,
        ssim_filter,
        scale_filter,
        crop_filter,
        audio_resampler,
        volume_filter,
        encoder_opts,
    )?;

    muxer.write_trailer()?;
    muxer.flush()?;

    Ok(())
}

// ============================================================================
// Transcode to WAV
// ============================================================================

fn transcode_to_wav(
    input_filename: &str,
    output_filename: &str,
    audio_stream: Option<StreamInfo>,
    audio_codec: &str,
    duration: Option<i64>,
    progress: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let audio_stream =
        audio_stream.ok_or("No audio stream found for WAV output")?;

    if audio_codec != "pcm" && audio_codec != "copy" {
        return Err("WAV format only supports PCM audio codec".into());
    }

    // Create WAV muxer
    let output_file = File::create(output_filename)?;
    let writer = BufWriter::new(output_file);
    let mut muxer = WavMuxer::new(writer);

    // Create PCM stream for WAV output
    let mut out_stream = audio_stream.clone();
    out_stream.index = 0;
    out_stream.codec = "pcm".to_string();
    muxer.add_stream(out_stream)?;
    muxer.write_header()?;

    // Create decoder for audio
    let mut decoder = create_decoder_for_stream(&audio_stream)?;

    // Open demuxer
    let input_path = std::path::Path::new(input_filename);
    let (mut demuxer, _) = open_demuxer(input_path)?;
    process_audio_to_wav(
        &mut *demuxer,
        &mut decoder,
        &mut muxer,
        audio_stream.index,
        duration,
        progress,
        audio_stream.time_base,
    )?;

    muxer.write_trailer()?;
    muxer.flush()?;
    Ok(())
}

fn process_audio_to_wav<W: std::io::Write + std::io::Seek>(
    demuxer: &mut dyn Demuxer,
    decoder: &mut Box<dyn DecoderWrapper>,
    muxer: &mut WavMuxer<W>,
    audio_stream_idx: usize,
    _duration: Option<i64>,
    progress: bool,
    time_base: (u32, u32),
) -> Result<(), Box<dyn std::error::Error>> {
    let mut packet_count = 0;
    let mut frame_count = 0;
    let start_time = std::time::Instant::now();
    let mut progress_state = if progress {
        Some(ProgressState::new())
    } else {
        None
    };

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                if packet.stream_index() != audio_stream_idx {
                    continue;
                }

                packet_count += 1;

                if let Some(ps) = progress_state.as_mut() {
                    let pts_us = packet.pts().map(|p| {
                        if time_base.1 > 0 { p * time_base.0 as i64 * 1_000_000 / time_base.1 as i64 } else { p }
                    });
                    ps.update(pts_us);
                }

                decoder.send_packet(&packet)?;

                while let Ok(frame) = decoder.receive_frame() {
                    frame_count += 1;

                    // Get PCM data from frame and write as packet
                    if let Some(data) = frame.data().first() {
                        let pcm_packet = rust_media::Packet::new(data.to_vec(), 0, MediaType::Audio)
                            .with_pts(frame.pts().unwrap_or(0))
                            .with_duration(frame.duration().unwrap_or(0));
                        muxer.write_packet(&pcm_packet)?;
                    }
                }
            }
            Err(rust_media::Error::EndOfStream) => break,
            Err(e) => return Err(format!("Demuxer error: {:?}", e).into()),
        }
    }

    // Flush decoder
    decoder.flush()?;
    while let Ok(frame) = decoder.receive_frame() {
        if let Some(data) = frame.data().first() {
            let pcm_packet = rust_media::Packet::new(data.to_vec(), 0, MediaType::Audio)
                .with_pts(frame.pts().unwrap_or(0))
                .with_duration(frame.duration().unwrap_or(0));
            muxer.write_packet(&pcm_packet)?;
        }
    }

    if let Some(ps) = &progress_state {
        ps.finish();
    }

    eprintln!(
        "Processed {} packets, {} frames in {:.2}s",
        packet_count,
        frame_count,
        start_time.elapsed().as_secs_f64()
    );

    Ok(())
}

// ============================================================================
// Generic transcode pipeline
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn run_transcode_pipeline<M: Muxer>(
    input_filename: &str,
    muxer: &mut M,
    video_stream: Option<StreamInfo>,
    audio_stream: Option<StreamInfo>,
    video_out_idx: Option<usize>,
    audio_out_idx: Option<usize>,
    video_codec: &str,
    audio_codec: &str,
    video_bitrate: u64,
    audio_bitrate: u64,
    duration: Option<i64>,
    progress: bool,
    ssim_filter: &mut Option<SsimFilterContext>,
    scale_filter: &Option<ScaleFilter>,
    crop_filter: &Option<CropFilter>,
    audio_resampler: &mut Option<AudioResampler>,
    volume_filter: &Option<VolumeFilter>,
    encoder_opts: &EncoderOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create decoders and encoders
    let mut video_decoder: Option<Box<dyn DecoderWrapper>> = None;
    let mut video_encoder: Option<Box<dyn EncoderWrapper>> = None;
    let mut audio_decoder: Option<Box<dyn DecoderWrapper>> = None;
    let mut audio_encoder: Option<Box<dyn EncoderWrapper>> = None;

    let video_in_idx = video_stream.as_ref().map(|s| s.index);
    let audio_in_idx = audio_stream.as_ref().map(|s| s.index);

    // Setup video pipeline
    if let Some(ref vs) = video_stream {
        if video_codec != "none" && video_codec != "copy" {
            // Decoder uses original (source) dimensions from `vs`
            video_decoder = Some(create_decoder_for_stream(vs)?);

            // Encoder uses crop and/or scale dimensions if filters are active.
            // Order: crop applies first, then scale, so scale (if set) wins.
            let mut encoder_stream = vs.clone();
            if let Some(cf) = crop_filter {
                if let StreamParams::Video(ref mut vp) = encoder_stream.params {
                    vp.width = cf.width();
                    vp.height = cf.height();
                }
            }
            if let Some(sf) = scale_filter {
                if let StreamParams::Video(ref mut vp) = encoder_stream.params {
                    vp.width = sf.target_width();
                    vp.height = sf.target_height();
                }
            }
            video_encoder = Some(create_video_encoder(
                &encoder_stream,
                video_codec,
                video_bitrate,
                encoder_opts,
            )?);
        }
    }

    // Setup audio pipeline
    if let Some(ref aus) = audio_stream {
        if audio_codec != "none" && audio_codec != "copy" {
            audio_decoder = Some(create_decoder_for_stream(aus)?);
            audio_encoder = Some(create_audio_encoder(aus, audio_codec, audio_bitrate)?);
        }
    }

    // Build stream timebase lookup (index → timebase)
    let max_stream_idx = [video_in_idx, audio_in_idx]
        .iter()
        .filter_map(|x| *x)
        .max()
        .unwrap_or(0);
    let mut stream_time_bases = vec![(1u32, 1_000_000u32); max_stream_idx + 1];
    if let Some(ref vs) = video_stream {
        if vs.index < stream_time_bases.len() {
            stream_time_bases[vs.index] = vs.time_base;
        }
    }
    if let Some(ref aus) = audio_stream {
        if aus.index < stream_time_bases.len() {
            stream_time_bases[aus.index] = aus.time_base;
        }
    }

    let start_time = std::time::Instant::now();
    let mut packet_count = 0;
    let mut frame_count = 0;
    let mut progress_state = if progress {
        Some(ProgressState::new())
    } else {
        None
    };

    // Open input demuxer and process
    let input_path = std::path::Path::new(input_filename);
    let (mut demuxer, _) = open_demuxer(input_path)?;

    process_packets(
        &mut *demuxer,
        muxer,
        video_in_idx,
        audio_in_idx,
        video_out_idx,
        audio_out_idx,
        &mut video_decoder,
        &mut video_encoder,
        &mut audio_decoder,
        &mut audio_encoder,
        video_codec,
        audio_codec,
        duration,
        progress,
        &mut packet_count,
        &mut frame_count,
        &mut progress_state,
        &stream_time_bases,
        ssim_filter,
        scale_filter,
        crop_filter,
        audio_resampler,
        volume_filter,
    )?;

    if let Some(ps) = &mut progress_state {
        ps.finish();
    }

    eprintln!(
        "Processed {} packets, {} frames in {:.2}s",
        packet_count,
        frame_count,
        start_time.elapsed().as_secs_f64()
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_packets<M: Muxer>(
    demuxer: &mut dyn Demuxer,
    muxer: &mut M,
    video_in_idx: Option<usize>,
    audio_in_idx: Option<usize>,
    video_out_idx: Option<usize>,
    audio_out_idx: Option<usize>,
    video_decoder: &mut Option<Box<dyn DecoderWrapper>>,
    video_encoder: &mut Option<Box<dyn EncoderWrapper>>,
    audio_decoder: &mut Option<Box<dyn DecoderWrapper>>,
    audio_encoder: &mut Option<Box<dyn EncoderWrapper>>,
    video_codec: &str,
    audio_codec: &str,
    _duration: Option<i64>,
    _progress: bool,
    packet_count: &mut usize,
    frame_count: &mut usize,
    progress_state: &mut Option<ProgressState>,
    stream_time_bases: &[(u32, u32)],
    ssim_filter: &mut Option<SsimFilterContext>,
    scale_filter: &Option<ScaleFilter>,
    crop_filter: &Option<CropFilter>,
    audio_resampler: &mut Option<AudioResampler>,
    volume_filter: &Option<VolumeFilter>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Reorder buffer for video frames (B-frame decode order → PTS order)
    let mut reorder_buf = FrameReorderBuffer::new();
    let mut output_bytes: u64 = 0;
    // One-time notice when 10→8 bit conversion is auto-inserted
    let mut bit_depth_notice_shown = false;
    // AV1 (rav1e) is the only encoder we currently have that accepts 10-bit
    // input directly. Everything else gets auto-downconverted from
    // YUV420P10LE → YUV420P at the encoder boundary.
    let encoder_supports_10bit = matches!(video_codec, "av1" | "av01");

    /// Helper: encode a frame and write resulting packets to the muxer.
    /// Accumulates output bytes written into `output_bytes`.
    fn encode_and_mux<M: Muxer>(
        encoder: &mut Box<dyn EncoderWrapper>,
        muxer: &mut M,
        frame: &Frame,
        out_idx: usize,
        output_bytes: &mut u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        encoder.send_frame(frame)?;
        while let Ok(mut enc_packet) = encoder.receive_packet() {
            *output_bytes += enc_packet.size() as u64;
            enc_packet.set_stream_index(out_idx);
            muxer.write_packet(&enc_packet)?;
        }
        Ok(())
    }


    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                let stream_idx = packet.stream_index();
                *packet_count += 1;

                // Update progress (convert PTS to microseconds using stream timebase)
                if let Some(ps) = progress_state.as_mut() {
                    ps.total_size = output_bytes;
                    let pts_us = packet.pts().map(|p| {
                        let tb = stream_time_bases.get(stream_idx).copied().unwrap_or((1, 1_000_000));
                        if tb.1 > 0 { p * tb.0 as i64 * 1_000_000 / tb.1 as i64 } else { p }
                    });
                    ps.update(pts_us);
                }

                // Handle video packet
                if Some(stream_idx) == video_in_idx {
                    if let Some(out_idx) = video_out_idx {
                        if video_codec == "copy" {
                            // Passthrough
                            let mut out_packet = packet.clone();
                            output_bytes += out_packet.size() as u64;
                            out_packet.set_stream_index(out_idx);
                            muxer.write_packet(&out_packet)?;
                        } else if let (Some(decoder), Some(encoder)) =
                            (video_decoder.as_mut(), video_encoder.as_mut())
                        {
                            // Transcode: decode → reorder → encode
                            decoder.send_packet(&packet)?;
                            while let Ok(frame) = decoder.receive_frame() {
                                *frame_count += 1;

                                // Process SSIM filter if enabled
                                if let Some(filter) = ssim_filter.as_mut() {
                                    if let Err(e) = filter.process_frame(&frame) {
                                        eprintln!("Warning: SSIM filter error: {}", e);
                                    }
                                }

                                // Auto-convert 10-bit → 8-bit if the target
                                // encoder doesn't accept 10-bit input. AV1
                                // (rav1e) handles 10-bit natively, so we leave
                                // YUV420P10LE frames untouched in that case.
                                let frame = if !encoder_supports_10bit
                                    && matches!(
                                        frame.video_params().map(|p| p.format),
                                        Some(rust_media::PixelFormat::YUV420P10LE)
                                    )
                                {
                                    if !bit_depth_notice_shown {
                                        eprintln!(
                                            "Auto-converting video: 10-bit (YUV420P10LE) -> 8-bit (YUV420P)"
                                        );
                                        bit_depth_notice_shown = true;
                                    }
                                    rust_media_filter::video::yuv420p10le_to_yuv420p(&frame)?
                                } else {
                                    frame
                                };

                                // Apply crop filter (runs before scale)
                                let frame = if let Some(cf) = crop_filter {
                                    cf.process(&frame)?
                                } else {
                                    frame
                                };

                                // Apply scale filter if enabled
                                let frame = if let Some(sf) = scale_filter {
                                    sf.process(&frame)?
                                } else {
                                    frame
                                };

                                // Push into reorder buffer; encode frames that are ready
                                reorder_buf.push(frame);
                                while let Some(ordered_frame) = reorder_buf.pop_ready() {
                                    encode_and_mux(encoder, muxer, &ordered_frame, out_idx, &mut output_bytes)?;
                                }
                            }
                        }
                    }
                }

                // Handle audio packet
                if Some(stream_idx) == audio_in_idx {
                    if let Some(out_idx) = audio_out_idx {
                        if audio_codec == "copy" {
                            // Passthrough
                            let mut out_packet = packet.clone();
                            output_bytes += out_packet.size() as u64;
                            out_packet.set_stream_index(out_idx);
                            muxer.write_packet(&out_packet)?;
                        } else if let (Some(decoder), Some(encoder)) =
                            (audio_decoder.as_mut(), audio_encoder.as_mut())
                        {
                            // Transcode (with optional resampling)
                            decoder.send_packet(&packet)?;
                            while let Ok(frame) = decoder.receive_frame() {
                                *frame_count += 1;
                                let frame = if let Some(resampler) = audio_resampler {
                                    resampler.resample(&frame)?
                                } else {
                                    frame
                                };
                                let frame = if let Some(vf) = volume_filter {
                                    vf.process(&frame)?
                                } else {
                                    frame
                                };
                                encoder.send_frame(&frame)?;
                                while let Ok(mut enc_packet) = encoder.receive_packet() {
                                    output_bytes += enc_packet.size() as u64;
                                    enc_packet.set_stream_index(out_idx);
                                    muxer.write_packet(&enc_packet)?;
                                }
                            }
                        }
                    }
                }
            }
            Err(rust_media::Error::EndOfStream) => break,
            Err(e) => return Err(format!("Demuxer error: {:?}", e).into()),
        }
    }

    // Flush video decoder → reorder buffer → encoder
    if let (Some(decoder), Some(encoder)) = (video_decoder.as_mut(), video_encoder.as_mut()) {
        decoder.flush()?;
        while let Ok(frame) = decoder.receive_frame() {
            // Process SSIM filter if enabled
            if let Some(filter) = ssim_filter.as_mut() {
                if let Err(e) = filter.process_frame(&frame) {
                    eprintln!("Warning: SSIM filter error during flush: {}", e);
                }
            }

            // Auto-convert 10-bit → 8-bit unless the encoder accepts 10-bit
            let frame = if !encoder_supports_10bit
                && matches!(
                    frame.video_params().map(|p| p.format),
                    Some(rust_media::PixelFormat::YUV420P10LE)
                )
            {
                rust_media_filter::video::yuv420p10le_to_yuv420p(&frame)?
            } else {
                frame
            };

            // Apply crop filter (runs before scale)
            let frame = if let Some(cf) = crop_filter {
                cf.process(&frame)?
            } else {
                frame
            };

            // Apply scale filter if enabled
            let frame = if let Some(sf) = scale_filter {
                sf.process(&frame)?
            } else {
                frame
            };

            reorder_buf.push(frame);
            while let Some(ordered_frame) = reorder_buf.pop_ready() {
                if let Some(out_idx) = video_out_idx {
                    encode_and_mux(encoder, muxer, &ordered_frame, out_idx, &mut output_bytes)?;
                }
            }
        }

        // Drain all remaining frames from reorder buffer
        while let Some(ordered_frame) = reorder_buf.flush_next() {
            if let Some(out_idx) = video_out_idx {
                encode_and_mux(encoder, muxer, &ordered_frame, out_idx, &mut output_bytes)?;
            }
        }

        // Flush the encoder itself
        encoder.flush()?;
        while let Ok(mut enc_packet) = encoder.receive_packet() {
            if let Some(out_idx) = video_out_idx {
                enc_packet.set_stream_index(out_idx);
                muxer.write_packet(&enc_packet)?;
            }
        }
    }

    // Flush audio encoder
    if let (Some(decoder), Some(encoder)) = (audio_decoder.as_mut(), audio_encoder.as_mut()) {
        decoder.flush()?;
        while let Ok(frame) = decoder.receive_frame() {
            let frame = if let Some(resampler) = audio_resampler {
                resampler.resample(&frame)?
            } else {
                frame
            };
            let frame = if let Some(vf) = volume_filter {
                vf.process(&frame)?
            } else {
                frame
            };
            encoder.send_frame(&frame)?;
            while let Ok(mut enc_packet) = encoder.receive_packet() {
                if let Some(out_idx) = audio_out_idx {
                    enc_packet.set_stream_index(out_idx);
                    muxer.write_packet(&enc_packet)?;
                }
            }
        }

        encoder.flush()?;
        while let Ok(mut enc_packet) = encoder.receive_packet() {
            if let Some(out_idx) = audio_out_idx {
                enc_packet.set_stream_index(out_idx);
                muxer.write_packet(&enc_packet)?;
            }
        }
    }

    Ok(())
}

/// Tracks progress state for ffmpeg-style status line output.
struct ProgressState {
    start_time: std::time::Instant,
    last_print: std::time::Instant,
    total_size: u64,
    last_pts_us: Option<i64>,
}

impl ProgressState {
    fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            start_time: now,
            last_print: now,
            total_size: 0,
            last_pts_us: None,
        }
    }

    /// Update progress timestamp. Prints at most every 500ms.
    /// `pts_us` should be pre-converted to microseconds by the caller.
    fn update(&mut self, pts_us: Option<i64>) {
        if let Some(us) = pts_us {
            // Only update if this is a larger timestamp (avoids audio/video interleaving jitter)
            if self.last_pts_us.is_none_or(|prev| us > prev) {
                self.last_pts_us = Some(us);
            }
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.last_print).as_millis() < 500 {
            return;
        }
        self.last_print = now;
        self.print();
    }

    /// Force-print the current status (used at end of transcode).
    fn finish(&self) {
        self.print();
        eprintln!();
    }

    fn print(&self) {
        use std::io::Write;
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let pts_us = self.last_pts_us;

        // Time position
        let time_str = match pts_us {
            Some(us) if us >= 0 => {
                let secs = us as f64 / 1_000_000.0;
                let h = (secs / 3600.0) as u64;
                let m = ((secs % 3600.0) / 60.0) as u64;
                let s = secs % 60.0;
                format!("{:02}:{:02}:{:05.2}", h, m, s)
            }
            _ => "N/A".to_string(),
        };

        // Speed (media seconds per wall second)
        let speed_str = match pts_us {
            Some(us) if us > 0 && elapsed > 0.1 => {
                let media_secs = us as f64 / 1_000_000.0;
                format!("{:.2}x", media_secs / elapsed)
            }
            _ => "N/A".to_string(),
        };

        // Output size
        let size_str = if self.total_size > 1_000_000 {
            format!("{:.1}MB", self.total_size as f64 / 1_000_000.0)
        } else {
            format!("{:.0}kB", self.total_size as f64 / 1_000.0)
        };

        // Bitrate
        let bitrate_str = if elapsed > 0.5 {
            let kbits = (self.total_size as f64 * 8.0) / (elapsed * 1000.0);
            format!("{:.1}kbits/s", kbits)
        } else {
            "N/A".to_string()
        };

        eprint!(
            "\rtime={} bitrate={} size={} speed={}    ",
            time_str, bitrate_str, size_str, speed_str
        );
        let _ = std::io::stderr().flush();
    }
}

// ============================================================================
// Decoder wrapper trait for dynamic dispatch
// ============================================================================

trait DecoderWrapper {
    fn send_packet(&mut self, packet: &Packet) -> rust_media::Result<()>;
    fn receive_frame(&mut self) -> rust_media::Result<Frame>;
    fn flush(&mut self) -> rust_media::Result<()>;
}

impl<D: Decoder> DecoderWrapper for D {
    fn send_packet(&mut self, packet: &Packet) -> rust_media::Result<()> {
        Decoder::send_packet(self, packet)
    }

    fn receive_frame(&mut self) -> rust_media::Result<Frame> {
        Decoder::receive_frame(self)
    }

    fn flush(&mut self) -> rust_media::Result<()> {
        Decoder::flush(self)
    }
}

// ============================================================================
// Encoder wrapper trait for dynamic dispatch
// ============================================================================

trait EncoderWrapper {
    fn send_frame(&mut self, frame: &Frame) -> rust_media::Result<()>;
    fn receive_packet(&mut self) -> rust_media::Result<Packet>;
    fn flush(&mut self) -> rust_media::Result<()>;
}

impl<E: Encoder> EncoderWrapper for E {
    fn send_frame(&mut self, frame: &Frame) -> rust_media::Result<()> {
        Encoder::send_frame(self, frame)
    }

    fn receive_packet(&mut self) -> rust_media::Result<Packet> {
        Encoder::receive_packet(self)
    }

    fn flush(&mut self) -> rust_media::Result<()> {
        Encoder::flush(self)
    }
}

// ============================================================================
// Demuxer opening with format detection
// ============================================================================

/// Open a demuxer for the given file path, using magic bytes + extension detection.
fn open_demuxer(
    path: &std::path::Path,
) -> Result<(Box<dyn Demuxer>, ContainerFormat), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let format = rust_media_format::detect::detect(&mut reader, path).ok_or_else(|| {
        format!(
            "Unrecognized format for '{}'. Supported: mp4, mov, m4a, m4v, webm, mkv, wav",
            path.display()
        )
    })?;

    let demuxer: Box<dyn Demuxer> = match format {
        ContainerFormat::Mp4 => Box::new(Mp4Demuxer::new(reader)?),
        ContainerFormat::WebM | ContainerFormat::Mkv => Box::new(WebmDemuxer::open(reader)?),
        ContainerFormat::Wav => Box::new(WavDemuxer::open(reader)?),
    };

    Ok((demuxer, format))
}

// ============================================================================
// Decoder/Encoder creation functions
// ============================================================================

fn create_decoder(stream: &StreamInfo) -> Option<Box<dyn DecoderWrapper>> {
    match stream.codec.as_str() {
        "pcm" | "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" => {
            rust_media_codec::PcmDecoder::new(stream.clone())
                .ok()
                .map(|d| Box::new(d) as Box<dyn DecoderWrapper>)
        }
        "opus" => rust_media_codec::OpusDecoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "mp3" => rust_media_codec::Mp3Decoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "vorbis" => rust_media_codec::VorbisDecoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "vp8" => rust_media_codec::Vp8Decoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "vp9" => rust_media_codec::Vp9Decoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "av1" | "av01" => rust_media_codec::Av1Decoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "h264" | "avc" => {
            // On macOS with videotoolbox feature, try VideoToolbox first (hardware acceleration + all profiles)
            // Falls back to OpenH264 if VideoToolbox fails
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            {
                if let Ok(decoder) = rust_media_codec::VideoToolboxH264Decoder::new(stream.clone()) {
                    return Some(Box::new(decoder) as Box<dyn DecoderWrapper>);
                }
            }
            // Fall back to OpenH264 (Constrained Baseline only)
            rust_media_codec::H264Decoder::new(stream.clone())
                .ok()
                .map(|d| Box::new(d) as Box<dyn DecoderWrapper>)
        }
        "hevc" | "h265" | "hvc1" | "hev1" => {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            {
                if let Ok(decoder) = rust_media_codec::VideoToolboxHevcDecoder::new(stream.clone()) {
                    return Some(Box::new(decoder) as Box<dyn DecoderWrapper>);
                }
            }
            None // No software HEVC decoder available
        }
        #[cfg(feature = "fdk-aac")]
        "aac" => rust_media_codec::FdkAacDecoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        _ => None,
    }
}

fn create_decoder_for_stream(
    stream: &StreamInfo,
) -> Result<Box<dyn DecoderWrapper>, Box<dyn std::error::Error>> {
    create_decoder(stream).ok_or_else(|| format!("No decoder available for codec: {}", stream.codec).into())
}

/// Translate the CLI's `EncoderOptions` into an `Av1EncoderConfig`.
///
/// Defaults are pulled from `Av1EncoderConfig::default()`. Each `Some(_)`
/// in `opts` overrides the corresponding rav1e default. The `bitrate`
/// argument is the value that came from `--video-bitrate` (in bps): it's
/// applied **only if `opts.qp` is not set**, matching FFmpeg's behavior
/// where `-qp` puts the encoder into constant-quality mode and overrides
/// `-b:v`.
fn build_vp8_config_from_opts(bitrate: u64, opts: &EncoderOptions) -> rust_media_codec::Vp8EncoderConfig {
    let mut cfg = rust_media_codec::Vp8EncoderConfig::new();
    if let Some(speed) = opts.speed {
        cfg = cfg.speed(speed as i32);
    }
    if let Some(qp) = opts.qp {
        // Map --qp to CQ mode with the bitrate as a cap
        cfg = cfg.rate_control(rust_media_codec::Vp8RateControl::CQ {
            bitrate: bitrate as u32,
            cq_level: qp as u32,
        });
    } else if bitrate > 0 {
        cfg = cfg.rate_control(rust_media_codec::Vp8RateControl::VBR(bitrate as u32));
    }
    if let Some(gop) = opts.gop_size {
        cfg = cfg.gop_size(gop as u32);
    }
    if let Some(min) = opts.keyint_min {
        cfg = cfg.keyint_min(min as u32);
    }
    cfg
}

fn build_vp9_config_from_opts(bitrate: u64, opts: &EncoderOptions) -> rust_media_codec::Vp9EncoderConfig {
    let mut cfg = rust_media_codec::Vp9EncoderConfig::new();
    if let Some(speed) = opts.speed {
        cfg = cfg.speed(speed as i32);
    }
    if let Some(qp) = opts.qp {
        cfg = cfg.rate_control(rust_media_codec::Vp9RateControl::CQ {
            bitrate: bitrate as u32,
            cq_level: qp as u32,
        });
    } else if bitrate > 0 {
        cfg = cfg.rate_control(rust_media_codec::Vp9RateControl::VBR(bitrate as u32));
    }
    if let Some(gop) = opts.gop_size {
        cfg = cfg.gop_size(gop as u32);
    }
    if let Some(min) = opts.keyint_min {
        cfg = cfg.keyint_min(min as u32);
    }
    if let Some(n) = opts.tile_columns {
        cfg = cfg.tile_columns(n as i32);
    }
    if let Some(n) = opts.tile_rows {
        cfg = cfg.tile_rows(n as i32);
    }
    cfg
}

#[cfg(feature = "gpl-x264")]
fn build_x264_config_from_opts(opts: &EncoderOptions) -> rust_media_codec::X264EncoderConfig {
    let mut cfg = rust_media_codec::X264EncoderConfig::new();
    if let Some(speed) = opts.speed {
        cfg = cfg.speed(speed);
    }
    if let Some(qp) = opts.qp {
        cfg = cfg.crf(qp as f32);
    }
    if let Some(gop) = opts.gop_size {
        cfg = cfg.gop_size(gop as u32);
    }
    if let Some(min) = opts.keyint_min {
        cfg = cfg.keyint_min(min as u32);
    }
    cfg
}

fn build_av1_config_from_opts(bitrate: u64, opts: &EncoderOptions) -> rust_media_codec::Av1EncoderConfig {
    let mut cfg = rust_media_codec::Av1EncoderConfig::new();
    if let Some(speed) = opts.speed {
        cfg = cfg.speed_preset(speed);
    }
    // Rate control: --qp wins over --video-bitrate (mirrors ffmpeg behavior).
    if let Some(qp) = opts.qp {
        cfg = cfg.quantizer(qp);
    } else if bitrate > 0 {
        let bps = bitrate.min(u32::MAX as u64) as u32;
        cfg = cfg.bitrate(bps);
    }
    // Keyframe interval. If only one of (gop_size, keyint_min) is supplied,
    // keep the rav1e default for the other.
    if opts.gop_size.is_some() || opts.keyint_min.is_some() {
        let min = opts.keyint_min.unwrap_or(12);
        let max = opts.gop_size.unwrap_or(240);
        cfg = cfg.key_frame_interval(min, max);
    }
    if let Some(n) = opts.tile_columns {
        cfg = cfg.tile_cols(n);
    }
    if let Some(n) = opts.tile_rows {
        cfg = cfg.tile_rows(n);
    }
    if let Some(n) = opts.tiles {
        cfg = cfg.tiles(n);
    }
    cfg
}

/// Builds the AV1 sequence header (av1C / Matroska CodecPrivate) for the
/// given input stream by spinning up a throwaway rav1e encoder.
///
/// The sequence header is fully determined by the encoder config (width,
/// height, bit depth, chroma sampling), so we don't need to send any frames
/// through the encoder first. The throwaway encoder is dropped immediately
/// after — the real one is constructed later in `create_video_encoder`.
///
/// The throwaway encoder must be built with the **same config** as the real
/// one (otherwise the av1C bytes won't match what gets emitted), so we accept
/// the same `encoder_opts` here and feed it through `build_av1_config_from_opts`.
fn build_av1_codec_config(
    input_stream: &StreamInfo,
    bitrate: u64,
    encoder_opts: &EncoderOptions,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let (width, height, frame_rate, input_pixel_format) = match &input_stream.params {
        StreamParams::Video(p) => (p.width, p.height, p.frame_rate, p.pixel_format),
        _ => return Err("av1 codec config: not a video stream".into()),
    };
    let encoder_pixel_format = match input_pixel_format {
        PixelFormat::YUV420P10LE => PixelFormat::YUV420P10LE,
        _ => PixelFormat::YUV420P,
    };
    let video_params = VideoStreamParams::new(width, height, encoder_pixel_format)
        .with_frame_rate(frame_rate.0, frame_rate.1);
    let stream_info = StreamInfo::new(0, MediaType::Video, "av1".to_string())
        .with_params(StreamParams::Video(video_params))
        .with_time_base(input_stream.time_base.0, input_stream.time_base.1);
    let av1_config = build_av1_config_from_opts(bitrate, encoder_opts);
    let encoder = rust_media_codec::Av1Encoder::with_config(stream_info, av1_config)?;
    Ok(encoder.codec_config())
}

/// Build the VP9 codec configuration record (vpcC payload) for a given
/// input stream by spinning up a throwaway VP9 encoder.
fn build_vp9_codec_config(
    input_stream: &StreamInfo,
    bitrate: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let (width, height, frame_rate) = match &input_stream.params {
        StreamParams::Video(p) => (p.width, p.height, p.frame_rate),
        _ => return Err("vp9 codec config: not a video stream".into()),
    };
    let video_params = VideoStreamParams::new(width, height, PixelFormat::YUV420P)
        .with_frame_rate(frame_rate.0, frame_rate.1);
    let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
        .with_params(StreamParams::Video(video_params))
        .with_bitrate(bitrate)
        .with_time_base(input_stream.time_base.0, input_stream.time_base.1);
    let encoder = rust_media_codec::Vp9Encoder::new(stream_info)?;
    Ok(encoder.codec_config())
}

/// Build the H.264 avcC (AVCDecoderConfigurationRecord) for a given input
/// stream by spinning up a throwaway x264 encoder.
#[cfg(feature = "gpl-x264")]
fn build_h264_codec_config(
    input_stream: &StreamInfo,
    bitrate: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let (width, height, frame_rate) = match &input_stream.params {
        StreamParams::Video(p) => (p.width, p.height, p.frame_rate),
        _ => return Err("h264 codec config: not a video stream".into()),
    };
    let video_params = VideoStreamParams::new(width, height, PixelFormat::YUV420P)
        .with_frame_rate(frame_rate.0, frame_rate.1);
    let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
        .with_params(StreamParams::Video(video_params))
        .with_bitrate(bitrate)
        .with_time_base(input_stream.time_base.0, input_stream.time_base.1);
    let encoder = rust_media_codec::X264Encoder::new(stream_info)?;
    Ok(encoder.codec_config().to_vec())
}

fn create_video_encoder(
    input_stream: &StreamInfo,
    codec: &str,
    bitrate: u64,
    encoder_opts: &EncoderOptions,
) -> Result<Box<dyn EncoderWrapper>, Box<dyn std::error::Error>> {
    let (width, height, frame_rate, input_pixel_format) = match &input_stream.params {
        StreamParams::Video(params) => (
            params.width,
            params.height,
            params.frame_rate,
            params.pixel_format,
        ),
        _ => return Err("Not a video stream".into()),
    };

    // For AV1 we honor the input bit depth so 10-bit content can be encoded
    // natively without going through the auto-downconversion path. Other
    // encoders are 8-bit only and always get YUV420P.
    let encoder_pixel_format = match (codec, input_pixel_format) {
        ("av1" | "av01", PixelFormat::YUV420P10LE) => PixelFormat::YUV420P10LE,
        _ => PixelFormat::YUV420P,
    };

    let video_params = VideoStreamParams::new(width, height, encoder_pixel_format)
        .with_frame_rate(frame_rate.0, frame_rate.1);

    let mut stream_info = StreamInfo::new(0, MediaType::Video, codec.to_string())
        .with_params(StreamParams::Video(video_params))
        .with_bitrate(bitrate)
        .with_time_base(input_stream.time_base.0, input_stream.time_base.1);

    // Copy extra data if same codec
    if input_stream.codec == codec {
        stream_info.extra_data = input_stream.extra_data.clone();
    }

    match codec {
        "vp8" => {
            let vp8_config = build_vp8_config_from_opts(bitrate, encoder_opts);
            let encoder = rust_media_codec::Vp8Encoder::with_config(stream_info, vp8_config)?;
            Ok(Box::new(encoder))
        }
        "vp9" => {
            let vp9_config = build_vp9_config_from_opts(bitrate, encoder_opts);
            let encoder = rust_media_codec::Vp9Encoder::with_config(stream_info, vp9_config)?;
            Ok(Box::new(encoder))
        }
        "av1" | "av01" => {
            // Use the explicit AV1 builder so the per-encoder flags
            // (--speed, --qp, --gop-size, --keyint-min, --tiles, ...) are
            // honored. We strip stream_info.bitrate here because the config
            // already encodes the rate-control choice — leaving it set
            // would just be redundant.
            stream_info.bitrate = None;
            let av1_config = build_av1_config_from_opts(bitrate, encoder_opts);
            let encoder = rust_media_codec::Av1Encoder::with_config(stream_info, av1_config)?;
            Ok(Box::new(encoder))
        }
        #[cfg(feature = "gpl-x264")]
        "h264" => {
            let x264_config = build_x264_config_from_opts(encoder_opts);
            let encoder = rust_media_codec::X264Encoder::with_config(stream_info, x264_config)?;
            Ok(Box::new(encoder))
        }
        _ => Err(format!("No encoder available for video codec: {}", codec).into()),
    }
}

/// Returns the nearest supported sample rate for the given encoder codec,
/// or None if the rate is already supported.
/// Parse `scale` filter dimensions from a Filter spec.
///
/// Accepts both ffmpeg-style positional syntax (`scale=640:480`) and named
/// syntax (`scale=w=640:h=480` or `scale=width=640:height=480`).
///
/// Returns signed values to allow ffmpeg-style aspect-ratio markers:
/// - `-1`: preserve aspect ratio from the other dimension (no rounding)
/// - `-2`: preserve aspect ratio, round to nearest even value (recommended for video)
///
/// Use `resolve_scale_dimensions` to convert these to concrete pixel dimensions
/// once the source frame size is known.
fn parse_scale_dimensions(filter: &Filter) -> Result<(i32, i32), String> {
    // Named form: w=W:h=H or width=W:height=H
    if let Some(w_str) = filter
        .get_param("w")
        .or_else(|| filter.get_param("width"))
    {
        let h_str = filter
            .get_param("h")
            .or_else(|| filter.get_param("height"))
            .ok_or("missing height (use h=H or height=H)")?;
        let w = w_str
            .parse::<i32>()
            .map_err(|_| format!("invalid width: {}", w_str))?;
        let h = h_str
            .parse::<i32>()
            .map_err(|_| format!("invalid height: {}", h_str))?;
        return Ok((w, h));
    }

    // Positional form: scale=W:H (uses Filter.positional which preserves order)
    if filter.positional.len() == 2 {
        let w = filter.positional[0]
            .parse::<i32>()
            .map_err(|_| format!("invalid width: {}", filter.positional[0]))?;
        let h = filter.positional[1]
            .parse::<i32>()
            .map_err(|_| format!("invalid height: {}", filter.positional[1]))?;
        return Ok((w, h));
    }

    Err("scale filter requires width and height (e.g., scale=640:480 or scale=w=640:h=480)"
        .to_string())
}

/// Resolve `scale` dimensions, expanding ffmpeg-style aspect-ratio markers
/// (`-1` and `-2`) into concrete pixel dimensions based on the source size.
///
/// - `-1`: preserve aspect ratio (may yield odd dimensions)
/// - `-2`: preserve aspect ratio, round to nearest even value
/// - positive integer: use as-is
///
/// At least one of `parsed_w` and `parsed_h` must be a positive integer.
fn resolve_scale_dimensions(
    parsed_w: i32,
    parsed_h: i32,
    src_w: usize,
    src_h: usize,
) -> Result<(usize, usize), String> {
    if src_w == 0 || src_h == 0 {
        return Err(format!("invalid source dimensions {}x{}", src_w, src_h));
    }

    let resolve = |target: i32, fixed: i32, src_a: usize, src_b: usize| -> Result<usize, String> {
        match target {
            n if n > 0 => Ok(n as usize),
            -1 => {
                // Preserve aspect ratio: target = fixed * src_a / src_b
                if fixed <= 0 {
                    return Err(
                        "scale: at least one of width/height must be positive when using -1/-2"
                            .to_string(),
                    );
                }
                let computed = (fixed as i64 * src_a as i64) / src_b as i64;
                if computed <= 0 {
                    return Err(format!(
                        "scale: aspect-ratio computation produced non-positive result {}",
                        computed
                    ));
                }
                Ok(computed as usize)
            }
            -2 => {
                if fixed <= 0 {
                    return Err(
                        "scale: at least one of width/height must be positive when using -1/-2"
                            .to_string(),
                    );
                }
                let computed = (fixed as i64 * src_a as i64) / src_b as i64;
                if computed <= 0 {
                    return Err(format!(
                        "scale: aspect-ratio computation produced non-positive result {}",
                        computed
                    ));
                }
                // Round to nearest even value
                let rounded = ((computed + 1) / 2) * 2;
                Ok(rounded.max(2) as usize)
            }
            _ => Err(format!(
                "scale: invalid dimension {} (use a positive integer, -1, or -2)",
                target
            )),
        }
    };

    let w = resolve(parsed_w, parsed_h, src_w, src_h)?;
    let h = resolve(parsed_h, parsed_w, src_h, src_w)?;
    Ok((w, h))
}

/// Parse `crop` filter parameters from a Filter spec.
///
/// Accepts:
/// - `crop=W:H` (centered, X and Y default to centered)
/// - `crop=W:H:X:Y` (positional, all four)
/// - `crop=w=W:h=H[:x=X][:y=Y]` (named)
///
/// Returns `(width, height, x, y)` where x/y are None if centered.
#[allow(clippy::type_complexity)]
fn parse_crop_params(
    filter: &Filter,
) -> Result<(usize, usize, Option<usize>, Option<usize>), String> {
    // Named form: w=W:h=H[:x=X][:y=Y] or width/height/x/y
    if let Some(w_str) = filter
        .get_param("w")
        .or_else(|| filter.get_param("width"))
    {
        let h_str = filter
            .get_param("h")
            .or_else(|| filter.get_param("height"))
            .ok_or("missing height (use h=H or height=H)")?;
        let w = w_str
            .parse::<usize>()
            .map_err(|_| format!("invalid width: {}", w_str))?;
        let h = h_str
            .parse::<usize>()
            .map_err(|_| format!("invalid height: {}", h_str))?;
        let x = filter
            .get_param("x")
            .map(|s| s.parse::<usize>().map_err(|_| format!("invalid x: {}", s)))
            .transpose()?;
        let y = filter
            .get_param("y")
            .map(|s| s.parse::<usize>().map_err(|_| format!("invalid y: {}", s)))
            .transpose()?;
        return Ok((w, h, x, y));
    }

    // Positional form: W:H or W:H:X:Y
    match filter.positional.len() {
        2 => {
            let w = filter.positional[0]
                .parse::<usize>()
                .map_err(|_| format!("invalid width: {}", filter.positional[0]))?;
            let h = filter.positional[1]
                .parse::<usize>()
                .map_err(|_| format!("invalid height: {}", filter.positional[1]))?;
            Ok((w, h, None, None))
        }
        4 => {
            let w = filter.positional[0]
                .parse::<usize>()
                .map_err(|_| format!("invalid width: {}", filter.positional[0]))?;
            let h = filter.positional[1]
                .parse::<usize>()
                .map_err(|_| format!("invalid height: {}", filter.positional[1]))?;
            let x = filter.positional[2]
                .parse::<usize>()
                .map_err(|_| format!("invalid x: {}", filter.positional[2]))?;
            let y = filter.positional[3]
                .parse::<usize>()
                .map_err(|_| format!("invalid y: {}", filter.positional[3]))?;
            Ok((w, h, Some(x), Some(y)))
        }
        _ => Err(
            "crop filter requires width:height or width:height:x:y (e.g., crop=320:240 or crop=320:240:160:120)"
                .to_string(),
        ),
    }
}

fn nearest_supported_sample_rate(codec: &str, input_rate: u32) -> Option<u32> {
    let supported: &[u32] = match codec {
        "opus" => &[8000, 12000, 16000, 24000, 48000],
        "pcm" => return None, // PCM supports any rate
        #[cfg(feature = "fdk-aac")]
        "aac" => return None, // fdk-aac supports a wide range
        _ => return None,
    };

    if supported.contains(&input_rate) {
        return None; // already supported
    }

    // Find the nearest supported rate
    Some(
        *supported
            .iter()
            .min_by_key(|&&r| (r as i64 - input_rate as i64).unsigned_abs())
            .unwrap(),
    )
}

fn create_audio_encoder(
    input_stream: &StreamInfo,
    codec: &str,
    bitrate: u64,
) -> Result<Box<dyn EncoderWrapper>, Box<dyn std::error::Error>> {
    let (sample_rate, channels) = match &input_stream.params {
        StreamParams::Audio(params) => (params.sample_rate, params.channels),
        _ => return Err("Not an audio stream".into()),
    };

    let audio_params = AudioStreamParams::new(sample_rate, channels, SampleFormat::S16);

    let stream_info = StreamInfo::new(0, MediaType::Audio, codec.to_string())
        .with_params(StreamParams::Audio(audio_params))
        .with_bitrate(bitrate)
        .with_time_base(1, sample_rate);

    match codec {
        "opus" => {
            let encoder = rust_media_codec::OpusEncoder::new(stream_info)?;
            Ok(Box::new(encoder))
        }
        "pcm" => {
            let encoder = rust_media_codec::PcmEncoder::new(stream_info)?;
            Ok(Box::new(encoder))
        }
        #[cfg(feature = "fdk-aac")]
        "aac" => {
            let encoder = rust_media_codec::FdkAacEncoder::new(stream_info)?;
            Ok(Box::new(encoder))
        }
        _ => Err(format!("No encoder available for audio codec: {}", codec).into()),
    }
}

// ============================================================================
// Media info implementation
// ============================================================================

fn run_info(
    input: &std::path::Path,
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
    output_format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let filename = input.to_string_lossy().to_string();

    let (mut demuxer, _format) = open_demuxer(input)?;

    let container = demuxer.container_info()?;
    let raw_streams = demuxer.streams()?;
    let streams_json: Vec<StreamInfoJson> = raw_streams.iter().map(stream_to_json).collect();
    let time_bases: Vec<(u32, u32)> = raw_streams.iter().map(|s| s.time_base).collect();

    match output_format {
        OutputFormat::Text => {
            // Print header immediately
            print_text_header(&filename, &container.format_name, container.duration, container.bitrate, &streams_json);

            // Stream packets/frames as they are decoded
            if show_packets || show_frames {
                stream_packets_and_frames_text(
                    &mut *demuxer,
                    &raw_streams,
                    &time_bases,
                    show_packets,
                    show_frames,
                    count,
                    stream_filter,
                )?;
            }
        }
        OutputFormat::Json => {
            // JSON needs the complete structure, so buffer everything
            let (packets, frames) = if show_packets || show_frames {
                collect_packets_and_frames(
                    &mut *demuxer,
                    &raw_streams,
                    &time_bases,
                    show_packets,
                    show_frames,
                    count,
                    stream_filter,
                )?
            } else {
                (Vec::new(), Vec::new())
            };

            let media_info = MediaInfo {
                format: FormatInfo {
                    filename: filename.clone(),
                    format_name: container.format_name,
                    duration_us: container.duration,
                    duration: container.duration.map(format_duration),
                    bitrate: container.bitrate,
                    nb_streams: streams_json.len(),
                },
                streams: streams_json,
                packets,
                frames,
            };
            println!("{}", serde_json::to_string_pretty(&media_info)?);
        }
    }

    Ok(())
}

// ============================================================================
// Packet and frame collection (used by JSON output mode)
// ============================================================================

fn collect_packets_and_frames(
    demuxer: &mut dyn Demuxer,
    streams: &[StreamInfo],
    time_bases: &[(u32, u32)],
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
) -> Result<(Vec<PacketInfo>, Vec<FrameInfo>), Box<dyn std::error::Error>> {
    let mut packets = Vec::new();
    let mut frames = Vec::new();
    let mut packet_index = 0;
    let mut frame_index = 0;

    // Create decoders for frame analysis if needed
    let mut decoders: Vec<Option<Box<dyn DecoderWrapper>>> = if show_frames {
        streams.iter().map(|s| create_decoder(s)).collect()
    } else {
        vec![]
    };

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                let stream_idx = packet.stream_index();

                // Apply stream filter
                if let Some(filter) = stream_filter {
                    if stream_idx != filter {
                        continue;
                    }
                }

                let time_base = time_bases.get(stream_idx).copied().unwrap_or((1, 1000000));

                // Collect packet info
                if show_packets {
                    let pkt_info = PacketInfo {
                        packet_index,
                        stream_index: stream_idx,
                        media_type: format!("{:?}", packet.media_type()).to_lowercase(),
                        pts: packet.pts(),
                        pts_time: packet.pts().map(|p| format_time(p, time_base)),
                        dts: packet.dts(),
                        dts_time: packet.dts().map(|d| format_time(d, time_base)),
                        duration: packet.duration(),
                        size: packet.size(),
                        is_keyframe: packet.is_keyframe(),
                    };
                    packets.push(pkt_info);
                }

                // Decode and collect frame info
                if show_frames {
                    if let Some(Some(decoder)) = decoders.get_mut(stream_idx) {
                        if decoder.send_packet(&packet).is_ok() {
                            while let Ok(frame) = decoder.receive_frame() {
                                let frame_info = FrameInfo {
                                    frame_index,
                                    stream_index: stream_idx,
                                    media_type: format!("{:?}", frame.media_type()).to_lowercase(),
                                    pts: frame.pts(),
                                    pts_time: frame.pts().map(|p| format_time(p, time_base)),
                                    is_keyframe: frame.is_keyframe(),
                                    pict_type: Some(if frame.is_keyframe() {
                                        "I".to_string()
                                    } else {
                                        "P".to_string()
                                    }),
                                    params: frame_to_params_json(&frame),
                                };
                                frames.push(frame_info);
                                frame_index += 1;

                                // Check count limit for frames
                                if count > 0 && frames.len() >= count {
                                    break;
                                }
                            }
                        }
                    }
                }

                packet_index += 1;

                // Check count limit
                let check_count = if show_frames {
                    frames.len()
                } else {
                    packets.len()
                };
                if count > 0 && check_count >= count {
                    break;
                }
            }
            Err(rust_media::Error::EndOfStream) => break,
            Err(_) => break,
        }
    }

    // Flush decoders
    if show_frames {
        for (stream_idx, decoder_opt) in decoders.iter_mut().enumerate() {
            if let Some(decoder) = decoder_opt {
                let _ = decoder.flush();
                let time_base = time_bases.get(stream_idx).copied().unwrap_or((1, 1000000));
                while let Ok(frame) = decoder.receive_frame() {
                    let frame_info = FrameInfo {
                        frame_index,
                        stream_index: stream_idx,
                        media_type: format!("{:?}", frame.media_type()).to_lowercase(),
                        pts: frame.pts(),
                        pts_time: frame.pts().map(|p| format_time(p, time_base)),
                        is_keyframe: frame.is_keyframe(),
                        pict_type: Some(if frame.is_keyframe() {
                            "I".to_string()
                        } else {
                            "P".to_string()
                        }),
                        params: frame_to_params_json(&frame),
                    };
                    frames.push(frame_info);
                    frame_index += 1;

                    if count > 0 && frames.len() >= count {
                        break;
                    }
                }
            }
        }
    }

    Ok((packets, frames))
}

// ============================================================================
// Helper functions
// ============================================================================

fn stream_to_json(stream: &StreamInfo) -> StreamInfoJson {
    StreamInfoJson {
        index: stream.index,
        media_type: format!("{:?}", stream.media_type).to_lowercase(),
        codec: stream.codec.clone(),
        time_base: format!("{}/{}", stream.time_base.0, stream.time_base.1),
        duration: stream.duration,
        bitrate: stream.bitrate,
        params: match &stream.params {
            StreamParams::Video(v) => StreamParamsJson::Video {
                width: v.width,
                height: v.height,
                pixel_format: format!("{:?}", v.pixel_format).to_lowercase(),
                frame_rate: format!("{}/{}", v.frame_rate.0, v.frame_rate.1),
                color_space: match v.color_space {
                    rust_media::ColorSpace::Unknown => None,
                    cs => Some(format!("{:?}", cs).to_lowercase()),
                },
                bit_depth: v.bit_depth,
            },
            StreamParams::Audio(a) => StreamParamsJson::Audio {
                sample_rate: a.sample_rate,
                channels: a.channels,
                sample_format: format!("{:?}", a.sample_format).to_lowercase(),
            },
            StreamParams::Unknown => StreamParamsJson::Unknown {},
        },
    }
}

fn frame_to_params_json(frame: &Frame) -> FrameParamsJson {
    match frame.media_type() {
        MediaType::Video => {
            if let Some(params) = frame.video_params() {
                FrameParamsJson::Video {
                    width: params.width,
                    height: params.height,
                    pixel_format: format!("{:?}", params.format).to_lowercase(),
                }
            } else {
                FrameParamsJson::Unknown {}
            }
        }
        MediaType::Audio => {
            if let Some(params) = frame.audio_params() {
                FrameParamsJson::Audio {
                    nb_samples: params.num_samples,
                    channels: params.channels,
                    sample_format: format!("{:?}", params.format).to_lowercase(),
                }
            } else {
                FrameParamsJson::Unknown {}
            }
        }
        _ => FrameParamsJson::Unknown {},
    }
}

fn format_time(timestamp: i64, time_base: (u32, u32)) -> String {
    let seconds = (timestamp as f64) * (time_base.0 as f64) / (time_base.1 as f64);
    format!("{:.6}", seconds)
}

fn format_duration(microseconds: i64) -> String {
    let total_seconds = microseconds as f64 / 1_000_000.0;
    let hours = (total_seconds / 3600.0) as u32;
    let minutes = ((total_seconds % 3600.0) / 60.0) as u32;
    let seconds = total_seconds % 60.0;

    if hours > 0 {
        format!("{}:{:02}:{:06.3}", hours, minutes, seconds)
    } else {
        format!("{}:{:06.3}", minutes, seconds)
    }
}

// ============================================================================
// Text output formatting
// ============================================================================

/// Print the header section (format info + streams) for text output mode.
fn print_text_header(
    filename: &str,
    format_name: &str,
    duration: Option<i64>,
    bitrate: Option<u64>,
    streams: &[StreamInfoJson],
) {
    println!("Input: {}", filename);
    println!("  Format:     {}", format_name);
    if let Some(dur) = duration {
        println!("  Duration:   {}", format_duration(dur));
    }
    if let Some(br) = bitrate {
        println!("  Bitrate:    {} kb/s", br / 1000);
    }
    println!("  Streams:    {}", streams.len());
    println!();

    println!("Streams:");
    for stream in streams {
        print!(
            "  Stream #{}: {} ({})",
            stream.index, stream.media_type, stream.codec
        );

        match &stream.params {
            StreamParamsJson::Video {
                width,
                height,
                pixel_format,
                frame_rate,
                bit_depth,
                ..
            } => {
                print!(", {}x{}", width, height);
                print!(", {}", pixel_format);
                print!(", {} fps", frame_rate);
                print!(", {} bit", bit_depth);
            }
            StreamParamsJson::Audio {
                sample_rate,
                channels,
                sample_format,
            } => {
                print!(", {} Hz", sample_rate);
                print!(", {} ch", channels);
                print!(", {}", sample_format);
            }
            StreamParamsJson::Unknown {} => {}
        }

        if let Some(br) = stream.bitrate {
            print!(", {} kb/s", br / 1000);
        }

        println!();
    }
}

/// Print a single packet line in text format.
fn print_text_packet(pkt: &PacketInfo) {
    println!(
        "{:>6} {:>6} {:>8} {:>12} {:>12} {:>12} {:>8} {:>4}",
        pkt.packet_index,
        pkt.stream_index,
        pkt.media_type,
        pkt.pts.map(|p| p.to_string()).unwrap_or("-".to_string()),
        pkt.pts_time.as_deref().unwrap_or("-"),
        pkt.dts.map(|d| d.to_string()).unwrap_or("-".to_string()),
        pkt.size,
        if pkt.is_keyframe { "K" } else { "" }
    );
}

/// Print a single frame line in text format.
fn print_text_frame(frm: &FrameInfo) {
    let info_str = match &frm.params {
        FrameParamsJson::Video {
            width,
            height,
            pixel_format,
        } => format!("{}x{} {}", width, height, pixel_format),
        FrameParamsJson::Audio {
            nb_samples,
            channels,
            ..
        } => format!("{} samples, {} ch", nb_samples, channels),
        FrameParamsJson::Unknown {} => "-".to_string(),
    };

    println!(
        "{:>6} {:>6} {:>8} {:>12} {:>12} {:>6} {:>20}",
        frm.frame_index,
        frm.stream_index,
        frm.media_type,
        frm.pts.map(|p| p.to_string()).unwrap_or("-".to_string()),
        frm.pts_time.as_deref().unwrap_or("-"),
        frm.pict_type.as_deref().unwrap_or("-"),
        info_str
    );
}

/// Stream packets/frames to stdout in text mode — prints each line immediately.
fn stream_packets_and_frames_text(
    demuxer: &mut dyn Demuxer,
    streams: &[StreamInfo],
    time_bases: &[(u32, u32)],
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut packet_index = 0;
    let mut frame_index = 0;
    let mut output_count = 0;

    // Print column headers
    if show_packets {
        println!();
        println!("Packets:");
        println!(
            "{:>6} {:>6} {:>8} {:>12} {:>12} {:>12} {:>8} {:>4}",
            "PKT", "STREAM", "TYPE", "PTS", "PTS_TIME", "DTS", "SIZE", "KEY"
        );
    }
    if show_frames {
        println!();
        println!("Frames:");
        println!(
            "{:>6} {:>6} {:>8} {:>12} {:>12} {:>6} {:>20}",
            "FRAME", "STREAM", "TYPE", "PTS", "PTS_TIME", "PICT", "INFO"
        );
    }

    let mut decoders: Vec<Option<Box<dyn DecoderWrapper>>> = if show_frames {
        streams.iter().map(|s| create_decoder(s)).collect()
    } else {
        vec![]
    };

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                let stream_idx = packet.stream_index();

                if let Some(filter) = stream_filter {
                    if stream_idx != filter {
                        continue;
                    }
                }

                let time_base = time_bases.get(stream_idx).copied().unwrap_or((1, 1000000));

                if show_packets {
                    let pkt_info = PacketInfo {
                        packet_index,
                        stream_index: stream_idx,
                        media_type: format!("{:?}", packet.media_type()).to_lowercase(),
                        pts: packet.pts(),
                        pts_time: packet.pts().map(|p| format_time(p, time_base)),
                        dts: packet.dts(),
                        dts_time: packet.dts().map(|d| format_time(d, time_base)),
                        duration: packet.duration(),
                        size: packet.size(),
                        is_keyframe: packet.is_keyframe(),
                    };
                    print_text_packet(&pkt_info);
                    if !show_frames {
                        output_count += 1;
                    }
                }

                if show_frames {
                    if let Some(Some(decoder)) = decoders.get_mut(stream_idx) {
                        if decoder.send_packet(&packet).is_ok() {
                            while let Ok(frame) = decoder.receive_frame() {
                                let frame_info = FrameInfo {
                                    frame_index,
                                    stream_index: stream_idx,
                                    media_type: format!("{:?}", frame.media_type()).to_lowercase(),
                                    pts: frame.pts(),
                                    pts_time: frame.pts().map(|p| format_time(p, time_base)),
                                    is_keyframe: frame.is_keyframe(),
                                    pict_type: Some(if frame.is_keyframe() {
                                        "I".to_string()
                                    } else {
                                        "P".to_string()
                                    }),
                                    params: frame_to_params_json(&frame),
                                };
                                print_text_frame(&frame_info);
                                frame_index += 1;
                                output_count += 1;

                                if count > 0 && output_count >= count {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                packet_index += 1;

                if count > 0 && output_count >= count {
                    break;
                }
            }
            Err(rust_media::Error::EndOfStream) => break,
            Err(_) => break,
        }
    }

    // Flush decoders
    if show_frames {
        for (stream_idx, decoder_opt) in decoders.iter_mut().enumerate() {
            if let Some(decoder) = decoder_opt {
                let _ = decoder.flush();
                let time_base = time_bases.get(stream_idx).copied().unwrap_or((1, 1000000));
                while let Ok(frame) = decoder.receive_frame() {
                    let frame_info = FrameInfo {
                        frame_index,
                        stream_index: stream_idx,
                        media_type: format!("{:?}", frame.media_type()).to_lowercase(),
                        pts: frame.pts(),
                        pts_time: frame.pts().map(|p| format_time(p, time_base)),
                        is_keyframe: frame.is_keyframe(),
                        pict_type: Some(if frame.is_keyframe() {
                            "I".to_string()
                        } else {
                            "P".to_string()
                        }),
                        params: frame_to_params_json(&frame),
                    };
                    print_text_frame(&frame_info);
                    frame_index += 1;
                    output_count += 1;

                    if count > 0 && output_count >= count {
                        return Ok(());
                    }
                }
            }
        }
    }

    Ok(())
}


// ============================================================================
// SSIM helper functions (CLI-specific: demuxer opening, stream finding)
// ============================================================================

/// Open a demuxer for the given file
fn open_demuxer_for_ssim(
    path: &std::path::Path,
) -> Result<Box<dyn Demuxer>, Box<dyn std::error::Error>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "mp4" | "m4a" | "m4v" | "mov" => {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(Box::new(Mp4Demuxer::new(reader)?))
        }
        "webm" => {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(Box::new(WebmDemuxer::open(reader)?))
        }
        "wav" => {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(Box::new(WavDemuxer::open(reader)?))
        }
        _ => Err(format!("Unsupported format: {}", ext).into()),
    }
}

/// Find the first video stream in a demuxer
fn find_video_stream(
    streams: &[StreamInfo],
    stream_index: Option<usize>,
) -> Option<&StreamInfo> {
    if let Some(idx) = stream_index {
        streams.get(idx).filter(|s| {
            matches!(s.params, StreamParams::Video(_))
        })
    } else {
        streams.iter().find(|s| {
            matches!(s.params, StreamParams::Video(_))
        })
    }
}

