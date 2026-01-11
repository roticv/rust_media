# rust_media

A Rust-based media processing framework - an FFmpeg equivalent with streaming APIs.

## Overview

`rust_media` is a comprehensive media processing framework that provides:
- **Streaming APIs** for bounded memory usage with arbitrarily large files
- **Container format** support (MP4, MKV, WebM)
- **Codec** support (PCM, Opus implemented; H.264, VP9, AV1, AAC planned)
- **Modular architecture** with separate crates for different components

## Project Structure

This is a Cargo workspace with multiple crates:

- **[rust_media_core](crates/rust_media_core)** - Core types (Packet, Frame) and traits (Demuxer, Decoder, Encoder, Muxer)
- **[rust_media_format](crates/rust_media_format)** - Container format implementations
- **[rust_media_codec](crates/rust_media_codec)** - Codec implementations
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

This project is in early development. Current status:

- ✅ Core data structures (Packet, Frame)
- ✅ Trait definitions (Demuxer, Decoder, Encoder, Muxer)
- ✅ Workspace structure
- ✅ **PCM codec** (decoder + encoder)
- ✅ **Opus codec** (decoder + encoder)
- ✅ **WAV demuxer** (read PCM from WAV files)
- 🚧 Container format implementations (MP4, MKV, WebM muxers)
- 🚧 Codec implementations (H.264, VP9, AV1, AAC)
- 📋 Filter system (scale, crop, format conversion, etc.) - Planned
- 🚧 CLI tool

## Contributing

See [CLAUDE.md](CLAUDE.md) for detailed architectural guidance.

## License

MIT OR Apache-2.0
