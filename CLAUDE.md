# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`rust_media` is an ambitious project to build a Rust equivalent of FFmpeg - a comprehensive media processing framework and CLI tool. The project emphasizes clean architecture, performance, and modern codec support.

## Workspace Structure

The project uses a **Cargo workspace** with multiple crates for modularity and separation of concerns:

```
rust_media/
├── Cargo.toml              # Workspace root
├── Cargo.lock              # Shared lock file
├── CLAUDE.md
├── README.md
├── crates/
│   ├── rust_media_core/    # Core types and traits
│   │   ├── Packet, Frame
│   │   ├── Demuxer, Decoder, Encoder, Muxer traits
│   │   └── Error types, stream metadata
│   │
│   ├── rust_media_format/  # Container format implementations
│   │   ├── MP4/MOV demuxer/muxer
│   │   ├── MKV demuxer/muxer
│   │   └── WebM demuxer/muxer
│   │
│   ├── rust_media_codec/   # Codec implementations
│   │   ├── Video: H.264, VP9, AV1 (planned)
│   │   └── Audio: PCM ✅, Opus ✅, AAC (planned)
│   │
│   ├── rust_media_filter/  # Filter implementations (planned)
│   │   ├── Video: scale, crop, overlay, rotate
│   │   └── Audio: resample, mix, volume
│   │
│   ├── rust_media/         # Main library (re-exports)
│   │   └── Convenience crate that re-exports all components
│   │
│   └── rust_media_cli/     # CLI binary
│       └── Command-line tool
│
└── target/                 # Shared build directory
```

### Crate Responsibilities

- **rust_media_core**: Foundation - all other crates depend on this
- **rust_media_format**: Implements Demuxer and Muxer traits for containers
- **rust_media_codec**: Implements Decoder and Encoder traits for codecs
- **rust_media_filter**: Implements Filter trait for frame processing (planned)
- **rust_media**: Main public API - users typically only need this
- **rust_media_cli**: Binary executable for command-line usage

### Benefits of This Structure

1. **Modularity**: Each component is independently testable
2. **Compile times**: Only rebuild what changes
3. **Optional dependencies**: Users can depend on just the parts they need
4. **Clear separation**: Mirrors FFmpeg's architecture (libavformat, libavcodec, etc.)
5. **Parallel development**: Different crates can be worked on independently

## Common Commands

### Build and Run
```bash
# Build the entire workspace
cargo build

# Build in release mode (use for performance testing)
cargo build --release

# Build a specific crate
cargo build -p rust_media_core
cargo build -p rust_media_codec

# Run the CLI tool
cargo run -p rust_media_cli

# Run the CLI with arguments
cargo run -p rust_media_cli -- [args]
```

### Testing
```bash
# Run all tests in the workspace
cargo test

# Run tests for a specific crate
cargo test -p rust_media_core
cargo test -p rust_media_codec

# Run integration tests only
cargo test --test '*'

# Run a specific test
cargo test test_name

# Run tests with output shown
cargo test -- --nocapture

# Run benchmarks
cargo bench
```

### Code Quality
```bash
# Check all crates without building
cargo check --workspace

# Check a specific crate
cargo check -p rust_media_core

# Format all code
cargo fmt --all

# Lint all crates
cargo clippy --workspace

# Lint with all warnings
cargo clippy --workspace -- -W clippy::all
```

## Architecture Overview

### Core Data Structures

The project is built around two fundamental abstractions:

- **Packet**: Container-level data structure for compressed/encoded media data. Packets are produced by demuxers (which read container formats) and consumed by decoders (which decompress the codec data).
- **Frame**: Bit-level data structure for raw, decoded media data (video frames, audio samples). Frames are produced by decoders and can be processed by filters or consumed by encoders.

Both structures support:
- Image/Video: Single frames or sequences of frames
- Audio: Audio samples and buffers

### Container vs Codec Distinction

**Critical**: Maintain a clear separation between container formats and codecs:

- **Container Formats** (handled by demuxers/muxers): MP4, MOV, MKV, WebM
  - Containers wrap compressed media streams and provide metadata, timing, and stream multiplexing
  - A demuxer reads a container and produces Packets
  - A muxer takes Packets and writes a container

- **Codecs** (handled by decoders/encoders):
  - **Video codecs**: H.264, H.265/HEVC, VP8, VP9, AV1, MPEG-4, MPEG-2
  - **Audio codecs**: AAC (LC, HE, HEv2), Opus, MP3, Vorbis, FLAC
  - **Image codecs**: JPEG, PNG, HEIC, AVIF
  - A decoder takes Packets and produces Frames
  - An encoder takes Frames and produces Packets

### Development Milestones

The project follows a phased development approach:

#### 1. Core Data Structures (Foundation) ✅ COMPLETED
Implement Packet and Frame abstractions with support for various media types. This forms the foundation for all subsequent work.

**Status**: Core types (Packet, Frame) and all traits (Demuxer, Decoder, Encoder, Muxer) are implemented in `rust_media_core`.

#### 2. Integration Test Framework 🚧 PLANNED
Build comprehensive testing infrastructure including:
- Demuxing/decoding validation
- Encoding/muxing validation
- Quality metrics (SSIM for video, others for audio)
- Performance benchmarking
- Reference file generation and comparison

#### 3. Container Format and Codec Support 🚧 IN PROGRESS

