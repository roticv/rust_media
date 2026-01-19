//! Integration tests for rust_media CLI tool
//!
//! These tests verify that the `info` and `transform` commands work correctly
//! with the test assets in the repository.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the workspace root directory
fn workspace_root() -> PathBuf {
    // The CLI crate is at crates/rust_media_cli, so workspace root is ../../
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Get the path to a test asset
fn test_asset(name: &str) -> String {
    workspace_root()
        .join("test_assets")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

/// Get the path to the built binary
fn cli_binary() -> PathBuf {
    // Use the pre-built binary from target/debug
    workspace_root().join("target").join("debug").join("rust_media")
}

/// Helper to run the CLI and capture JSON output
fn run_cli(args: &[&str]) -> Result<Value, String> {
    let binary = cli_binary();

    // Build the binary first if needed
    if !binary.exists() {
        let build_output = Command::new("cargo")
            .args(["build", "-p", "rust_media_cli"])
            .current_dir(workspace_root())
            .output()
            .map_err(|e| format!("Failed to build: {}", e))?;

        if !build_output.status.success() {
            return Err(format!("Build failed: {}", String::from_utf8_lossy(&build_output.stderr)));
        }
    }

    let output = Command::new(&binary)
        .args(args)
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Command failed: {} {}", stderr, stdout));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse JSON: {} - {}", e, stdout))
}

/// Helper to run the CLI and capture text output
fn run_cli_text(args: &[&str]) -> Result<String, String> {
    let binary = cli_binary();

    // Build the binary first if needed
    if !binary.exists() {
        let build_output = Command::new("cargo")
            .args(["build", "-p", "rust_media_cli"])
            .current_dir(workspace_root())
            .output()
            .map_err(|e| format!("Failed to build: {}", e))?;

        if !build_output.status.success() {
            return Err(format!("Build failed: {}", String::from_utf8_lossy(&build_output.stderr)));
        }
    }

    let output = Command::new(&binary)
        .args(args)
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Command failed: {} {}", stderr, stdout));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ============================================================================
// Stream Info Tests
// ============================================================================

mod stream_info {
    use super::*;

    #[test]
    fn test_vp9_webm_stream_info() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-o", "json"])
            .expect("Failed to run info command");

        // Verify format info
        let format = &json["format"];
        assert_eq!(format["format_name"], "webm");
        assert_eq!(format["duration_us"], 1000000);
        assert_eq!(format["nb_streams"], 1);

        // Verify stream info
        let streams = json["streams"].as_array().expect("streams should be array");
        assert_eq!(streams.len(), 1);

        let stream = &streams[0];
        assert_eq!(stream["index"], 0);
        assert_eq!(stream["media_type"], "video");
        assert_eq!(stream["codec"], "vp9");
        assert_eq!(stream["width"], 640);
        assert_eq!(stream["height"], 480);
        assert_eq!(stream["pixel_format"], "yuv420p");
        assert_eq!(stream["frame_rate"], "25/1");
        assert_eq!(stream["bit_depth"], 8);
    }

    #[test]
    fn test_vp8_webm_stream_info() {
        let asset = test_asset("test_vp8.webm");
        let json = run_cli(&["info", &asset, "-o", "json"])
            .expect("Failed to run info command");

        // Verify format info
        let format = &json["format"];
        assert_eq!(format["format_name"], "webm");
        assert_eq!(format["duration_us"], 1000000);
        assert_eq!(format["nb_streams"], 1);

        // Verify stream info
        let streams = json["streams"].as_array().expect("streams should be array");
        assert_eq!(streams.len(), 1);

        let stream = &streams[0];
        assert_eq!(stream["index"], 0);
        assert_eq!(stream["media_type"], "video");
        assert_eq!(stream["codec"], "vp8");
        assert_eq!(stream["width"], 640);
        assert_eq!(stream["height"], 480);
        assert_eq!(stream["pixel_format"], "yuv420p");
        assert_eq!(stream["frame_rate"], "25/1");
        assert_eq!(stream["bit_depth"], 8);
    }

    #[test]
    fn test_opus_webm_stream_info() {
        let asset = test_asset("reference_opus.webm");
        let json = run_cli(&["info", &asset, "-o", "json"])
            .expect("Failed to run info command");

        // Verify format info
        let format = &json["format"];
        assert_eq!(format["format_name"], "webm");
        assert_eq!(format["duration_us"], 1008000);
        assert_eq!(format["nb_streams"], 1);

        // Verify stream info
        let streams = json["streams"].as_array().expect("streams should be array");
        assert_eq!(streams.len(), 1);

        let stream = &streams[0];
        assert_eq!(stream["index"], 0);
        assert_eq!(stream["media_type"], "audio");
        assert_eq!(stream["codec"], "opus");
        assert_eq!(stream["sample_rate"], 48000);
        assert_eq!(stream["channels"], 2);
        assert_eq!(stream["sample_format"], "s16");
    }

    #[test]
    fn test_wav_stream_info() {
        let asset = test_asset("reference.wav");
        let json = run_cli(&["info", &asset, "-o", "json"])
            .expect("Failed to run info command");

        // Verify format info
        let format = &json["format"];
        assert_eq!(format["format_name"], "wav");
        assert_eq!(format["duration_us"], 1000000);
        assert_eq!(format["nb_streams"], 1);

        // Verify stream info
        let streams = json["streams"].as_array().expect("streams should be array");
        assert_eq!(streams.len(), 1);

        let stream = &streams[0];
        assert_eq!(stream["index"], 0);
        assert_eq!(stream["media_type"], "audio");
        assert_eq!(stream["codec"], "pcm");
        assert_eq!(stream["sample_rate"], 48000);
        assert_eq!(stream["channels"], 2);
        assert_eq!(stream["sample_format"], "s16");
    }
}

