# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`rust_media` is an ambitious project to build a Rust equivalent of FFmpeg - a comprehensive media processing framework and CLI tool. The project emphasizes clean architecture, performance, and modern codec support.

## Licensing Strategy

The project is **MIT licensed** by default, prioritizing permissively-licensed codecs. However, certain industry-standard codecs require GPL libraries, which we support through **optional Cargo features**.

### Default (MIT License)

All codecs in the default build use permissively-licensed libraries (MIT, BSD, Apache-2.0):
- **Video**: VP8 ✅ (libvpx/BSD-3-Clause), VP9 ✅ (libvpx/BSD-3-Clause), H.264 decode ✅ (rust_h264/MIT), AV1 (dav1d, rav1e/BSD-MIT)
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
| VP9 | libvpx (vpx-rs) | BSD-3-Clause | *(default)* | ✅ Implemented |
| AV1 | dav1d/rav1e | BSD/MIT | *(default)* | Planned |
| H.264 (decode) | rust_h264 | MIT/Apache-2.0 | *(default)* | ✅ Implemented (Baseline, Main, High) |
| H.264 (decode) | VideoToolbox | Apple | `videotoolbox` | ✅ Implemented (all profiles, macOS) |
| H.264 (encode) | x264 | **GPL v2+** | `gpl-x264` | ✅ Implemented |
| H.265/HEVC (decode) | VideoToolbox | Apple | `videotoolbox` | ✅ Implemented (macOS, all profiles) |
| H.265/HEVC (encode) | x265 | **GPL v2+** | `gpl-x265` | Future |
| Opus | libopus | BSD-3-Clause | *(default)* | ✅ Implemented |
| MP3 (decode) | minimp3 | MIT | *(default)* | ✅ Implemented |
| AAC | libfdk-aac | FDK AAC License | `fdk-aac` | ✅ Implemented |
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
│   │   ├── WAV demuxer/muxer ✅
│   │   ├── WebM demuxer/muxer ✅
│   │   ├── MKV demuxer ✅ (shares EBML parser with WebM, supports H.264/H.265/AAC/MP3/etc.)
│   │   └── MP4 demuxer/muxer ✅
│   │
│   ├── rust_media_codec/   # Codec implementations
│   │   ├── Video: VP8 ✅, VP9 ✅, H.264 ✅ (rust_h264/VideoToolbox decode, x264 encode), H.265/HEVC ✅ (VideoToolbox decode), AV1 (planned)
│   │   └── Audio: PCM ✅, Opus ✅, MP3 ✅ (decode, minimp3), AAC ✅ (fdk-aac)
│   │
│   ├── rust_media_filter/  # Filter implementations
│   │   ├── Video: SSIM ✅; scale, crop, overlay, rotate (planned)
│   │   ├── Audio: resample ✅, volume ✅; mix (planned)
│   │   └── Filter graph parsing (FFmpeg-like syntax) ✅
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
- **rust_media_filter**: Filter graph parsing + filter implementations (resampling, volume, SSIM, etc.)
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
  - **Video codecs**: VP8 ✅, VP9 ✅, H.264 ✅ (decoder + encoder), H.265/HEVC ✅ (VideoToolbox decode), AV1, MPEG-4, MPEG-2
  - **Audio codecs**: PCM ✅, Opus ✅, AAC ✅ (decoder + encoder), MP3, Vorbis, FLAC
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
- ✅ **WebM** (Matroska subset): Demuxer/muxer **IMPLEMENTED** in `rust_media_format/src/webm/`
  - Supports VP8, VP9, AV1 video and Opus, Vorbis audio
  - Streaming API with incremental packet processing
  - Efficient: ~50% less overhead than FFmpeg output (363 bytes vs 711 bytes)
  - See `examples/webm_remux.rs` for usage
