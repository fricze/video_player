extern crate ffmpeg_next as ffmpeg;

extern crate sdl2;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::TextureAccess;
use std::env;
use std::sync::{Arc, Mutex};
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ffmpeg::init().unwrap_or_else(|e| {
        eprintln!("Failed to initialize ffmpeg: {}", e);
        std::process::exit(1);
    });

    // Parse the input video file path from command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo run <path_to_video>");
        return Ok(());
    }
    let input_path = &args[1];

    // Open the input video file and find the best stream
    // that's super cool that you can just ask ffmpeg for "best" stream
    let ictx = ffmpeg::format::input(&input_path)?;
    let video_stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("Could not find a video stream")?
        .index();

    let codec_context = ffmpeg::codec::context::Context::from_parameters(
        ictx.stream(video_stream_index).unwrap().parameters(),
    )?;
    let decoder = codec_context.decoder().video().unwrap();

    // SDL2 Initialization
    let sdl_context = sdl2::init().unwrap_or_else(|e| {
        eprintln!("Failed to initialize SDL2: {}", e);
        std::process::exit(1);
    });
    let video_subsystem = sdl_context.video().unwrap();

    let window = video_subsystem
        .window(
            "Basic Video Player",
            decoder.width() as u32,
            decoder.height() as u32,
        )
        .position_centered()
        .resizable()
        .build()
        .unwrap_or_else(|e| {
            eprintln!("Failed to create SDL2 window: {}", e);
            std::process::exit(1);
        });

    // Create SDL2 Canvas
    let mut canvas = window.into_canvas().build().unwrap();

    // Create SDL2 Texture for video frame rendering
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture(
            PixelFormatEnum::IYUV,
            TextureAccess::Streaming,
            decoder.width() as u32,
            decoder.height() as u32,
        )
        .unwrap_or_else(|e| {
            eprintln!("Failed to create SDL2 texture: {}", e);
            std::process::exit(1);
        });

    // Frame decoding
    let mut scaler = ffmpeg::software::scaling::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::format::Pixel::YUV420P,
        decoder.width(),
        decoder.height(),
        ffmpeg::software::scaling::Flags::BILINEAR,
    )?;

    let mut event_pump = sdl_context.event_pump()?;

    let frame_sender = Arc::new(Mutex::new(Vec::<ffmpeg::util::frame::Video>::new()));

    // Worker thread for decoding frames and converting to textures
    let frame_sender_worker = Arc::clone(&frame_sender);
    let input_path_worker = input_path.to_string();
    thread::spawn(move || {
        let mut ictx_worker = ffmpeg::format::input(&input_path_worker).unwrap();
        let video_stream_index_worker = ictx_worker
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or("Could not find a video stream")
            .unwrap()
            .index();

        let codec_context_worker = ffmpeg::codec::context::Context::from_parameters(
            ictx_worker
                .stream(video_stream_index_worker)
                .unwrap()
                .parameters(),
        )
        .unwrap();
        let mut decoder_worker = codec_context_worker.decoder().video().unwrap();
        let mut frame_worker = ffmpeg::util::frame::Video::empty();

        // Process packets and decode frames
        for (stream, packet) in ictx_worker.packets() {
            if stream.index() == video_stream_index_worker {
                decoder_worker.send_packet(&packet).unwrap();
                while decoder_worker.receive_frame(&mut frame_worker).is_ok() {
                    // Send the decoded frame to the main thread for texture conversion
                    let mut frame_sender_lock = frame_sender_worker.lock().unwrap();
                    frame_sender_lock.push(frame_worker.clone());
                }
            }
        }
    });

    let mut quit = false;
    let mut frame_queue = Vec::new();

    // Main loop for decoding and rendering
    // 'main: for (stream, packet) in ictx.packets() {
    'main: loop {
        for event in event_pump.poll_iter() {
            if let sdl2::event::Event::Quit { .. } = event {
                println!("Quit event received");
                quit = true;
                break;
            }
        }
        if quit {
            break 'main;
        }

        {
            let mut frame_sender_lock = frame_sender.lock().unwrap();
            if let Some(decoded_frame) = frame_sender_lock.pop() {
                frame_queue.push(decoded_frame);
            } else {
                quit = true;
            }
        }

        if !frame_queue.is_empty() {
            // Take the next frame from the queue and perform texture conversion
            let frame = frame_queue.remove(0);
            let mut yuv_frame = ffmpeg::util::frame::Video::empty();
            scaler.run(&frame, &mut yuv_frame).unwrap();

            texture
                .update_yuv(
                    None,
                    yuv_frame.data(0),
                    yuv_frame.stride(0) as usize,
                    yuv_frame.data(1),
                    yuv_frame.stride(1) as usize,
                    yuv_frame.data(2),
                    yuv_frame.stride(2) as usize,
                )
                .unwrap();

            canvas.clear();
            canvas.copy(&texture, None, None).unwrap();
            canvas.present();

            // Simulate ~25 FPS (40ms per frame)
            std::thread::sleep(std::time::Duration::from_millis(40));
        }

        // if stream.index() == video_stream_index {
        //     decoder.send_packet(&packet)?;
        //     while decoder.receive_frame(&mut frame).is_ok() {
        //         let mut yuv_frame = ffmpeg::util::frame::Video::empty();
        //         scaler.run(&frame, &mut yuv_frame)?;

        //         // Update SDL2 texture with the YUV frame data
        //         texture.update_yuv(
        //             None,
        //             yuv_frame.data(0),
        //             yuv_frame.stride(0) as usize,
        //             yuv_frame.data(1),
        //             yuv_frame.stride(1) as usize,
        //             yuv_frame.data(2),
        //             yuv_frame.stride(2) as usize,
        //         )?;

        //         // Clear, copy texture, and present to screen
        //         canvas.clear();
        //         canvas.copy(&texture, None, None)?;
        //         canvas.present();

        //         // Simulate 40ms per frame (~25 FPS)
        //         std::thread::sleep(std::time::Duration::from_millis(40));
        //     }
        // }
    }

    println!("Playback Finished!");
    Ok(())
}
