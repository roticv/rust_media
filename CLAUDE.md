# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`rust_media` is an ambitious project to build a Rust equivalent of FFmpeg - a comprehensive media processing framework and CLI tool. The project emphasizes clean architecture, performance, and modern codec support.

## Licensing Strategy

The project is **MIT licensed** by default, prioritizing permissively-licensed codecs. However, certain industry-standard codecs require GPL libraries, which we support through **optional Cargo features**.

### Default (MIT License)

All codecs in the default build use permissively-licensed libraries (MIT, BSD, Apache-2.0):
- **Video**: VP8 ✅ (libvpx/BSD-3-Clause), VP9 ✅ (libvpx/BSD-3-Clause), H.264 decode ✅ (rust_h264/MIT), AV1 decode ✅ (dav1d/BSD-2-Clause), AV1 encode ✅ (rav1e/BSD-2-Clause)
- **Audio**: PCM ✅, Opus ✅ (libopus/BSD-3-Clause), MP3 decode ✅ (minimp3/MIT), Vorbis decode ✅ (lewton/BSD-3-Clause); FLAC (claxon/Apache-2.0) planned

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
| VP8 | libvpx (vpx-rs) | BSD-3-Clause | *(default)* | ✅ Implemented (8-bit) |
| VP9 | libvpx (vpx-rs) | BSD-3-Clause | *(default)* | ✅ Implemented (8-bit; 10-bit Profile 2 planned) |
| AV1 (decode) | dav1d | BSD-2-Clause | *(default)* | ✅ Implemented (Main + Main 10, 8-bit + 10-bit) |
| AV1 (encode) | rav1e | BSD-2-Clause | *(default)* | ✅ Implemented (8-bit + 10-bit, YUV420) |
| H.264 (decode) | rust_h264 | MIT/Apache-2.0 | *(default)* | ✅ Implemented (Baseline, Main, High) |
| H.264 (decode) | VideoToolbox | Apple | `videotoolbox` | ✅ Implemented (all profiles, macOS) |
| H.264 (encode) | x264 | **GPL v2+** | `gpl-x264` | ✅ Implemented |
| H.265/HEVC (decode) | VideoToolbox | Apple | `videotoolbox` | ✅ Implemented (macOS, all profiles, 8-bit + 10-bit Main 10) |
| H.265/HEVC (encode) | x265 | **GPL v2+** | `gpl-x265` | Future |
| Opus | libopus | BSD-3-Clause | *(default)* | ✅ Implemented |
| MP3 (decode) | minimp3 | MIT | *(default)* | ✅ Implemented |
| AAC | libfdk-aac | FDK AAC License | `fdk-aac` | ✅ Implemented |
| Vorbis (decode) | lewton | BSD-3-Clause | *(default)* | ✅ Implemented |
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
│   │   ├── Video: VP8 ✅, VP9 ✅, H.264 ✅ (rust_h264/VideoToolbox decode, x264 encode), H.265/HEVC ✅ (VideoToolbox decode, 8/10-bit), AV1 ✅ (dav1d decode + rav1e encode, 8/10-bit)
│   │   └── Audio: PCM ✅, Opus ✅, MP3 ✅ (decode, minimp3), Vorbis ✅ (decode, lewton), AAC ✅ (fdk-aac)
│   │
│   ├── rust_media_filter/  # Filter implementations
│   │   ├── Video: scale ✅ (bilinear, 8-bit + 10-bit), crop ✅ (8-bit + 10-bit), SSIM ✅, format ✅ (10→8 bit); overlay, rotate (planned)
│   │   ├── Audio: resample ✅ (sinc/rubato), volume ✅; mix (planned)
│   │   └── Filter graph parsing (FFmpeg-like syntax, named + positional) ✅
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
  - **Audio codecs**: PCM ✅, Opus ✅, AAC ✅ (decoder + encoder), MP3 ✅ (decode), Vorbis ✅ (decode), FLAC
  - **Image codecs**: JPEG, PNG, HEIC, AVIF
  - A decoder takes Packets and produces Frames
  - An encoder takes Frames and produces Packets

### Development Milestones

The project follows a phased development approach:

#### 1. Core Data Structures (Foundation) ✅ COMPLETED
Implement Packet and Frame abstractions with support for various media types. This forms the foundation for all subsequent work.

**Status**: Core types (Packet, Frame) and all traits (Demuxer, Decoder, Encoder, Muxer) are implemented in `rust_media_core`.