**Status**: Workspace structure created with `rust_media_format` and `rust_media_codec` crates. Implementations needed.

**Container Formats** (demuxers/muxers):
- ✅ **WAV** (RIFF WAVE): PCM audio demuxer **IMPLEMENTED** in `rust_media_format/src/wav/demuxer.rs`
- MP4, MOV (ISO Base Media File Format)
- MKV (Matroska)
- WebM (Matroska subset)

**Video Codecs** (decoders/encoders) - Priority:
- H.264/AVC
- VP9
- AV1

**Audio Codecs** (decoders/encoders) - Priority:
- ✅ **PCM** (Pulse Code Modulation): Raw uncompressed audio - fundamental for all audio processing. **IMPLEMENTED** in `rust_media_codec/src/audio/pcm.rs`
- ✅ **Opus**: Lossy codec for speech and music, optimized for low-latency transmission. **IMPLEMENTED** in `rust_media_codec/src/audio/opus.rs`
  - Requires: libopus (install via `brew install opus` on macOS, `apt-get install libopus-dev` on Linux)
  - Supports: 8, 12, 16, 24, 48 kHz sample rates, mono and stereo
- AAC: LC, HE (AAC+), HEv2 (AAC++ / eAAC+)

**Image Codecs** (for thumbnails, still images) - Priority:
- JPEG
- PNG
- HEIC
- AVIF

**Possible Future Codec Support** (depending on requirements):
- **H.265/HEVC**: Modern successor to H.264, better compression but more complex
- **VP8**: Predecessor to VP9, also used in WebP image format
- **MP3**: Legacy audio codec
- **Vorbis**: Open audio codec (used in WebM)
- **FLAC**: Lossless audio codec

#### 4. Color Space Handling 🚧 PLANNED
Support for color conversions and bit-depth transformations:
- 8-bit ↔ 10-bit conversions
- YUV ↔ RGB conversions
- Color space metadata handling

#### 5. Filter Graph System 🚧 PLANNED
Build a flexible filtering system that:
- Supports composable filter chains
- Allows CLI-based filter graph construction
- Handles video and audio processing
- Enables common operations (scaling, cropping, mixing, etc.)

**Status**: Planned for future development after core codec support is implemented.

**Key Requirements**:
- **Streaming API**: Filters must operate on frames incrementally (send/receive pattern)
- **Graph construction**: Support both programmatic and CLI-based filter graph creation
- **Zero-copy where possible**: Minimize frame copying in filter chains
- **Common filters**: Scale, crop, overlay, rotate, format conversion, audio mixing, resampling

#### 6. FFmpeg CLI Compatibility 🚧 PLANNED
Develop tooling to convert FFmpeg CLI commands to rust_media equivalents, easing migration and adoption.

## Design Philosophy

- **Performance**: Leverage Rust's zero-cost abstractions and memory safety
- **Streaming Architecture**: All APIs (demuxer, decoder, encoder, muxer) must be streaming-based
  - Process data incrementally without loading entire files into memory
  - Enable real-time processing and low-latency pipelines
  - Support arbitrarily large media files (multi-GB videos)
  - Demuxers yield packets one at a time, decoders yield frames one at a time
- **Modularity**: Each component (demuxer, decoder, filter, encoder, muxer) should be independently testable
- **Correctness**: Validate output quality using metrics like SSIM, not just successful execution
- **Modern Codecs**: Prioritize contemporary formats (AV1, VP9) alongside legacy support (H.264)

## Key Implementation Considerations

- **All APIs must be streaming-based**:
  - Demuxers: `read_packet()` returns one packet at a time, never loading the entire file
  - Decoders: `send_packet()` / `receive_frame()` pattern for incremental processing
  - Encoders: `send_frame()` / `receive_packet()` pattern (symmetric to decoders)
  - Muxers: `write_packet()` writes packets incrementally to output
  - This enables processing of arbitrarily large files with bounded memory usage
  - Supports real-time streaming use cases (live transcoding, streaming servers)

- Packet and Frame should be zero-copy where possible

- **Container format handling (demuxers/muxers) must be separate from codec implementation (decoders/encoders)**
  - Example: MP4 is a container, H.264 is a codec. An MP4 file can contain H.264, H.265, or other video codecs
  - The demuxer reads the MP4 container and extracts H.264 packets
  - The decoder then decodes the H.264 packets into raw frames

- **Streaming Pipeline Architecture**:
  ```
  Input File → Demuxer → Packets → Decoder → Frames → Filter → Frames → Encoder → Packets → Muxer → Output File
  ```
  - Each stage processes data incrementally
  - Bounded memory usage regardless of input file size
  - Data flows through the pipeline without buffering entire streams

- **PCM audio codec is the foundation** for all audio processing:
  - All compressed audio codecs (AAC, Opus, etc.) decode to PCM format
  - PCM is the format that audio filters operate on
  - PCM is essential for validation (comparing decoded output against reference)
  - PCM encoder/decoder is trivial (mostly pass-through) making it ideal for testing the pipeline

- **Opus codec is implemented** as the first compressed audio codec:
  - Uses libopus via the audiopus crate (requires system libopus)
  - Excellent for low-latency VoIP, streaming, and music
  - Provides compression ratios of ~10x with good quality
  - See `examples/opus_example.rs` for usage demonstration

- Filter graph should support both programmatic and CLI-based construction

- Integration tests should include both correctness (quality metrics) and performance (benchmarks)
