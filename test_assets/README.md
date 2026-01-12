# Test Assets

This directory contains test audio files for the rust_media test suite.

## Files

### WebM/Opus Files
- `test_sine_opus.webm` - 1 second, 440 Hz sine wave, mono, Opus @ 64kbps
- `reference_opus.webm` - Reference Opus file for validation tests
- `test_opus.webm` - Minimal WebM structure test file

### WAV/PCM Files
- `reference.wav` - Reference PCM audio (48kHz, stereo, 16-bit)

## Generating Test Files

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

## Notes

- These files are used by integration tests in `rust_media_format`
- Tests will skip gracefully if files are not present
- Use ffmpeg to regenerate if files become corrupted
- Keep files small (< 100 KB) for fast test execution
