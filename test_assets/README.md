# Test Assets

This directory contains test media files for the rust_media test suite.

## Files

### WebM/Video Files
- `test_vp8.webm` - VP8 video test file (640x480, 30fps, 1s, 39KB)
- `test_vp9.webm` - VP9 video test file (640x480, 30fps, 1s, 21KB)

### WebM/Opus Files
- `test_sine_opus.webm` - 1 second, 440 Hz sine wave, mono, Opus @ 64kbps
- `reference_opus.webm` - Reference Opus file for validation tests
- `test_opus.webm` - Minimal WebM structure test file

### WAV/PCM Files
- `reference.wav` - Reference PCM audio (48kHz, stereo, 16-bit)

## Generating Test Files

### Create Test VP8 Video
```bash
# Generate a simple test pattern video with VP8 codec
ffmpeg -f lavfi -i testsrc=duration=1:size=640x480:rate=30 \
  -c:v libvpx -b:v 320k -auto-alt-ref 0 \
  test_assets/test_vp8.webm
```

### Create Test VP9 Video
```bash
# Generate a simple test pattern video with VP9 codec
ffmpeg -f lavfi -i testsrc=duration=1:size=640x480:rate=30 \
  -c:v libvpx-vp9 -b:v 170k \
  test_assets/test_vp9.webm
```

### Create Test WebM with Opus Audio
```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=1" -c:a libopus -b:a 64k test_assets/test_sine_opus.webm
```

### Create Reference Files for Validation
```bash
# Generate reference PCM
ffmpeg -f lavfi -i "sine=frequency=440:duration=1" -ar 48000 -ac 2 test_assets/reference.wav

# Encode to Opus
ffmpeg -i test_assets/reference.wav -c:a libopus -b:a 64k test_assets/reference_opus.webm
```

## Usage in Tests

Tests automatically look for files in this directory:

```rust
// Tests will find files at:
// {workspace_root}/test_assets/test_sine_opus.webm
// {workspace_root}/test_assets/reference.wav
// {workspace_root}/test_assets/reference_opus.webm
```

## File Specifications

### test_vp8.webm
- **Format**: WebM (Matroska subset)
- **Video Codec**: VP8
- **Resolution**: 640×480
- **Frame Rate**: ~30 fps
- **Duration**: 1.0 seconds (~30 frames)
- **Bitrate**: 319 kbps
- **Content**: Test pattern (color bars and gradients)
- **File Size**: 39,876 bytes (~39 KB)

### test_vp9.webm
- **Format**: WebM (Matroska subset)
- **Video Codec**: VP9
- **Resolution**: 640×480
- **Frame Rate**: ~30 fps
- **Duration**: 1.0 seconds (~30 frames)
- **Bitrate**: 171 kbps (~47% smaller than VP8!)
- **Content**: Test pattern (color bars and gradients)
- **File Size**: 21,337 bytes (~21 KB)
- **Note**: Demonstrates VP9's superior compression (nearly 2x better than VP8)

### test_sine_opus.webm
- **Format**: WebM (Matroska subset)
- **Audio Codec**: Opus
- **Sample Rate**: 48000 Hz
- **Channels**: Mono (1 channel)
- **Bitrate**: 64 kbps
- **Duration**: 1 second
- **Content**: 440 Hz sine wave

### reference.wav
- **Format**: WAV (RIFF WAVE)
- **Audio Codec**: PCM (uncompressed)
- **Sample Rate**: 48000 Hz
- **Channels**: Stereo (2 channels)
- **Bit Depth**: 16-bit signed
- **Duration**: 1 second
- **Content**: 440 Hz sine wave

### reference_opus.webm
- **Format**: WebM
- **Audio Codec**: Opus
- **Sample Rate**: 48000 Hz
- **Channels**: Stereo (2 channels)
- **Bitrate**: 64 kbps
- **Duration**: 1 second
- **Content**: Encoded from reference.wav

## Usage in Examples

### Video Codec Examples
```bash
# Test VP8 encoder/decoder
cargo run -p rust_media_codec --example test_vp8_codec

# Test VP9 encoder/decoder
cargo run -p rust_media_codec --example test_vp9_codec
```

These examples demonstrate:
- Decoding VP8/VP9 from WebM files
- Frame-by-frame decoding with PTS information
- Decode → Encode roundtrip testing
- WebM muxing with encoded video

## Notes

- These files are used by integration tests in `rust_media_format` and `rust_media_codec`
- Tests will skip gracefully if files are not present
- Use ffmpeg to regenerate if files become corrupted
- Keep files small (< 100 KB) for fast test execution
- VP8 files typically larger than VP9 at equivalent quality (VP9 achieves ~30-50% better compression)
- Test pattern content (color bars) compresses well and provides good visual verification