- ✅ **MKV** (Matroska): Demuxer **IMPLEMENTED** in `rust_media_format/src/mkv/` (re-exports `WebmDemuxer`)
  - Shares EBML parser with WebM (same container format, MKV is a superset)
  - Supports H.264 (V_MPEG4/ISO/AVC), H.265/HEVC (V_MPEGH/ISO/HEVC), VP8/VP9/AV1 video
  - Supports AAC (A_AAC), MP3 (A_MPEG/L3), FLAC (A_FLAC), AC3 (A_AC3), Opus, Vorbis, PCM audio
  - Parses CodecPrivate into StreamInfo.extra_data so decoders get codec config (avcC, hvcC, AudioSpecificConfig)
  - Distinguishes WebM from MKV via EBML DocType field (reports format_name as "webm" or "matroska")
  - No muxer yet (use WebM muxer for VP8/VP9/Opus content)
- ✅ **MP4** (ISO Base Media File Format): Demuxer + Muxer **IMPLEMENTED** in `rust_media_format/src/mp4/`
  - Supports H.264/AVC (avc1), H.265/HEVC (hvc1/hev1), VP9 (vp09), AAC (mp4a), MP3 (.mp3), and Opus audio
  - Muxer: Streaming API with incremental packet writing
  - Muxer: File structure: ftyp | mdat | moov (streaming-friendly)
  - Complete sample table support: stts, stsc, stsz, stco/co64, stss, ctts
  - Automatic 64-bit chunk offsets for files > 4GB
  - Demuxer: Parses moov box, builds sample table, sequential packet reading
  - Demuxer: Supports seeking by timestamp

**Video Codecs** (decoders/encoders) - Priority:
- ✅ **VP8**: Google's open video codec **IMPLEMENTED** in `rust_media_codec/src/video/vp8.rs`
  - Uses libvpx via vpx-rs bindings (version 0.2.1)
  - Decoder: Fully functional with YUV420P (I420) output
  - Encoder: Functional with limited configuration options (see limitations below)
  - See `crates/rust_media_codec/examples/test_vp8_codec.rs` for decode/encode roundtrip example
- ✅ **H.264/AVC** (decoder + encoder) **IMPLEMENTED**
  - **Decoder**: rust_h264 (MIT/Apache-2.0) in `rust_media_codec/src/video/h264.rs`
    - Uses rust_h264 crate (v0.2.0) - pure Rust H.264 decoder
    - Supports Baseline, Main, and High profiles
    - YUV420P (I420) output format
    - POC-based frame reordering for correct display order with B-frames
    - Supports both AVCC format (MP4) and Annex B format (raw H.264)
    - Automatic SPS/PPS extraction from AVCDecoderConfigurationRecord
  - **Encoder**: x264 (GPL v2+) in `rust_media_codec/src/video/x264.rs`
    - Requires `gpl-x264` feature flag
    - Uses x264 crate (v0.5.0) - safe Rust bindings to libx264
    - Requires system libx264 (`brew install x264` or `apt-get install libx264-dev`)
    - Profile: High (maximum quality/features)
    - Preset: Medium (balanced speed/quality)
    - Annex B output format for muxer compatibility
    - Full B-frame support with proper flush handling
    - Build with: `cargo build --features gpl-x264`
- ✅ **VP9**: Google's successor to VP8 **IMPLEMENTED** in `rust_media_codec/src/video/vp9.rs`
  - Uses libvpx via vpx-rs bindings (version 0.2.1)
  - Decoder: Fully functional with YUV420P (I420) output, 16-bit support planned
  - Encoder: Functional with limited configuration options (similar limitations to VP8)
  - 30-50% better compression than VP8 at same quality
  - Tile-based encoding for better parallelization
  - BSD-3-Clause licensed - default, MIT-compatible
  - See `crates/rust_media_codec/examples/test_vp9_codec.rs` for decode/encode roundtrip example
- ✅ **H.265/HEVC** (decoder) **IMPLEMENTED** via VideoToolbox
  - **Decoder**: VideoToolbox (macOS only) in `rust_media_codec/src/video/videotoolbox.rs`
    - Requires `videotoolbox` feature flag
    - Hardware accelerated on Apple Silicon
    - Supports Main, Main 10 (10-bit/HDR), and Main Still Picture profiles
    - Parses HEVCDecoderConfigurationRecord (hvcC) from MP4
    - YUV420P output (NV12 internally, converted to I420)
    - No software fallback available (macOS only)