// ============================================================================
// Packet Info Tests
// ============================================================================

mod packet_info {
    use super::*;

    #[test]
    fn test_vp9_packet_count() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-p", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");
        assert_eq!(packets.len(), 30, "VP9 test file should have 30 packets");
    }

    #[test]
    fn test_vp9_packet_timing() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-p", "-n", "5", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");
        assert_eq!(packets.len(), 5);

        // First packet should be at PTS 0
        assert_eq!(packets[0]["pts"], 0);
        assert_eq!(packets[0]["stream_index"], 0);
        assert_eq!(packets[0]["media_type"], "video");

        // Verify packet timing increments (33ms = 33000us for 30fps, but file is 25fps = 40ms)
        // The test file uses ~33ms intervals
        assert_eq!(packets[1]["pts"], 33000);
        assert_eq!(packets[2]["pts"], 67000);
        assert_eq!(packets[3]["pts"], 100000);
        assert_eq!(packets[4]["pts"], 133000);
    }

    #[test]
    fn test_vp9_packet_sizes() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-p", "-n", "5", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");

        // First packet (keyframe) should be larger
        let first_size = packets[0]["size"].as_u64().unwrap();
        assert!(
            first_size > 1000,
            "First packet (keyframe) should be larger than 1KB, got {}",
            first_size
        );

        // Verify all packets have size > 0
        for packet in packets {
            let size = packet["size"].as_u64().unwrap();
            assert!(size > 0, "Packet size should be > 0");
        }
    }

    #[test]
    fn test_vp8_packet_count() {
        let asset = test_asset("test_vp8.webm");
        let json = run_cli(&["info", &asset, "-p", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");
        assert_eq!(packets.len(), 30, "VP8 test file should have 30 packets");
    }

    #[test]
    fn test_opus_packet_count() {
        let asset = test_asset("reference_opus.webm");
        let json = run_cli(&["info", &asset, "-p", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");
        assert_eq!(
            packets.len(),
            51,
            "Opus test file should have 51 packets"
        );
    }

    #[test]
    fn test_opus_packet_timing() {
        let asset = test_asset("reference_opus.webm");
        let json = run_cli(&["info", &asset, "-p", "-n", "5", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");

        // All packets should be audio type
        for packet in packets {
            assert_eq!(packet["media_type"], "audio");
            assert_eq!(packet["stream_index"], 0);
        }

        // First packet should be at PTS 0
        assert_eq!(packets[0]["pts"], 0);
    }

    #[test]
    fn test_packet_limit() {
        // Test -n option limits packet count
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-p", "-n", "10", "-o", "json"])
            .expect("Failed to run info command");

        let packets = json["packets"].as_array().expect("packets should be array");
        assert_eq!(packets.len(), 10, "Should have exactly 10 packets with -n 10");
    }
}

// ============================================================================
// Frame Info Tests
// ============================================================================

mod frame_info {
    use super::*;

    #[test]
    fn test_vp9_frame_count() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-f", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");
        assert_eq!(frames.len(), 30, "VP9 test file should decode to 30 frames");
    }

    #[test]
    fn test_vp9_frame_dimensions() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-f", "-n", "5", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");

        for frame in frames {
            assert_eq!(frame["media_type"], "video");
            assert_eq!(frame["width"], 640);
            assert_eq!(frame["height"], 480);
            assert_eq!(frame["pixel_format"], "yuv420p");
        }
    }

    #[test]
    fn test_vp9_frame_timing() {
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-f", "-n", "5", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");
        assert_eq!(frames.len(), 5);

        // First frame should be at PTS 0
        assert_eq!(frames[0]["pts"], 0);
        assert_eq!(frames[0]["frame_index"], 0);

        // Verify frame timing matches packet timing
        assert_eq!(frames[1]["pts"], 33000);
        assert_eq!(frames[2]["pts"], 67000);
        assert_eq!(frames[3]["pts"], 100000);
        assert_eq!(frames[4]["pts"], 133000);
    }

    #[test]
    fn test_vp8_frame_count() {
        let asset = test_asset("test_vp8.webm");
        let json = run_cli(&["info", &asset, "-f", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");
        assert_eq!(frames.len(), 30, "VP8 test file should decode to 30 frames");
    }

    #[test]
    fn test_vp8_frame_dimensions() {
        let asset = test_asset("test_vp8.webm");
        let json = run_cli(&["info", &asset, "-f", "-n", "3", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");

        for frame in frames {
            assert_eq!(frame["media_type"], "video");
            assert_eq!(frame["width"], 640);
            assert_eq!(frame["height"], 480);
            assert_eq!(frame["pixel_format"], "yuv420p");
        }
    }

    #[test]
    fn test_opus_frame_count() {
        let asset = test_asset("reference_opus.webm");
        let json = run_cli(&["info", &asset, "-f", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");
        assert_eq!(
            frames.len(),
            51,
            "Opus test file should decode to 51 frames"
        );
    }

    #[test]
    fn test_opus_frame_info() {
        let asset = test_asset("reference_opus.webm");
        let json = run_cli(&["info", &asset, "-f", "-n", "5", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");

        for frame in frames {
            assert_eq!(frame["media_type"], "audio");
            assert_eq!(frame["channels"], 2);
            // Opus frames typically have 960 samples (20ms at 48kHz)
            let samples = frame["nb_samples"].as_u64().unwrap();
            assert!(
                samples > 0,
                "Audio frame should have samples, got {}",
                samples
            );
        }
    }

    #[test]
    fn test_frame_limit() {
        // Test -n option limits frame count
        let asset = test_asset("test_vp9.webm");
        let json = run_cli(&["info", &asset, "-f", "-n", "10", "-o", "json"])
            .expect("Failed to run info command");

        let frames = json["frames"].as_array().expect("frames should be array");
        assert_eq!(frames.len(), 10, "Should have exactly 10 frames with -n 10");
    }
}

// ============================================================================
// Transform Command Tests
// ============================================================================

mod transform {
    use super::*;

    #[test]
    fn test_video_copy_webm_to_webm() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let output_path = temp_dir.path().join("output.webm");
        let output_str = output_path.to_str().unwrap();
        let input = test_asset("test_vp9.webm");

        // Copy VP9 video to new WebM
        let result = run_cli_text(&[
            "transform",
            &input,
            output_str,
            "-v",
            "copy",
            "-a",
            "none",
        ]);
        assert!(result.is_ok(), "Transform should succeed: {:?}", result);

        // Verify output file has correct stream info
        let json = run_cli(&["info", output_str, "-o", "json"])
            .expect("Failed to read output file");

        let format = &json["format"];
        assert_eq!(format["format_name"], "webm");
        assert_eq!(format["nb_streams"], 1);

        let streams = json["streams"].as_array().unwrap();
        assert_eq!(streams[0]["codec"], "vp9");
        assert_eq!(streams[0]["width"], 640);
        assert_eq!(streams[0]["height"], 480);
    }

    #[test]
    fn test_video_transcode_vp9_to_vp8() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let output_path = temp_dir.path().join("output.webm");
        let output_str = output_path.to_str().unwrap();
        let input = test_asset("test_vp9.webm");

        // Transcode VP9 to VP8
        let result = run_cli_text(&[
            "transform",
            &input,
            output_str,
            "-v",
            "vp8",
            "-a",
            "none",
        ]);
        assert!(result.is_ok(), "Transform should succeed: {:?}", result);

        // Verify output file has VP8 codec
        let json = run_cli(&["info", output_str, "-o", "json"])
            .expect("Failed to read output file");

        let streams = json["streams"].as_array().unwrap();
        assert_eq!(streams[0]["codec"], "vp8");
        assert_eq!(streams[0]["width"], 640);
        assert_eq!(streams[0]["height"], 480);

        // Verify we can decode the transcoded file
        let json_frames = run_cli(&["info", output_str, "-f", "-o", "json"])
            .expect("Failed to decode output file");
        let frames = json_frames["frames"].as_array().unwrap();
        assert!(!frames.is_empty(), "Output should have decodable frames");
    }

    #[test]
    fn test_video_transcode_vp8_to_vp9() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let output_path = temp_dir.path().join("output.webm");
        let output_str = output_path.to_str().unwrap();
        let input = test_asset("test_vp8.webm");

        // Transcode VP8 to VP9
        let result = run_cli_text(&[
            "transform",
            &input,
            output_str,
            "-v",
            "vp9",
            "-a",
            "none",
        ]);
        assert!(result.is_ok(), "Transform should succeed: {:?}", result);

        // Verify output file has VP9 codec
        let json = run_cli(&["info", output_str, "-o", "json"])
            .expect("Failed to read output file");

        let streams = json["streams"].as_array().unwrap();
        assert_eq!(streams[0]["codec"], "vp9");
        assert_eq!(streams[0]["width"], 640);
        assert_eq!(streams[0]["height"], 480);
    }

    #[test]
    fn test_audio_copy_webm_to_webm() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let output_path = temp_dir.path().join("output.webm");
        let output_str = output_path.to_str().unwrap();
        let input = test_asset("reference_opus.webm");

        // Copy Opus audio to new WebM
        let result = run_cli_text(&[
            "transform",
            &input,
            output_str,
            "-a",
            "copy",
            "--no-video",
        ]);
        assert!(result.is_ok(), "Transform should succeed: {:?}", result);

        // Verify output file has correct stream info
        let json = run_cli(&["info", output_str, "-o", "json"])
            .expect("Failed to read output file");

        let format = &json["format"];
        assert_eq!(format["format_name"], "webm");
        assert_eq!(format["nb_streams"], 1);

        let streams = json["streams"].as_array().unwrap();
        assert_eq!(streams[0]["codec"], "opus");
        assert_eq!(streams[0]["sample_rate"], 48000);
        assert_eq!(streams[0]["channels"], 2);
    }

    #[test]
    fn test_audio_extract_to_wav() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let output_path = temp_dir.path().join("output.wav");
        let output_str = output_path.to_str().unwrap();
        let input = test_asset("reference_opus.webm");

        // Extract Opus audio to WAV
        let result = run_cli_text(&[
            "transform",
            &input,
            output_str,
            "-a",
            "pcm",
            "--no-video",
        ]);
        assert!(result.is_ok(), "Transform should succeed: {:?}", result);

        // Verify output file is valid WAV
        let json = run_cli(&["info", output_str, "-o", "json"])
            .expect("Failed to read output file");

        let format = &json["format"];
        assert_eq!(format["format_name"], "wav");
        assert_eq!(format["nb_streams"], 1);

        let streams = json["streams"].as_array().unwrap();
        assert_eq!(streams[0]["codec"], "pcm");
        assert_eq!(streams[0]["sample_rate"], 48000);
        assert_eq!(streams[0]["channels"], 2);
    }

    #[test]
    fn test_transform_preserves_frame_count() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let output_path = temp_dir.path().join("output.webm");
        let output_str = output_path.to_str().unwrap();
        let input = test_asset("test_vp9.webm");

        // Copy video (should preserve frame count)
        run_cli_text(&[
            "transform",
            &input,
            output_str,
            "-v",
            "copy",
            "-a",
            "none",
        ])
        .expect("Transform should succeed");

        // Count packets in output
        let json = run_cli(&["info", output_str, "-p", "-o", "json"])
            .expect("Failed to read output file");

        let packets = json["packets"].as_array().unwrap();
        assert_eq!(
            packets.len(),
            30,
            "Copied file should have same packet count as source"
        );
    }
}

// ============================================================================
// Text Output Tests
// ============================================================================

mod text_output {
    use super::*;

    #[test]
    fn test_info_text_output_contains_format() {
        let asset = test_asset("test_vp9.webm");
        let output = run_cli_text(&["info", &asset])
            .expect("Failed to run info command");

        assert!(output.contains("Format:"), "Output should contain Format:");
        assert!(output.contains("webm"), "Output should contain 'webm'");
    }

    #[test]
    fn test_info_text_output_contains_stream() {
        let asset = test_asset("test_vp9.webm");
        let output = run_cli_text(&["info", &asset])
            .expect("Failed to run info command");

        assert!(
            output.contains("Stream #0"),
            "Output should contain Stream #0"
        );
        assert!(output.contains("vp9"), "Output should contain 'vp9'");
        assert!(output.contains("640x480"), "Output should contain dimensions");
    }

    #[test]
    fn test_info_packet_text_output() {
        let asset = test_asset("test_vp9.webm");
        let output = run_cli_text(&["info", &asset, "-p", "-n", "5"])
            .expect("Failed to run info command");

        assert!(
            output.contains("Packets:"),
            "Output should contain Packets header"
        );
        assert!(output.contains("PKT"), "Output should contain PKT column");
        assert!(output.contains("video"), "Output should contain video type");
    }

    #[test]
    fn test_info_frame_text_output() {
        let asset = test_asset("test_vp9.webm");
        let output = run_cli_text(&["info", &asset, "-f", "-n", "5"])
            .expect("Failed to run info command");

        assert!(
            output.contains("Frames:"),
            "Output should contain Frames header"
        );
        assert!(output.contains("FRAME"), "Output should contain FRAME column");
    }
}
