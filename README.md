# rust_media

A Rust-based media processing framework - an FFmpeg equivalent with streaming APIs.

## Overview

`rust_media` is a comprehensive media processing framework that provides:
- **Streaming APIs** for bounded memory usage with arbitrarily large files
- **Container format** support (WAV ✅, WebM ✅, MKV ✅ demuxer, MP4 ✅ demuxer + muxer)
- **Codec** support (PCM ✅, Opus ✅, Vorbis ✅ decode, FLAC ✅ decode, MP3 ✅ decode, VP8 ✅, VP9 ✅, H.264 ✅, H.265/HEVC ✅ decode, AV1 ✅ decode + encode, AAC ✅)
- **Filter** support (audio resampling ✅, volume ✅, video scale ✅, crop ✅, SSIM quality metric ✅)
- **10-bit pixel format** support end-to-end (decode, scale, crop, with auto 10→8 conversion at encoder boundary)
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

# Run CLI smoke tests (exit codes, error handling, stderr/stdout routing)
cargo test -p rust_media_cli --test cli_smoke_tests

# Run codec round-trip tests (uses generated sine waves and color ramps,
# no binary fixtures needed)
cargo test -p rust_media_codec --test opus_roundtrip_test
cargo test -p rust_media_codec --test vp8_roundtrip_test
cargo test -p rust_media_codec --test vp9_roundtrip_test

# Run end-to-end MP4 pipeline test (encode → mux → demux → decode → verify)
cargo test -p rust_media_format --test mp4_roundtrip_test
```

The codec round-trip and MP4 pipeline tests **generate test content in pure
Rust** (sine waves, color ramps, gradients) and verify the full pipeline
without any committed binary fixtures or external `ffmpeg` dependency.

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
- ✅ **WebM** demuxer + muxer (VP8/VP9/AV1 video, Opus/Vorbis audio)
- ✅ **MKV** (Matroska) demuxer (H.264, H.265/HEVC, VP8/VP9/AV1 video; AAC, MP3, FLAC, AC3, Opus, Vorbis, PCM audio) - shares EBML parser with WebM
- ✅ **MP4** demuxer + muxer (H.264, H.265/HEVC, VP9 video; AAC, MP3, Opus audio)

### Codecs
- ✅ **PCM** codec (decoder + encoder)
- ✅ **Opus** codec (decoder + encoder)
- ✅ **Vorbis** decoder (via lewton, pure Rust, BSD-3-Clause)
- ✅ **FLAC** decoder (via claxon, pure Rust, Apache-2.0)
- ✅ **MP3** decoder (via minimp3, MIT)
- ✅ **VP8** codec (decoder + encoder via libvpx, configurable via `Vp8EncoderConfig`)
- ✅ **VP9** codec (decoder + encoder via libvpx, 8-bit + 10-bit Profile 2, configurable via `Vp9EncoderConfig`)
- ✅ **H.264** decoder (via rust_h264, pure Rust, MIT/Apache-2.0) - *Baseline, Main, High profiles*
- ✅ **H.264** decoder (via VideoToolbox, requires `videotoolbox` feature) - *All profiles, macOS only*
- ✅ **H.264** encoder (via x264, requires `gpl-x264` feature, configurable via `X264EncoderConfig`)
- ✅ **H.265/HEVC** decoder (via VideoToolbox, requires `videotoolbox` feature) - *macOS only, hardware accelerated, 8-bit + 10-bit (Main 10 / HDR)*
- ✅ **AV1** decoder (via dav1d, BSD-2-Clause, default build) - *Main + Main 10 profiles (8-bit + 10-bit)*
- ✅ **AV1** encoder (via rav1e, BSD-2-Clause, default build) - *8-bit and 10-bit YUV420; configurable via `Av1EncoderConfig`*
- ✅ **AAC** codec (encoder + decoder via libfdk-aac, requires `fdk-aac` feature)

### Filters
- ✅ **aresample** - Audio resampling (sinc interpolation via rubato)
- ✅ **volume** - Audio volume/gain adjustment (linear or dB)
- ✅ **scale** - Bilinear video scaling (8-bit and 10-bit; supports `scale=W:H`, `scale=W:-1`, `scale=W:-2` aspect-ratio markers)
- ✅ **crop** - Video cropping (8-bit and 10-bit; centered or explicit offset)
- ✅ **ssim** - Video SSIM quality comparison
- ✅ Auto-resample when encoder requires a different sample rate
- ✅ Auto 10→8 bit conversion at encoder boundary (YUV420P10LE → YUV420P)

### Pixel Formats
- ✅ **YUV420P** (8-bit, I420) - the default for most workflows
- ✅ **YUV420P10LE** (10-bit, little-endian) - for HDR / Main 10 content
- 📋 12-bit (Profile 2) - rejected with explicit "unimplemented" error
- ✅ Bit depth correctly parsed from `av1C` (AV1) and `hvcC` (HEVC) config records, surfaced via `info`

### CLI Tool
- ✅ **info** - Analyze media files (similar to ffprobe)
  - Stream information display
  - Packet analysis with timing and sizes
  - Frame analysis with decoding
  - JSON and text output formats
- ✅ **transform** - Transcode media files (similar to ffmpeg)
  - Video transcoding (VP8, VP9, H.264, AV1)
  - Audio transcoding (Opus, AAC, PCM)
  - Audio decoding (Vorbis, FLAC, MP3, AAC, Opus, PCM)
  - Stream copy (passthrough)
  - Bitrate control
  - Encoder options: `--speed`, `--qp`, `-g` (GOP size), `--keyint-min`, `--tile-columns`, `--tile-rows` — work for all video encoders
  - Audio filters (`--af`): resampling, volume (`--af "volume=0.5,aresample=48000"`)
  - Video filters (`--vf`): scale, crop, SSIM quality comparison (`--vf "crop=320:240,scale=160:120"`)
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

**Note**: The `videotoolbox` feature enables hardware-accelerated H.264 and H.265/HEVC decoding on macOS with support for all profiles. HEVC Main 10 is fully supported: 10-bit content is delivered as `YUV420P10LE` via the documented `'x420'` (P010) pixel format, then auto-downconverted at the encoder boundary if the target encoder is 8-bit only.

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