#### 2. Integration Test Framework 🚧 IN PROGRESS

**Status**: Foundation in place. Round-trip integration tests for major codecs and CLI smoke tests cover the surface.

**Implemented**:
- **Pure-Rust test source generators** in `rust_media_codec/tests/common/mod.rs` and `rust_media_format/tests/common/mod.rs`:
  - `sine_wave_frame()` / `sine_wave_frames()` — generate sine waves at any frequency/sample rate
  - `solid_color_frame()` / `color_ramp_frames()` — generate YUV420P video frames with known content
  - Analysis helpers: `rms()`, `estimate_frequency_zero_crossings()`, `mean_y()`
- **Codec round-trip tests** (`rust_media_codec/tests/`) — encode → decode → verify content preservation:
  - `opus_roundtrip_test`: 1 kHz sine → Opus → recover frequency + RMS within tolerance
  - `vp8_roundtrip_test`, `vp9_roundtrip_test`: color ramp → encode → verify per-frame mean Y
  - `av1_roundtrip_test`: color ramp → rav1e encode → dav1d decode → verify mean Y; also asserts `codec_config()` first byte is `0x81` (av1C marker+version)
  - `h264_roundtrip_test` (gpl-x264): exercises B-frame reordering through the POC reorder buffer
  - `aac_roundtrip_test` (fdk-aac): sine wave with AudioSpecificConfig wiring
- **MP4 full pipeline test** (`rust_media_format/tests/mp4_roundtrip_test.rs`): generate → encode (VP9 + Opus) → mux to MP4 → demux → decode → verify content. Exercises sample tables, codec config records, stream metadata round-trip.
- **CLI smoke tests** (`rust_media_cli/tests/cli_smoke_tests.rs`) using `assert_cmd`: exit codes, error messages on stderr, stdout/stderr separation, format detection edge cases, filter parsing errors, help output. Complements the existing JSON-validation tests in `integration_tests.rs`.

**Key principle**: All round-trip and pipeline tests **generate test content in pure Rust** — no committed binary fixtures, no `ffmpeg` dependency. Verification uses tolerance-based metrics (frequency, RMS, mean luma) since lossy codecs can't preserve exact byte-level content.

**Planned**:
- Performance benchmarking suite
- Reference comparison against ffmpeg output (where available)

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
  - Uses libvpx via vpx-rs bindings
  - Decoder: Fully functional with YUV420P (I420) output
  - Encoder: Fully configurable via `Vp8EncoderConfig` builder
    - Rate control: VBR, CBR, CQ (constrained quality), Q (pure quantizer)
    - Speed (cpu-used), GOP size, keyframe intervals, quantizer range, threads
    - `Vp8Encoder::with_config(stream_info, config)` preferred constructor
  - See `crates/rust_media_codec/examples/test_vp8_codec.rs` for decode/encode roundtrip example
- ✅ **H.264/AVC** (decoder + encoder) **IMPLEMENTED**
  - **Decoder**: rust_h264 (MIT/Apache-2.0) in `rust_media_codec/src/video/h264.rs`
    - Uses rust_h264 crate (v0.3.0) - pure Rust H.264 decoder
    - Supports Baseline, Main, and High profiles
    - YUV420P (I420) output format
    - **Display-order output** comes from upstream `decoder::OrderedDecoder`,
      which wraps the raw decoder with a POC-based reorder buffer (max depth
      16) and tracks GOP boundaries via IDR slices. We previously did this
      ourselves with a `BinaryHeap<PocFrame>` and an IDR-counter; that's
      gone in favor of the upstream implementation.
    - **AVCC parsing** comes from `nal::parse_avcc_config` (extracts SPS/PPS
      and length-prefix size from the `avcC` box) and `nal::parse_avcc`
      (parses length-prefixed sample data). No more hand-rolled
      `avcc_to_annex_b` conversion.
    - Supports both AVCC samples (MP4/MKV) and Annex B (raw H.264). The
      decoder picks the parser based on whether `extra_data` is non-empty.
    - PTS handling: rust_h264 has no PTS field, but `OrderedDecoder` already
      emits frames in display order (which matches input order at the
      packet boundary), so a simple `VecDeque<Option<i64>>` FIFO suffices —
      we push one entry per input packet and pop one per output frame.
  - **Encoder**: x264 (GPL v2+) in `rust_media_codec/src/video/x264.rs`
    - Requires `gpl-x264` feature flag
    - Uses x264 crate (v0.5.0) - safe Rust bindings to libx264
    - Requires system libx264 (`brew install x264` or `apt-get install libx264-dev`)
    - Configurable via `X264EncoderConfig` builder:
      - Speed preset 0..9 (ultrafast → placebo), default 5 (medium)
      - CRF quality, GOP size, min keyframe interval
    - Profile: High, Annex B output, full B-frame support
    - `codec_config()` returns cached avcC (AVCDecoderConfigurationRecord)
      built from SPS/PPS at construction time
    - Build with: `cargo build --features gpl-x264`
