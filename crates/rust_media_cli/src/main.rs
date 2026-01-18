//! rust_media CLI - Command-line media processing tool
//!
//! A Rust-based media conversion and processing tool (FFmpeg equivalent)

use clap::{Parser, Subcommand, ValueEnum};
use rust_media::{Decoder, Demuxer, MediaType, StreamParams};
use rust_media_format::mp4::Mp4Demuxer;
use rust_media_format::wav::WavDemuxer;
use rust_media_format::webm::WebmDemuxer;
use serde::Serialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rust_media")]
#[command(author, version, about = "A Rust-based media processing framework", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
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
            if let Err(e) = run_info(&input, show_packets, show_frames, count, stream, output_format)
            {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

// ============================================================================
// Media info implementation
// ============================================================================

fn run_info(
    input: &PathBuf,
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
    output_format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let filename = input.to_string_lossy().to_string();

    // Detect format from extension
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Open the appropriate demuxer based on file extension
    let (format_name, streams, container_duration, container_bitrate, packets, frames) =
        match extension.as_str() {
            "mp4" | "m4a" | "m4v" | "mov" => {
                analyze_mp4(&filename, show_packets, show_frames, count, stream_filter)?
            }
            "webm" => analyze_webm(&filename, show_packets, show_frames, count, stream_filter)?,
            "wav" => analyze_wav(&filename, show_packets, show_frames, count, stream_filter)?,
            _ => {
                return Err(format!(
                    "Unsupported format: {}. Supported: mp4, m4a, m4v, mov, webm, wav",
                    extension
                )
                .into())
            }
        };

    // Build media info structure
    let media_info = MediaInfo {
        format: FormatInfo {
            filename: filename.clone(),
            format_name,
            duration_us: container_duration,
            duration: container_duration.map(|d| format_duration(d)),
            bitrate: container_bitrate,
            nb_streams: streams.len(),
        },
        streams,
        packets,
        frames,
    };

    // Output based on format
    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&media_info)?);
        }
        OutputFormat::Text => {
            print_text_output(&media_info);
        }
    }

    Ok(())
}

// ============================================================================
// Format-specific analyzers
// ============================================================================

fn analyze_mp4(
    filename: &str,
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
) -> Result<
    (
        String,
        Vec<StreamInfoJson>,
        Option<i64>,
        Option<u64>,
        Vec<PacketInfo>,
        Vec<FrameInfo>,
    ),
    Box<dyn std::error::Error>,
> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let mut demuxer = Mp4Demuxer::new(reader)?;

    let container = demuxer.container_info()?;
    let raw_streams = demuxer.streams()?;

    let streams: Vec<StreamInfoJson> = raw_streams
        .iter()
        .map(|s| stream_to_json(s))
        .collect();

    let mut packets = Vec::new();
    let mut frames = Vec::new();

    if show_packets || show_frames {
        let time_bases: Vec<(u32, u32)> = raw_streams.iter().map(|s| s.time_base).collect();
        let (p, f) = collect_packets_and_frames(
            &mut demuxer,
            &raw_streams,
            &time_bases,
            show_packets,
            show_frames,
            count,
            stream_filter,
        )?;
        packets = p;
        frames = f;
    }

    Ok((
        container.format_name,
        streams,
        container.duration,
        container.bitrate,
        packets,
        frames,
    ))
}

fn analyze_webm(
    filename: &str,
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
) -> Result<
    (
        String,
        Vec<StreamInfoJson>,
        Option<i64>,
        Option<u64>,
        Vec<PacketInfo>,
        Vec<FrameInfo>,
    ),
    Box<dyn std::error::Error>,
> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader)?;

    let container = demuxer.container_info()?;
    let raw_streams = demuxer.streams()?;

    let streams: Vec<StreamInfoJson> = raw_streams
        .iter()
        .map(|s| stream_to_json(s))
        .collect();

    let mut packets = Vec::new();
    let mut frames = Vec::new();

    if show_packets || show_frames {
        let time_bases: Vec<(u32, u32)> = raw_streams.iter().map(|s| s.time_base).collect();
        let (p, f) = collect_packets_and_frames(
            &mut demuxer,
            &raw_streams,
            &time_bases,
            show_packets,
            show_frames,
            count,
            stream_filter,
        )?;
        packets = p;
        frames = f;
    }

    Ok((
        container.format_name,
        streams,
        container.duration,
        container.bitrate,
        packets,
        frames,
    ))
}

fn analyze_wav(
    filename: &str,
    show_packets: bool,
    show_frames: bool,
    count: usize,
    stream_filter: Option<usize>,
) -> Result<
    (
        String,
        Vec<StreamInfoJson>,
        Option<i64>,
        Option<u64>,
        Vec<PacketInfo>,
        Vec<FrameInfo>,
    ),
    Box<dyn std::error::Error>,
> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let mut demuxer = WavDemuxer::open(reader)?;

    let container = demuxer.container_info()?;
    let raw_streams = demuxer.streams()?;

    let streams: Vec<StreamInfoJson> = raw_streams
        .iter()
        .map(|s| stream_to_json(s))
        .collect();

    let mut packets = Vec::new();
    let mut frames = Vec::new();

    if show_packets || show_frames {
        let time_bases: Vec<(u32, u32)> = raw_streams.iter().map(|s| s.time_base).collect();
        let (p, f) = collect_packets_and_frames(
            &mut demuxer,
            &raw_streams,
            &time_bases,
            show_packets,
            show_frames,
            count,
            stream_filter,
        )?;
        packets = p;
        frames = f;
    }

    Ok((
        container.format_name,
        streams,
        container.duration,
        container.bitrate,
        packets,
        frames,
    ))
}

// ============================================================================
// Packet and frame collection
// ============================================================================

fn collect_packets_and_frames<D: Demuxer>(
    demuxer: &mut D,
    streams: &[rust_media::StreamInfo],
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
        streams
            .iter()
            .map(|s| create_decoder(s))
            .collect()
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
                                        "P".to_string() // Simplified - actual B-frame detection requires codec-specific parsing
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
                let check_count = if show_frames { frames.len() } else { packets.len() };
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
// Decoder wrapper trait for dynamic dispatch
// ============================================================================

trait DecoderWrapper {
    fn send_packet(&mut self, packet: &rust_media::Packet) -> rust_media::Result<()>;
    fn receive_frame(&mut self) -> rust_media::Result<rust_media::Frame>;
    fn flush(&mut self) -> rust_media::Result<()>;
}

impl<D: Decoder> DecoderWrapper for D {
    fn send_packet(&mut self, packet: &rust_media::Packet) -> rust_media::Result<()> {
        Decoder::send_packet(self, packet)
    }

    fn receive_frame(&mut self) -> rust_media::Result<rust_media::Frame> {
        Decoder::receive_frame(self)
    }

    fn flush(&mut self) -> rust_media::Result<()> {
        Decoder::flush(self)
    }
}

fn create_decoder(stream: &rust_media::StreamInfo) -> Option<Box<dyn DecoderWrapper>> {
    match stream.codec.as_str() {
        "pcm" | "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" => {
            rust_media_codec::PcmDecoder::new(stream.clone())
                .ok()
                .map(|d| Box::new(d) as Box<dyn DecoderWrapper>)
        }
        "opus" => rust_media_codec::OpusDecoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "vp8" => rust_media_codec::Vp8Decoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        "vp9" => rust_media_codec::Vp9Decoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        #[cfg(feature = "fdk-aac")]
        "aac" => rust_media_codec::FdkAacDecoder::new(stream.clone())
            .ok()
            .map(|d| Box::new(d) as Box<dyn DecoderWrapper>),
        _ => None,
    }
}

// ============================================================================
// Helper functions
// ============================================================================

fn stream_to_json(stream: &rust_media::StreamInfo) -> StreamInfoJson {
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

fn frame_to_params_json(frame: &rust_media::Frame) -> FrameParamsJson {
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

fn print_text_output(info: &MediaInfo) {
    // Format section
    println!("Input: {}", info.format.filename);
    println!("  Format:     {}", info.format.format_name);
    if let Some(dur) = &info.format.duration {
        println!("  Duration:   {}", dur);
    }
    if let Some(br) = info.format.bitrate {
        println!("  Bitrate:    {} kb/s", br / 1000);
    }
    println!("  Streams:    {}", info.format.nb_streams);
    println!();

    // Streams section
    println!("Streams:");
    for stream in &info.streams {
        print!("  Stream #{}: {} ({})", stream.index, stream.media_type, stream.codec);

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

    // Packets section
    if !info.packets.is_empty() {
        println!();
        println!("Packets:");
        println!(
            "{:>6} {:>6} {:>8} {:>12} {:>12} {:>12} {:>8} {:>4}",
            "PKT", "STREAM", "TYPE", "PTS", "PTS_TIME", "DTS", "SIZE", "KEY"
        );
        for pkt in &info.packets {
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
    }

    // Frames section
    if !info.frames.is_empty() {
        println!();
        println!("Frames:");
        println!(
            "{:>6} {:>6} {:>8} {:>12} {:>12} {:>6} {:>20}",
            "FRAME", "STREAM", "TYPE", "PTS", "PTS_TIME", "PICT", "INFO"
        );
        for frm in &info.frames {
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
    }
}
