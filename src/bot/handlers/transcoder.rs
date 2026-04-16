use std::path::Path;
use ffmpeg_next as ffmpeg;
use ffmpeg::codec::Id;
use ffmpeg::{format, media, software::scaling, util::frame::video::Video};
use image::ImageReader;
use image::imageops::FilterType;
use image::GenericImageView;

pub fn init_ffmpeg() -> Result<(), String> {
    ffmpeg::init().map_err(|e| format!("Failed to init FFmpeg: {}", e))
}

pub fn convert_image_to_webp(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let img = ImageReader::open(input_path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;

    let (width, height) = img.dimensions();
    let (new_width, new_height) = if width > height {
        (512, (height as f32 * 512.0 / width as f32) as u32)
    } else {
        ((width as f32 * 512.0 / height as f32) as u32, 512)
    };

    let resized = img.resize_exact(new_width, new_height, FilterType::Lanczos3);

    resized
        .save_with_format(output_path, image::ImageFormat::WebP)
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn convert_video_to_webm(input_path: &Path, output_path: &Path) -> Result<(), String> {
    // Open input
    let mut ictx = format::input(&input_path).map_err(|e| format!("Input format error: {}", e))?;

    // Find video stream
    let input_stream = ictx
        .streams()
        .best(media::Type::Video)
        .ok_or("Could not find video stream")?;
    let video_stream_index = input_stream.index();
    
    // Get input timing info
    let input_time_base = input_stream.time_base();
    let input_frame_rate = input_stream.avg_frame_rate();
    let fps = if input_frame_rate.numerator() > 0 && input_frame_rate.denominator() > 0 {
        input_frame_rate.numerator() as f64 / input_frame_rate.denominator() as f64
    } else {
        30.0 // default fallback
    };

    // Setup decoder
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(input_stream.parameters())
        .map_err(|e| e.to_string())?
        .decoder()
        .video()
        .map_err(|e| e.to_string())?;

    let (width, height) = (decoder.width(), decoder.height());
    let (new_width, new_height) = if width > height {
        let h = ((height as f64 * 512.0 / width as f64).round() as u32).max(1);
        (512, h)
    } else {
        let w = ((width as f64 * 512.0 / height as f64).round() as u32).max(1);
        (w, 512)
    };

    // Setup output
    let mut octx = format::output(&output_path).map_err(|e| format!("Output format error: {}", e))?;
    
    let codec = ffmpeg::encoder::find(Id::VP9).ok_or("VP9 encoder not found")?;
    let global_header = octx.format().flags().contains(ffmpeg::format::flag::Flags::GLOBAL_HEADER);

    // Configure encoder
    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|e| e.to_string())?;

    // Use a fixed time base for predictable output timing
    // Time base 1/1000 means each unit = 1ms
    let output_time_base = ffmpeg::Rational::new(1, 1000);
    
    encoder.set_width(new_width);
    encoder.set_height(new_height);
    encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    encoder.set_time_base(output_time_base);
    encoder.set_bit_rate(256_000);
    
    // Set frame rate
    if input_frame_rate.numerator() > 0 && input_frame_rate.denominator() > 0 {
        encoder.set_frame_rate(Some(input_frame_rate));
    }
    
    if global_header {
        encoder.set_flags(ffmpeg::codec::flag::Flags::GLOBAL_HEADER);
    }
    
    let mut encoder = encoder.open_as(codec).map_err(|e| format!("Failed to open encoder: {}", e))?;

    // Add output stream
    let mut out_stream = octx.add_stream(codec).map_err(|e| e.to_string())?;
    out_stream.set_parameters(&encoder);
    out_stream.set_time_base(output_time_base);
    let out_stream_index = out_stream.index();

    octx.write_header().map_err(|e| e.to_string())?;

    // Setup scaler
    let mut scaler = scaling::Context::get(
        decoder.format(),
        width,
        height,
        ffmpeg::format::Pixel::YUV420P,
        new_width,
        new_height,
        scaling::flag::Flags::BICUBIC,
    ).map_err(|e| e.to_string())?;

    // Track output PTS (in output time base units = milliseconds)
    let mut output_pts: i64 = 0;
    let frame_duration_ms = (1000.0 / fps).round() as i64;
    
    // Track first PTS to calculate relative time
    let mut first_pts: Option<i64> = None;
    let max_duration_ms: i64 = 3000; // 3 seconds in ms

    // Process packets
    for (stream, packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue;
        }

        // Get packet timestamp
        let packet_pts = match packet.pts() {
            Some(pts) => pts,
            None => continue,
        };

        // Calculate relative time from first frame
        let relative_pts = if let Some(first) = first_pts {
            packet_pts - first
        } else {
            first_pts = Some(packet_pts);
            0
        };

        // Convert to milliseconds
        let relative_time_ms = relative_pts as f64 
            * input_time_base.numerator() as f64 
            / input_time_base.denominator() as f64 
            * 1000.0;

        // Skip if past 3 seconds
        if relative_time_ms > max_duration_ms as f64 {
            continue;
        }

        // Decode
        decoder.send_packet(&packet).map_err(|e| e.to_string())?;
        
        let mut decoded_frame = Video::empty();
        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            // Scale frame
            let mut scaled_frame = Video::empty();
            scaler.run(&decoded_frame, &mut scaled_frame).map_err(|e| e.to_string())?;
            
            // Set output PTS (sequential, starting from 0)
            scaled_frame.set_pts(Some(output_pts));
            
            // Encode
            encoder.send_frame(&scaled_frame).map_err(|e| e.to_string())?;
            
            let mut encoded_packet = ffmpeg::codec::packet::Packet::empty();
            while encoder.receive_packet(&mut encoded_packet).is_ok() {
                encoded_packet.set_stream(out_stream_index);
                
                // Set PTS/DTS if missing
                if encoded_packet.pts().is_none() {
                    encoded_packet.set_pts(Some(output_pts));
                }
                if encoded_packet.dts().is_none() {
                    encoded_packet.set_dts(encoded_packet.pts());
                }
                
                encoded_packet.write_interleaved(&mut octx).map_err(|e| e.to_string())?;
            }
            
            // Advance output PTS by frame duration
            output_pts += frame_duration_ms;
        }
    }
    
    // Flush encoder
    encoder.send_eof().map_err(|e| e.to_string())?;
    let mut encoded_packet = ffmpeg::codec::packet::Packet::empty();
    while encoder.receive_packet(&mut encoded_packet).is_ok() {
        encoded_packet.set_stream(out_stream_index);
        
        if encoded_packet.pts().is_none() {
            encoded_packet.set_pts(Some(output_pts));
        }
        if encoded_packet.dts().is_none() {
            encoded_packet.set_dts(encoded_packet.pts());
        }
        
        encoded_packet.write_interleaved(&mut octx).map_err(|e| e.to_string())?;
    }
    
    octx.write_trailer().map_err(|e| e.to_string())?;

    Ok(())
}
