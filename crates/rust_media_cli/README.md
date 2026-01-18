# rust_media_cli

Command-line tool for media analysis and processing, part of the rust_media framework.

## Installation

```bash
# Build from source
cargo build -p rust_media_cli --release

# The binary will be at target/release/rust_media
```

### Optional Features

```bash
# Build with AAC support
cargo build -p rust_media_cli --release --features fdk-aac

# Build with H.264 encoder support (GPL)
cargo build -p rust_media_cli --release --features gpl-x264

# Build with all features
cargo build -p rust_media_cli --release --features "fdk-aac gpl-x264"
```

## Commands

### `info` - Media File Analysis

The `info` command analyzes media files and displays information about streams, packets, and frames. It provides functionality similar to `ffprobe`.

#### Basic Usage

```bash
# Show stream information
rust_media info video.mp4

# Show packet information
rust_media info video.mp4 --show-packets

# Show frame information (requires decoding)
rust_media info video.mp4 --show-frames

# Limit output to first N packets/frames
rust_media info video.mp4 --show-packets --count 10

# Filter by stream index
rust_media info video.mp4 --show-packets --stream 0

# Output as JSON
rust_media info video.mp4 --output-format json
```

#### Command Options

| Option | Short | Description |
|--------|-------|-------------|
| `--show-packets` | `-p` | Display packet information (timing, sizes) |
| `--show-frames` | `-f` | Display frame information (requires decoding) |
| `--count <N>` | `-n` | Limit output to N packets/frames (0 = unlimited) |
| `--stream <INDEX>` | `-s` | Filter by stream index |
| `--output-format <FORMAT>` | `-o` | Output format: `text` (default) or `json` |

#### Output Fields

**Stream Information:**
- Format name, duration, bitrate
- Number of streams
- Per-stream: media type, codec, dimensions/sample rate, pixel/sample format

**Packet Information (`-p`):**
- Packet index and stream index
- Media type (video/audio)
- PTS (Presentation Timestamp) and PTS time in seconds
- DTS (Decoding Timestamp)
- Packet size in bytes
- Keyframe indicator

**Frame Information (`-f`):**
- Frame index and stream index
- Media type
- PTS and PTS time
- Picture type (I for keyframe, P for predicted)
- Video: dimensions and pixel format
- Audio: number of samples and channels

#### Examples

**Basic stream info:**
```
$ rust_media info test.webm

Input: test.webm
  Format:     webm
  Duration:   0:01.000
  Streams:    1

Streams:
  Stream #0: video (vp9), 640x480, yuv420p, 25/1 fps, 8 bit
```

**Packet dump:**
```
$ rust_media info test.webm -p -n 5

Input: test.webm
  Format:     webm
  Duration:   0:01.000
  Streams:    1

Streams:
  Stream #0: video (vp9), 640x480, yuv420p, 25/1 fps, 8 bit

Packets:
   PKT STREAM     TYPE          PTS     PTS_TIME          DTS     SIZE  KEY
     0      0    video            0     0.000000            -    10287    K
     1      0    video        33000     0.033000            -      247
     2      0    video        67000     0.067000            -      337
     3      0    video       100000     0.100000            -      269
     4      0    video       133000     0.133000            -      363
```

**JSON output:**
```
$ rust_media info test.webm -o json

{
  "format": {
    "filename": "test.webm",
    "format_name": "webm",
    "duration_us": 1000000,
    "duration": "0:01.000",
    "nb_streams": 1
  },
  "streams": [
    {
      "index": 0,
      "media_type": "video",
      "codec": "vp9",
      "time_base": "1/1000000",
      "width": 640,
      "height": 480,
      "pixel_format": "yuv420p",
      "frame_rate": "25/1",
      "bit_depth": 8
    }
  ]
}
```

## Supported Formats

| Format | Extensions | Codecs |
|--------|------------|--------|
| WAV | `.wav` | PCM |
| WebM | `.webm` | VP8, VP9, Opus |
| MP4 | `.mp4`, `.m4a`, `.m4v`, `.mov` | H.264, VP9, AAC*, Opus |

*AAC requires the `fdk-aac` feature.

## Comparison with ffprobe

### Feature Comparison

| Feature | rust_media info | ffprobe |
|---------|-----------------|---------|
| Stream information | ✅ | ✅ |
| Packet analysis | ✅ | ✅ |
| Frame analysis | ✅ | ✅ |
| JSON output | ✅ | ✅ |
| XML output | ❌ | ✅ |
| CSV output | ❌ | ✅ |
| Flat output | ❌ | ✅ |
| Stream selection | ✅ (by index) | ✅ (by specifier) |
| Count limiting | ✅ | ✅ |
| Chapter information | ❌ | ✅ |
| Format probing | ❌ (extension-based) | ✅ (content-based) |
| Network streams | ❌ | ✅ |
| Subtitle streams | ❌ | ✅ |