- **AV1** (planned)
  - Uses dav1d (decoder) and rav1e (encoder) - BSD/MIT licensed
  - Modern codec with best compression, royalty-free

**Audio Codecs** (decoders/encoders) - Priority:
- ✅ **PCM** (Pulse Code Modulation): Raw uncompressed audio - fundamental for all audio processing. **IMPLEMENTED** in `rust_media_codec/src/audio/pcm.rs`
- ✅ **Opus**: Lossy codec for speech and music, optimized for low-latency transmission. **IMPLEMENTED** in `rust_media_codec/src/audio/opus.rs`
  - Requires: libopus (install via `brew install opus` on macOS, `apt-get install libopus-dev` on Linux)
  - Supports: 8, 12, 16, 24, 48 kHz sample rates, mono and stereo
- ✅ **MP3**: MP3 decoding via minimp3. **IMPLEMENTED** in `rust_media_codec/src/audio/mp3.rs`
  - Uses minimp3 crate (v0.6, MIT) - C library bindings
  - Streaming decoder: accumulates packet data for proper frame boundary detection
  - Supports variable bitrate (VBR) and constant bitrate (CBR)
  - Decode only (no encoder)
- ✅ **AAC**: AAC-LC encoding and decoding via libfdk-aac. **IMPLEMENTED** in `rust_media_codec/src/audio/fdk_aac.rs`
  - Requires: libfdk-aac (install via `brew install fdk-aac` on macOS, `apt-get install libfdk-aac-dev` on Linux)
  - Requires feature flag: `fdk-aac`
  - Encoder: Supports 8-96 kHz sample rates, mono and stereo, raw AAC output for MP4 muxing
  - Decoder: Supports ADTS-wrapped AAC and raw AAC with AudioSpecificConfig (for MP4)
  - License: Fraunhofer FDK AAC License (not GPL, but has some restrictions)

**Image Codecs** (for thumbnails, still images) - Priority:
- JPEG
- PNG
- HEIC
- AVIF

**Possible Future Codec Support** (depending on requirements):
- **H.265/HEVC encode**: Via x265 (GPL v2+), requires `gpl-x265` feature
- **Vorbis**: Open audio codec (used in WebM)
- **FLAC**: Lossless audio codec

#### 4. Color Space Handling 🚧 PLANNED
Support for color conversions and bit-depth transformations:
- 8-bit ↔ 10-bit conversions
- YUV ↔ RGB conversions
- Color space metadata handling

#### 5. Filter Graph System 🚧 IN PROGRESS
Build a flexible filtering system that:
- Supports composable filter chains
- Allows CLI-based filter graph construction
- Handles video and audio processing
- Enables common operations (scaling, cropping, mixing, etc.)

**Status**: Foundation implemented in `rust_media_filter` crate. Filter graph parsing, audio resampling (sinc), volume, and SSIM video quality metric are available. Auto-resampling on sample rate mismatch. More filters planned.

**Implemented**:
- **Filter graph parsing**: FFmpeg-like syntax (`filter_name=param1=value1:param2=value2`, comma-separated chains)
- **aresample**: Audio resampling via sinc interpolation (rubato, Kaiser-windowed polyphase filter)
- **volume**: Audio volume/gain adjustment (linear factor or dB, e.g., `volume=0.5`, `volume=6dB`)
- **ssim**: SSIM quality comparison between two video streams
- **Auto-resample**: Automatically resamples when encoder requires a different sample rate (e.g., MP3 44100 Hz → Opus 48000 Hz)
- **Format detection**: Magic bytes detection with file extension fallback (`rust_media_format::detect`)
- CLI integration via `--af` (audio filters) and `--vf` (video filters)

**Key Requirements** (for future filters):
- **Streaming API**: Filters must operate on frames incrementally (send/receive pattern)
- **Graph construction**: Support both programmatic and CLI-based filter graph creation
- **Zero-copy where possible**: Minimize frame copying in filter chains
- **Common filters**: Scale, crop, overlay, rotate, format conversion, audio mixing

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
