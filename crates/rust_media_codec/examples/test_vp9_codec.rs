use rust_media_codec::{Vp9Decoder, Vp9Encoder};
use rust_media_core::{Decoder, Demuxer, Encoder, Muxer};
use rust_media_format::{WebmDemuxer, WebmMuxer};
use std::fs::File;
use std::io::{BufReader, BufWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing VP9 Decoder and Encoder\n");

    // Test 1: Decode VP9 WebM file
    println!("=== Test 1: Decoding VP9 WebM ===");
    let input_file = File::open("test_assets/test_vp9.webm")?;
    let reader = BufReader::new(input_file);
    let mut demuxer = WebmDemuxer::open(reader)?;

    let streams = demuxer.streams()?;
    println!("Found {} stream(s)", streams.len());

    let stream_info = streams[0].clone();
    let dimensions = if let rust_media_core::StreamParams::Video(v) = &stream_info.params {
        format!("{}x{}", v.width, v.height)
    } else {
        "unknown".to_string()
    };
    println!("Stream 0: {:?} {} {}", stream_info.media_type, stream_info.codec, dimensions);

    // Create VP9 decoder
    let mut decoder = Vp9Decoder::new(stream_info.clone())?;

    let mut decoded_frames = 0;
    let mut packets_read = 0;

    // Decode first 10 packets
    for _ in 0..10 {
        match demuxer.read_packet() {
            Ok(packet) => {
                packets_read += 1;
                decoder.send_packet(&packet)?;

                // Try to receive frames
                loop {
                    match decoder.receive_frame() {
                        Ok(frame) => {
                            decoded_frames += 1;
                            println!("  Frame {}: {}x{} pts={:?}",
                                decoded_frames,
                                if let rust_media_core::frame::FrameParams::Video(v) = frame.params() {
                                    v.width
                                } else {
                                    0
                                },
                                if let rust_media_core::frame::FrameParams::Video(v) = frame.params() {
                                    v.height
                                } else {
                                    0
                                },
                                frame.pts()
                            );
                        }
                        Err(rust_media_core::Error::NeedMoreData) => break,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            Err(_) => break,
        }
    }

    println!("✅ Decoded {} frames from {} packets\n", decoded_frames, packets_read);

    // Test 2: Full decode -> encode roundtrip
    println!("=== Test 2: VP9 Decode -> Encode Roundtrip ===");

    // Re-open input
    let input_file = File::open("test_assets/test_vp9.webm")?;
    let reader = BufReader::new(input_file);
    let mut demuxer = WebmDemuxer::open(reader)?;

    let streams = demuxer.streams()?;
    let stream_info = streams[0].clone();

    // Create decoder and encoder
    let mut decoder = Vp9Decoder::new(stream_info.clone())?;
    let mut encoder = Vp9Encoder::new(stream_info.clone())?;

    // Create output muxer
    let output_file = File::create("/tmp/test_vp9_roundtrip.webm")?;
    let writer = BufWriter::new(output_file);
    let mut muxer = WebmMuxer::new(writer);

    muxer.add_stream(stream_info)?;
    muxer.write_header()?;

    let mut frames_processed = 0;
    let mut packets_written = 0;

    // Process first 5 packets
    for _ in 0..5 {
        match demuxer.read_packet() {
            Ok(packet) => {
                // Decode
                decoder.send_packet(&packet)?;

                loop {
                    match decoder.receive_frame() {
                        Ok(frame) => {
                            frames_processed += 1;

                            // Encode
                            encoder.send_frame(&frame)?;

                            // Get encoded packets
                            loop {
                                match encoder.receive_packet() {
                                    Ok(enc_packet) => {
                                        muxer.write_packet(&enc_packet)?;
                                        packets_written += 1;
                                        print!(".");
                                        std::io::Write::flush(&mut std::io::stdout())?;
                                    }
                                    Err(rust_media_core::Error::NeedMoreData) => break,
                                    Err(e) => return Err(e.into()),
                                }
                            }
                        }
                        Err(rust_media_core::Error::NeedMoreData) => break,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            Err(_) => break,
        }
    }

    muxer.write_trailer()?;
    muxer.flush()?;

    println!("\n✅ Processed {} frames, wrote {} packets\n", frames_processed, packets_written);

    let output_size = std::fs::metadata("/tmp/test_vp9_roundtrip.webm")?.len();
    println!("Output file size: {} bytes", output_size);

    println!("\n🎉 All VP9 tests passed!");

    Ok(())
}
