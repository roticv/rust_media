# rust_media

A Rust-based media processing framework - an FFmpeg equivalent with streaming APIs.

## Overview

`rust_media` is a comprehensive media processing framework that provides:
- **Streaming APIs** for bounded memory usage with arbitrarily large files
- **Container format** support (WAV ✅, WebM ✅, MP4 ✅ demuxer + muxer, MKV planned)
- **Codec** support (PCM ✅, Opus ✅, MP3 ✅ decode, VP8 ✅, VP9 ✅, H.264 ✅, H.265/HEVC ✅ decode, AAC ✅; AV1 planned)
- **Filter** support (audio resampling ✅, volume ✅, SSIM quality metric ✅)
- **Format detection** from magic bytes with file extension fallback
- **Modular architecture** with separate crates for different components

## Project Structure

This is a Cargo workspace with multiple crates:

- **[rust_media_core](crates/rust_media_core)** - Core types (Packet, Frame) and traits (Demuxer, Decoder, Encoder, Muxer)
- **[rust_media_format](crates/rust_media_format)** - Container format implementations + format detection
- **[rust_media_codec](crates/rust_media_codec)** - Codec implementations
- **[rust_media_filter](crates/rust_media_filter)** - Filter implementations (resampling, SSIM, etc.)
- **[rust_media](crates/rust_media)** - Main library that re-exports all components
- **[rust_media_cli](crates/rust_media_cli)** - Command-line tool

## Quick Start

### Building

```bash
# Build the entire workspace
cargo build

# Build in release mode
cargo build --release

# Build a specific crate
cargo build -p rust_media_core
```

### Running

```bash
# Run the CLI tool
cargo run -p rust_media_cli
```

### Testing

```bash
# Run all tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p rust_media_core

# Run CLI integration tests (tests info and transform commands)
cargo test -p rust_media_cli --test integration_tests
```

## Architecture

The project uses a **streaming architecture** where all components process data incrementally:

```
Input File → Demuxer → Packets → Decoder → Frames → Filter → Frames → Encoder → Packets → Muxer → Output File
```

- **Demuxer**: Streams packets one at a time from containers
- **Decoder**: send/receive pattern for incremental decoding
- **Filter**: Processes frames incrementally (scale, crop, convert, etc.)
- **Encoder**: send/receive pattern for incremental encoding
- **Muxer**: Writes packets one at a time to containers

This enables:
- Processing of arbitrarily large files with bounded memory
- Real-time transcoding and streaming
- Low-latency pipelines

## Development Status

🚧 **Work in Progress** 🚧

This project is in active development. Current status:

### Core Infrastructure
- ✅ Core data structures (Packet, Frame)
- ✅ Trait definitions (Demuxer, Decoder, Encoder, Muxer)
- ✅ Workspace structure
- ✅ Format detection from magic bytes + file extension fallback

### Container Formats
- ✅ **WAV** demuxer + muxer (PCM audio)
- ✅ **WebM** demuxer + muxer (VP8/VP9 video, Opus audio)
- ✅ **MP4** demuxer + muxer (H.264, H.265/HEVC, VP9 video; AAC, MP3, Opus audio)
- 📋 MKV demuxer/muxer - Planned

### Codecs
- ✅ **PCM** codec (decoder + encoder)
- ✅ **Opus** codec (decoder + encoder)
- ✅ **MP3** decoder (via minimp3, MIT)
- ✅ **VP8** codec (decoder + encoder via libvpx)
- ✅ **VP9** codec (decoder + encoder via libvpx)
- ✅ **H.264** decoder (via rust_h264, pure Rust, MIT/Apache-2.0) - *Baseline, Main, High profiles*
- ✅ **H.264** decoder (via VideoToolbox, requires `videotoolbox` feature) - *All profiles, macOS only*
- ✅ **H.264** encoder (via x264, requires `gpl-x264` feature)
- ✅ **H.265/HEVC** decoder (via VideoToolbox, requires `videotoolbox` feature) - *macOS only, hardware accelerated*
- ✅ **AAC** codec (encoder + decoder via libfdk-aac, requires `fdk-aac` feature)
- 📋 AV1 codec - Planned

### Filters
- ✅ **aresample** - Audio resampling (sinc interpolation via rubato)
- ✅ **volume** - Audio volume/gain adjustment (linear or dB)
- ✅ **ssim** - Video SSIM quality comparison
- ✅ Auto-resample when encoder requires a different sample rate

### CLI Tool
- ✅ **info** - Analyze media files (similar to ffprobe)
  - Stream information display
  - Packet analysis with timing and sizes
  - Frame analysis with decoding
  - JSON and text output formats
- ✅ **transform** - Transcode media files (similar to ffmpeg)
  - Video transcoding (VP8, VP9, H.264)
  - Audio transcoding (Opus, AAC, PCM)
  - Stream copy (passthrough)
  - Bitrate control
  - Audio filters (`--af`): resampling, volume (`--af "volume=0.5,aresample=48000"`)
  - Video filters (`--vf`): SSIM quality comparison (`--vf ssim`)
  - Auto-resampling when encoder requires a different sample rate
  - ffmpeg-style progress output (`--progress`)
  - See [rust_media_cli documentation](crates/rust_media_cli/README.md) for details

## Optional Features

Some codecs require external libraries with specific licensing and are available through optional Cargo features:

| Feature | Codec | Library | License | Platform |
|---------|-------|---------|---------|----------|
| `gpl-x264` | H.264 encoder | x264 | GPL v2+ | All |
| `fdk-aac` | AAC encoder + decoder | libfdk-aac | Fraunhofer FDK AAC License | All |
| `videotoolbox` | H.264 + H.265/HEVC decoder (all profiles) | VideoToolbox | Apple | macOS only |

**Warning**: Enabling `gpl-x264` changes the license of compiled binaries to GPL.

**Note**: The `fdk-aac` feature uses the Fraunhofer FDK AAC License, which is more permissive than GPL but has some restrictions on use.

**Note**: The `videotoolbox` feature enables hardware-accelerated H.264 and H.265/HEVC decoding on macOS with support for all profiles (including Main 10 for HDR content).

```bash
# Build with H.264 encoder support (GPL)
cargo build --features gpl-x264

# Build with AAC encoder support
cargo build --features fdk-aac

# Build with VideoToolbox H.264/H.265 decoder (macOS, all profiles)
cargo build --features videotoolbox

# Build with both video and audio encoding
cargo build --features "gpl-x264 fdk-aac"

# Build without optional features (MIT/Apache-2.0)
cargo build
```

## Contributing

See [CLAUDE.md](CLAUDE.md) for detailed architectural guidance.

## License

MIT OR Apache-2.0 (default build)

When GPL features are enabled (e.g., `gpl-x264`), the compiled binary is licensed under GPL v2+.