### Equivalent Commands

| Task | rust_media | ffprobe |
|------|------------|---------|
| Basic info | `rust_media info file.mp4` | `ffprobe file.mp4` |
| JSON output | `rust_media info file.mp4 -o json` | `ffprobe -print_format json -show_format -show_streams file.mp4` |
| Show packets | `rust_media info file.mp4 -p` | `ffprobe -show_packets file.mp4` |
| Show frames | `rust_media info file.mp4 -f` | `ffprobe -show_frames file.mp4` |
| First 10 packets | `rust_media info file.mp4 -p -n 10` | `ffprobe -show_packets -read_intervals %+#10 file.mp4` |
| Video stream only | `rust_media info file.mp4 -p -s 0` | `ffprobe -select_streams v:0 -show_packets file.mp4` |

### Key Differences

1. **Format Detection**: rust_media uses file extension; ffprobe probes file content
2. **Stream Selection**: rust_media uses numeric index; ffprobe uses stream specifiers (`v:0`, `a:0`)
3. **Output Verbosity**: ffprobe has more granular control over which fields to display
4. **Protocol Support**: ffprobe supports network protocols (http, rtmp, etc.)

## Missing Features (vs ffprobe)

### High Priority

| Feature | Description | ffprobe Flag |
|---------|-------------|--------------|
| Format probing | Detect format from file content, not extension | (automatic) |
| Stream specifiers | Select streams by type (`v:0`, `a:1`, `s:0`) | `-select_streams` |
| Entry selection | Choose specific fields to display | `-show_entries` |
| Read intervals | Analyze specific time ranges | `-read_intervals` |
| Sexagesimal time | Display time as HH:MM:SS.mmm | `-sexagesimal` |

### Medium Priority

| Feature | Description | ffprobe Flag |
|---------|-------------|--------------|
| Chapter info | Display chapter markers | `-show_chapters` |
| Program info | Display program/service info | `-show_programs` |
| Pixel formats | List supported pixel formats | `-show_pixel_formats` |
| Private data | Show codec private data | `-show_private_data` |
| Packet side data | Display packet side data | (in packet output) |
| Frame side data | Display frame side data | (in frame output) |

### Low Priority

| Feature | Description | ffprobe Flag |
|---------|-------------|--------------|
| XML output | Output in XML format | `-print_format xml` |
| CSV output | Output in CSV format | `-print_format csv` |
| Flat output | Output in flat key=value format | `-print_format flat` |
| INI output | Output in INI format | `-print_format ini` |
| Compact output | Compact single-line output | `-print_format compact` |
| Network streams | HTTP, RTMP, RTSP, etc. | (protocol support) |
| Device input | Capture devices, screen recording | (device support) |
| Bitstream filters | Analyze bitstream filter output | `-bsf` |
| Decryption | Analyze encrypted content | `-decryption_key` |

### Codec Support Gaps

| Codec | rust_media | ffprobe |
|-------|------------|---------|
| H.264 decode | ❌ Planned | ✅ |
| H.265/HEVC | ❌ Planned | ✅ |
| AV1 | ❌ Planned | ✅ |
| MP3 | ❌ | ✅ |
| Vorbis | ❌ | ✅ |
| FLAC | ❌ | ✅ |
| Subtitles (SRT, ASS) | ❌ | ✅ |

### Container Support Gaps

| Container | rust_media | ffprobe |
|-----------|------------|---------|
| MKV | ❌ Planned | ✅ |
| AVI | ❌ | ✅ |
| FLV | ❌ | ✅ |
| TS/M2TS | ❌ | ✅ |
| OGG | ❌ | ✅ |

## Roadmap

### Planned Improvements

1. **Format probing** - Detect format from magic bytes instead of extension
2. **Stream specifiers** - Support `v:0`, `a:0` style stream selection
3. **Entry selection** - Allow selecting specific fields for output
4. **More codecs** - H.264 decoder, AV1, FLAC, Vorbis
5. **More containers** - MKV, AVI, FLV
6. **B-frame detection** - Proper I/P/B frame type detection from codec headers

## License

MIT OR Apache-2.0

When GPL features are enabled (e.g., `gpl-x264`), the compiled binary is licensed under GPL v2+.
