# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`rust_media` is an ambitious project to build a Rust equivalent of FFmpeg - a comprehensive media processing framework and CLI tool. The project emphasizes clean architecture, performance, and modern codec support.

## Licensing Strategy

The project is **MIT licensed** by default, prioritizing permissively-licensed codecs. However, certain industry-standard codecs require GPL libraries, which we support through **optional Cargo features**.

### Default (MIT License)

All codecs in the default build use permissively-licensed libraries (MIT, BSD, Apache-2.0):
- **Video**: VP8 ✅ (libvpx/BSD-3-Clause), VP9 (libvpx/BSD-3-Clause), AV1 (dav1d, rav1e/BSD-MIT)
- **Audio**: PCM ✅, Opus ✅ (libopus/BSD-3-Clause), Vorbis (libvorbis/BSD), FLAC (libflac/BSD)

### Optional GPL Features

Industry-standard codecs that require GPL libraries are available through opt-in Cargo features:

```toml
[features]
default = []
gpl-x264 = ["x264"]  # Enables H.264 encoding via x264 (GPL v2+)
# Future: gpl-x265, gpl-xvid, etc.

[dependencies]
x264 = { version = "...", optional = true }
```

**IMPORTANT**: When GPL features are enabled:
- The compiled binary becomes **GPL-licensed** (copyleft applies)
- Users must comply with GPL requirements for distribution
- Clearly document in README and build output which features change the license

### Codec Licensing Reference

| Codec | Library | License | Feature Flag | Status |
|-------|---------|---------|--------------|--------|
| VP8 | libvpx (vpx-rs) | BSD-3-Clause | *(default)* | ✅ Implemented |
| VP9 | libvpx | BSD-3-Clause | *(default)* | Planned |
| AV1 | dav1d/rav1e | BSD/MIT | *(default)* | Planned |
| H.264 (decode) | OpenH264 | BSD-2-Clause | *(default)* | Planned |
| H.264 (encode) | x264 | **GPL v2+** | `gpl-x264` | Planned |
| H.265/HEVC | x265 | **GPL v2+** | `gpl-x265` | Future |
| Opus | libopus | BSD-3-Clause | *(default)* | ✅ Implemented |
| PCM | *(native)* | N/A | *(default)* | ✅ Implemented |

### Why This Approach?

1. **User Choice**: Users decide their licensing requirements
2. **Industry Standard**: x264 is the gold standard for H.264 encoding quality
3. **Clear Separation**: Default build stays MIT, optional features are clearly marked GPL
4. **Legal Compliance**: No hidden GPL dependencies in default build

### Documentation Requirements

When implementing GPL-licensed codecs:

1. **Feature flag naming**: Use `gpl-` prefix (e.g., `gpl-x264`, `gpl-x265`)
2. **README.md**: Document which features change the license
3. **Build warnings**: Print license notice when GPL features are enabled
4. **Crate metadata**: Update `Cargo.toml` license field when GPL features are used

Example build warning:
```rust
#[cfg(feature = "gpl-x264")]
compile_warning!("GPL feature 'gpl-x264' enabled - binary is now GPL v2+ licensed");
```

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
│   │   ├── Video: VP8 ✅, H.264 (planned), VP9 (planned), AV1 (planned)
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

- **Container Formats** (handled by demuxers/muxers): MP4, MOV, MKV, WebM, WAV
  - Containers wrap compressed media streams and provide metadata, timing, and stream multiplexing
  - A demuxer reads a container and produces Packets
  - A muxer takes Packets and writes a container
  - Example: WebM container can hold VP8, VP9, or AV1 video streams with Opus or Vorbis audio

- **Codecs** (handled by decoders/encoders):
  - **Video codecs**: VP8 ✅, H.264, H.265/HEVC, VP9, AV1, MPEG-4, MPEG-2
  - **Audio codecs**: PCM ✅, Opus ✅, AAC (LC, HE, HEv2), MP3, Vorbis, FLAC
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
- ✅ **WAV** (RIFF WAVE): PCM audio demuxer/muxer **IMPLEMENTED** in `rust_media_format/src/wav/`
- ✅ **WebM** (Matroska subset): VP8/VP9/Opus demuxer/muxer **IMPLEMENTED** in `rust_media_format/src/webm/`
  - Supports VP8, VP9, and Opus codecs
  - Streaming API with incremental packet processing
  - Efficient: ~50% less overhead than FFmpeg output (363 bytes vs 711 bytes)
  - See `examples/webm_remux.rs` for usage
- MP4, MOV (ISO Base Media File Format)
- MKV (Matroska)

**Video Codecs** (decoders/encoders) - Priority:
- ✅ **VP8**: Google's open video codec **IMPLEMENTED** in `rust_media_codec/src/video/vp8.rs`
  - Uses libvpx via vpx-rs bindings (version 0.2.1)
  - Decoder: Fully functional with YUV420P (I420) output
  - Encoder: Functional with limited configuration options (see limitations below)
  - See `examples/test_vp8_codec.rs` for decode/encode roundtrip example