- ✅ **VP9**: Google's successor to VP8 **IMPLEMENTED** in `rust_media_codec/src/video/vp9.rs`
  - Uses libvpx via vpx-rs bindings
  - Decoder: Fully functional with YUV420P (I420) output, 10-bit Profile 2 planned
  - Encoder: Fully configurable via `Vp9EncoderConfig` builder
    - Rate control: VBR, CBR, CQ, Q (same as VP8)
    - Speed, GOP size, keyframe intervals, quantizer range, threads
    - VP9-specific: tile columns and tile rows for parallelization
    - `codec_config()` returns vpcC payload for MP4/WebM muxing
  - BSD-3-Clause licensed - default, MIT-compatible
  - See `crates/rust_media_codec/examples/test_vp9_codec.rs` for decode/encode roundtrip example
- ✅ **H.265/HEVC** (decoder) **IMPLEMENTED** via VideoToolbox
  - **Decoder**: VideoToolbox (macOS only) in `rust_media_codec/src/video/videotoolbox.rs`
    - Requires `videotoolbox` feature flag
    - Hardware accelerated on Apple Silicon
    - Supports Main, Main 10 (10-bit/HDR), and Main Still Picture profiles
    - Parses HEVCDecoderConfigurationRecord (hvcC) from MP4
    - 8-bit output: YUV420P (NV12 internally, deinterleaved to I420)
    - 10-bit output: YUV420P10LE (P010 internally via the documented `'x420'`
      pixel format, explicitly requested via `kCVPixelBufferPixelFormatTypeKey`
      in the destination buffer attributes — VideoToolbox otherwise picks the
      undocumented `'p420'` internal format which has no public layout spec)
    - No software fallback available (macOS only)
- ✅ **AV1** (decoder) **IMPLEMENTED** in `rust_media_codec/src/video/av1.rs`
  - Uses dav1d via the `dav1d` crate (BSD-2-Clause, default build)
  - Requires libdav1d (`brew install dav1d` / `apt-get install libdav1d-dev`)
  - Supports Main Profile (8-bit) → YUV420P and Main 10 (10-bit) → YUV420P10LE
  - 12-bit (Profile 2) is rejected with an explicit unimplemented error
  - 4:2:0 only — 4:2:2 / 4:4:4 returns Unsupported
  - Bit depth read from `picture.bit_depth()` (returns the actual bit depth
    8/10/12, **not** the storage size — easy thing to misread in the docs)
  - PTS round-tripped through dav1d's `timestamp` field via a small monotonic
    counter map, since dav1d doesn't preserve the original PTS otherwise
