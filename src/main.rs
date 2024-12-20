extern crate ffmpeg_next as ffmpeg;

extern crate sdl2;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::TextureAccess;
use std::env;
use std::sync::{Arc, Mutex};
use std::thread;

fn _format_duration(duration: i64) -> String {
    let duration_secs = duration as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;

    let hours = (duration_secs / 3600.0).floor() as u32;
    let minutes = ((duration_secs % 3600.0) / 60.0).floor() as u32;
    let seconds = (duration_secs % 60.0).floor() as u32;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

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
    let input_context = ffmpeg::format::input(&input_path)?;
    let video_stream_index = input_context
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("Could not find a video stream")?
        .index();

    let codec_context = ffmpeg::codec::context::Context::from_parameters(
        input_context
            .stream(video_stream_index)
            .unwrap()
            .parameters(),
    )?;
    let decoder = codec_context.decoder().video().unwrap();

    // let duration = format_duration(input_context.duration());
    // println!("{}", duration);

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

    let video_frames = Arc::new(Mutex::new(Vec::<ffmpeg::util::frame::Video>::new()));

    // Worker thread for decoding frames and converting to textures
    let frame_sender = Arc::clone(&video_frames);
    let input_path_clone = input_path.clone();
    thread::spawn(move || {
        let mut ictx = ffmpeg::format::input(&input_path_clone).unwrap();

        let position = 93184;
        ictx.seek(position, ..).unwrap();

        let video_stream_index = ictx
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or("Could not find a video stream")
            .unwrap()
            .index();

        let codec_context = ffmpeg::codec::context::Context::from_parameters(
            ictx.stream(video_stream_index).unwrap().parameters(),
        )
        .unwrap();
        let mut decoder = codec_context.decoder().video().unwrap();
        let mut new_frame = ffmpeg::util::frame::Video::empty();

        // Flush the decoder to ensure we start fresh after seeking
        decoder.flush();

        for (stream, packet) in ictx.packets() {
            if stream.index() == video_stream_index {
                decoder.send_packet(&packet).unwrap();
                while decoder.receive_frame(&mut new_frame).is_ok() {
                    if packet.pts().unwrap_or(0) >= position {
                        let mut frame_sender_lock = frame_sender.lock().unwrap();
                        frame_sender_lock.push(new_frame.clone());

                        // Simulate ~25 FPS (40ms per frame)
                        std::thread::sleep(std::time::Duration::from_millis(40));
                    }
                }
            }
        }
    });

    let mut quit = false;
    let mut event_pump = sdl_context.event_pump()?;

    'main: loop {
        for event in event_pump.poll_iter() {
            // println!("{:?}", event);

            match event {
                sdl2::event::Event::Quit { .. } => {
                    quit = true;
                    break;
                }
                _ => (),
            }
        }

        if quit {
            break 'main;
        }

        {
            let mut frames_lock = video_frames.lock().unwrap();
            if let Some(video_frame) = frames_lock.pop() {
                texture
                    .update_yuv(
                        None,
                        video_frame.data(0),
                        video_frame.stride(0) as usize,
                        video_frame.data(1),
                        video_frame.stride(1) as usize,
                        video_frame.data(2),
                        video_frame.stride(2) as usize,
                    )
                    .unwrap_or_else(|e| {
                        eprintln!("Failed to update SDL2 texture: {}", e);
                        std::process::exit(1);
                    });

                canvas.clear();
                canvas.copy(&texture, None, None).unwrap();

                canvas.present();
            }
        }
    }

    println!("Playback Finished!");
    Ok(())
}