- **H.264/AVC** (planned)
  - **Decoder**: OpenH264 (BSD-2-Clause) - default, MIT-compatible
  - **Encoder**: x264 (GPL v2+) - optional `gpl-x264` feature
  - Most widely deployed video codec, industry standard for compatibility
- **VP9** (planned)
  - Uses libvpx (BSD-3-Clause) - default, MIT-compatible
  - Successor to VP8, better compression efficiency
- **AV1** (planned)
  - Uses dav1d (decoder) and rav1e (encoder) - BSD/MIT licensed
  - Modern codec with best compression, royalty-free

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
- **Licensing Strategy**: MIT by default, with optional GPL features for industry-standard codecs
  - Default build uses only permissively-licensed libraries (BSD, MIT, Apache-2.0)
  - GPL-licensed codecs (x264, x265) available through opt-in Cargo features
  - Clear documentation of licensing implications for each feature

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
  - Example 1: MP4 is a container, H.264 is a codec. An MP4 file can contain H.264, H.265, or other video codecs
    - The demuxer reads the MP4 container and extracts H.264 packets
    - The decoder then decodes the H.264 packets into raw frames
  - Example 2: WebM is a container, VP8/VP9 are codecs. A WebM file can contain VP8 or VP9 video with Opus/Vorbis audio
    - The WebM demuxer reads the container and extracts VP8 packets
    - The VP8 decoder converts compressed packets into YUV420P frames

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

- **VP8 codec is implemented** for video encoding and decoding:
  - Uses libvpx via the vpx-rs crate (version 0.2.1)
  - Decoder: Fully functional with complete libvpx feature support
  - Encoder: Functional but with limited configuration options
  - See `examples/test_vp8_codec.rs` for usage demonstration

### VP8 Encoder Current Limitations

The VP8 encoder currently has hardcoded values for many settings that should be configurable:

**Settings extracted from StreamInfo** (configurable):
- Codec identifier (must be "vp8")
- Video dimensions (width × height)
- Bitrate (default: 1 Mbps if not specified)
- Timebase (numerator/denominator)

**Hardcoded settings** (not yet configurable):
- **Rate Control**: Hardcoded to Variable Bitrate (VBR)
  - Should support: Constant Bitrate (CBR), Constant Quality (CQ), Quantizer (Q) modes
- **Encoding Deadline**: Hardcoded to `GoodQuality` (balanced speed/quality)
  - Should support: `BestQuality` (slowest), `Realtime` (fastest)
- **GOP Size**: Uses libvpx default (automatic keyframe placement)
  - Should expose: GOP size configuration for predictable keyframe intervals
- **Keyframe Interval**: Uses libvpx default (~128-256 frames)
  - Should expose: Max keyframe distance control
- **Quality Range**: Uses libvpx defaults (min_q=4, max_q=63)
  - Should expose: Min/max quantizer control for quality tuning
- **Frame Duration**: Hardcoded to 1 timebase unit
  - Should derive: From frame rate automatically
- **Frame Flags**: Cannot force keyframes on specific frames
  - Should support: `FORCE_KEYFRAME` for scene changes, seek points
- **Threading**: Uses libvpx auto-detection
  - Should expose: Thread count configuration
- **Error Resilience**: Not configured
  - Should expose: Partition count for error resilience

**Future Enhancement Plan**:

Three approaches are documented in `rust_media_codec/src/video/vp8.rs`:

1. **Option 1: Extend StreamInfo** - Add generic key-value encoder config to StreamInfo
2. **Option 2: Codec-Specific Config Struct** (recommended) - Create `Vp8EncoderConfig` with all settings
3. **Option 3: Builder Pattern** (recommended) - `Vp8Encoder::builder().rate_control(...).gop_size(...).build()`

Options 2 and 3 are preferred for type-safety, clear documentation, and codec-specific feature support without polluting the generic StreamInfo structure.

See detailed documentation in `crates/rust_media_codec/src/video/vp8.rs` for comprehensive information about available vpx-rs settings not yet exposed and implementation details.

- Filter graph should support both programmatic and CLI-based construction

- Integration tests should include both correctness (quality metrics) and performance (benchmarks)

- **Implementing GPL-licensed codecs**:
  - Always use optional Cargo features with `gpl-` prefix (e.g., `gpl-x264`)
  - Gate GPL dependencies behind feature flags
  - Document licensing implications clearly in README and rustdoc
  - Consider adding compile-time warnings when GPL features are enabled
  - Keep GPL code isolated in separate modules for clarity
  - Example structure:
    ```rust
    #[cfg(feature = "gpl-x264")]
    pub mod x264;

    #[cfg(feature = "gpl-x264")]
    pub use x264::{X264Encoder, X264Decoder};
    ```