- ✅ **AV1 encoder (rav1e)** **IMPLEMENTED** in `rust_media_codec/src/video/av1.rs`
  - Pure-Rust, BSD-2-Clause, no external library required (in default build)
  - Supports both `YUV420P` (8-bit) and `YUV420P10LE` (10-bit) input. Bit
    depth is locked at construction from `StreamInfo.params.pixel_format`
    because `rav1e::Context<T>` is generic — the encoder holds an enum
    `Rav1eVariant::{Eight(Context<u8>), Ten(Context<u16>)}` chosen up front.
  - **Configuration**: `Av1EncoderConfig` builder exposes:
    - `speed_preset(0..=10)` — clamped to 10. Default 6.
    - `rate_control(Av1RateControl)` with shorthand `quantizer(u8)` and
      `bitrate(u32 bps)`. The two are mutually exclusive (modeled as an enum)
      and the last setter wins. Default `Quantizer(100)`.
    - `key_frame_interval(min, max)` — `max == 0` maps to "infinite" via
      rav1e's `set_key_frame_interval`. Default 12 / 240.
    - `tile_cols`, `tile_rows`, `tiles` — 0 lets rav1e choose. Default 0/0/0.
    - `low_latency`, `error_resilient` — bool flags. Default false.
  - Three constructors:
    - `Av1Encoder::new(stream_info)` — uses `Av1EncoderConfig::default()`.
      For backwards compatibility, if `stream_info.bitrate` is set it's
      promoted into `Av1RateControl::Bitrate(..)` automatically.
    - `Av1Encoder::with_bitrate(stream_info, bps)` — sets
      `stream_info.bitrate` then delegates to `new`. Legacy convenience.
    - `Av1Encoder::with_config(stream_info, config)` — preferred for any
      non-default settings. The config wins over `stream_info.bitrate`.
  - For testing, even at speed preset 6 a single 320x240 30-frame encode
    dominates a debug-mode test run, so the round-trip test uses
    160x120 / 8 frames.
  - PTS handling: rav1e doesn't take PTS, only sequential `input_frameno`s.
    We track input PTS in a `VecDeque<Option<i64>>` and pop one entry per
    output packet, since rav1e emits packets in display order with
    monotonically increasing `input_frameno`.
  - **Codec config record**: `Av1Encoder::codec_config()` exposes the
    sequence header in the format expected by both ISOBMFF (av1C box) and
    Matroska (CodecPrivate). It can be called immediately after construction
    (before any frames are sent) because the header is fully determined by
    the encoder config. The CLI uses this to populate `extra_data` on the
    muxer's stream before `add_stream()`.
  - **CLI integration**: When `-v av1` is passed, `create_video_encoder`
    propagates the input pixel format (8-bit or 10-bit) so 10-bit content
    is encoded natively. The CLI also sets `encoder_supports_10bit = true`
    for AV1, which **disables** the auto 10→8 bit downconversion that
    other 8-bit-only encoders trigger.
  - End-to-end verified: H.264 → AV1 (8-bit) and 10-bit HEVC → AV1
    (10-bit) both work without information loss; round-trip through dav1d
    confirms the av1C bytes are valid.
  - See `crates/rust_media_codec/tests/av1_roundtrip_test.rs` for the
    encode → decode round-trip test (uses generated color ramps + per-frame
    mean-Y validation, no committed binary fixtures).

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
- ✅ **Vorbis**: Vorbis decoding via lewton. **IMPLEMENTED** in `rust_media_codec/src/audio/vorbis.rs`
  - Uses lewton crate (v0.10, BSD-3-Clause) - pure Rust, no external libraries
  - Parses Xiph-laced identification/comment/setup headers from Matroska CodecPrivate
  - Uses lewton's low-level `read_audio_packet()` API for per-packet decoding
  - Maintains `PreviousWindowRight` state across packets for overlap-add windowing
  - Output: interleaved S16 PCM
  - Decode only (no encoder)

**Image Codecs** (for thumbnails, still images) - Priority:
- JPEG
- PNG
- HEIC
- AVIF

**Possible Future Codec Support** (depending on requirements):
- **H.265/HEVC encode**: Via x265 (GPL v2+), requires `gpl-x265` feature
- **FLAC**: Lossless audio codec

#### 4. Color Space and Bit Depth Handling 🚧 IN PROGRESS

**Bit depth — implemented**:
- ✅ `YUV420P` (8-bit) and `YUV420P10LE` (10-bit, value in lower 10 bits of LE u16)
- ✅ Bit depth parsed from codec config records: `av1C` (AV1) and `hvcC` (HEVC).
  Stored in `VideoStreamParams.bit_depth` and reflected in the `info` command.
- ✅ Decoders: dav1d (AV1) and VideoToolbox (HEVC) emit `YUV420P10LE` for
  10-bit content. dav1d returns the actual bit depth from `picture.bit_depth()`;
  VideoToolbox is forced into the documented `'x420'` (P010) format via
  `kCVPixelBufferPixelFormatTypeKey` and converted MSB→LSB inline.
- ✅ Filters: `scale` and `crop` have 8-bit and 10-bit code paths. The `format`
  filter (`rust_media_filter::video::yuv420p10le_to_yuv420p`) implements
  10→8 bit downconversion (right-shift by 2).
- ✅ CLI auto-conversion: when an 8-bit-only encoder receives a `YUV420P10LE`
  frame, the CLI emits a one-time stderr notice and downconverts.
- 📋 12-bit (Profile 2) — explicitly rejected with `Error::Unsupported` at
  every layer (not silently fallback).

**Color metadata — partial**:
- ✅ `ColorSpace`, `ColorRange`, `sample_aspect_ratio` fields exist on
  `VideoStreamParams` and `Frame`
- 📋 HDR metadata (mastering display, content light level) — not yet parsed
  from HEVC SEI; not yet preserved through MP4 mux
- 📋 YUV ↔ RGB conversion filter
- 📋 VUI parameters (color primaries, transfer characteristics, matrix
  coefficients) — not yet parsed from SPS / hvcC

#### 5. Filter Graph System 🚧 IN PROGRESS
Build a flexible filtering system that:
- Supports composable filter chains
- Allows CLI-based filter graph construction
- Handles video and audio processing
- Enables common operations (scaling, cropping, mixing, etc.)

**Status**: Foundation implemented in `rust_media_filter` crate. Filter graph parsing, audio resampling, volume, video scale, video crop, and SSIM video quality metric are available. Auto-resampling on sample rate mismatch. More filters planned.

**Implemented**:
- **Filter graph parsing**: FFmpeg-like syntax (`filter_name=param1=value1:param2=value2`, comma-separated chains). Supports both named and positional arguments.
- **aresample**: Audio resampling via sinc interpolation (rubato, Kaiser-windowed polyphase filter)
- **volume**: Audio volume/gain adjustment (linear factor or dB, e.g., `volume=0.5`, `volume=6dB`)
- **scale**: Bilinear video scaling, 8-bit and 10-bit. Syntax: `scale=W:H`, `scale=w=W:h=H`, plus FFmpeg-style aspect-ratio markers `scale=W:-1` (preserve aspect, round to even) and `scale=W:-2` (round dimension to nearest even). Requires even dimensions for YUV420 chroma alignment.
- **crop**: Video cropping, 8-bit and 10-bit. Syntax: `crop=W:H` (centered), `crop=W:H:X:Y` (positional with offset), or `crop=w=W:h=H:x=X:y=Y` (named). Requires even dimensions and offsets.
- **ssim**: SSIM quality comparison between two video streams
- **format conversion**: `yuv420p10le_to_yuv420p()` helper in `rust_media_filter::video::format` for 10→8 bit downconversion (right-shift by 2)
- **Auto-resample**: Automatically resamples when encoder requires a different sample rate (e.g., MP3 44100 Hz → Opus 48000 Hz)
- **Auto 10→8 bit conversion**: CLI downconverts `YUV420P10LE` frames to `YUV420P` when feeding an 8-bit-only encoder, with a one-time stderr notice
- **Format detection**: Magic bytes detection with file extension fallback (`rust_media_format::detect`)
- CLI integration via `--af` (audio filters) and `--vf` (video filters)
- **Filter pipeline order**: For video, the chain is `decode → SSIM → crop → scale → encode`, so chained filters like `--vf "crop=320:240,scale=160:120"` produce a final 160x120 output.

**Key Requirements** (for future filters):
- **Streaming API**: Filters must operate on frames incrementally (send/receive pattern)
- **Graph construction**: Support both programmatic and CLI-based filter graph creation
- **Zero-copy where possible**: Minimize frame copying in filter chains
- **Common filters**: Overlay, rotate, format conversion, audio mixing

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

### Encoder Configuration Pattern

All video encoders follow a consistent pattern with codec-specific config structs
and builder APIs, similar to AV1's `Av1EncoderConfig`:

- **VP8**: `Vp8EncoderConfig` — speed, rate control (VBR/CBR/CQ/Q), GOP size, keyframe min/max, quantizer range, threads
- **VP9**: `Vp9EncoderConfig` — same as VP8 plus tile columns/rows; `codec_config()` returns vpcC payload
- **H.264**: `X264EncoderConfig` — speed preset (0..9 mapping to x264 presets), CRF, GOP size, keyint min; `codec_config()` returns avcC record
- **AV1**: `Av1EncoderConfig` — speed preset, rate control (quantizer/bitrate), key frame interval, tiles, low latency, error resilient; `codec_config()` returns av1C sequence header

All encoders provide `new(stream_info)` for defaults, `with_bitrate(stream_info, bps)` for convenience,
and `with_config(stream_info, config)` for full control. The CLI maps its `--speed`, `--qp`, `-g`,
`--keyint-min`, `--tile-columns`, `--tile-rows` flags to each encoder's config automatically.

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
