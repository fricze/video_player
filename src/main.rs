extern crate ffmpeg_next as ffmpeg;
use ffmpeg::rescale::TIME_BASE;
use sdl2::rect::Rect;

extern crate sdl2;
use ffmpeg::ffi::{avio_seek, AVFormatContext, AVSEEK_SIZE};
use ffmpeg::util::frame::Video;
use ffmpeg::util::rational::Rational;

use sdl2::pixels::PixelFormatEnum;
use sdl2::render::TextureAccess;
use std::collections::BTreeMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::thread;

fn format_duration(duration_secs: f64) -> String {
    // let duration_secs = duration as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;

    let hours = (duration_secs / 3600.0).floor() as u32;
    let minutes = ((duration_secs % 3600.0) / 60.0).floor() as u32;
    let seconds = (duration_secs % 60.0).floor() as u32;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn fps_to_ms(fps: f64) -> f64 {
    if fps == 0.0 {
        panic!("FPS cannot be zero for frame duration calculation");
    }
    1000.0 / fps
}

fn rational_fps_to_ms(rate: Rational) -> f64 {
    if rate.numerator() == 0 {
        panic!("FPS numerator cannot be zero for frame duration calculation");
    }
    1000.0 * (rate.denominator() as f64 / rate.numerator() as f64)
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
    let ictx = ffmpeg::format::input(&input_path)?;
    let video_stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("Could not find a video stream")?
        .index();

    let stream = ictx.stream(video_stream_index).unwrap();
    let parameters = stream.parameters();
    let codec_context = ffmpeg::codec::context::Context::from_parameters(parameters)?;
    let time_base = stream.time_base();

    let avg_frame_rate = stream.avg_frame_rate();

    // println!("avg_frame_rate {}", frame_rate);

    let decoder = codec_context.decoder().video().unwrap();

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

    let video_frames = Arc::new(Mutex::new(BTreeMap::<i64, Vec<Video>>::new()));

    // Worker thread for decoding frames and converting to textures
    let frame_sender = Arc::clone(&video_frames);
    let input_path_clone = input_path.clone();
    thread::spawn(move || {
        let mut ictx = ffmpeg::format::input(&input_path_clone).unwrap();

        let position = 0;
        ictx.seek(position, ..).unwrap();

        let avio_ctx = unsafe {
            let format_ctx: *mut AVFormatContext = ictx.as_mut_ptr();
            (*format_ctx).pb
        };
        if avio_ctx.is_null() {
            println!("No AVIOContext found");
        } else {
            let position = unsafe { avio_seek(avio_ctx, 0, AVSEEK_SIZE) };
            println!("position: {}", position);
        }

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

        let frame_rate = codec_context.frame_rate();
        let ms_per_frame = {
            if frame_rate.numerator() == 0 {
                rational_fps_to_ms(avg_frame_rate)
            } else {
                rational_fps_to_ms(frame_rate)
            }
        };

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
                        let pts = new_frame.pts().unwrap_or(0);
                        frame_sender_lock
                            .entry(pts)
                            .or_default()
                            .push(new_frame.clone());

                        std::thread::sleep(std::time::Duration::from_millis(ms_per_frame as u64));
                    }
                }
            }
        }
    });

    let mut quit = false;
    let mut event_pump = sdl_context.event_pump()?;

    let ttf_context = sdl2::ttf::init().unwrap_or_else(|e| {
        eprintln!("Failed to initialize SDL2 TTF: {}", e);
        std::process::exit(1);
    });
    let font = ttf_context.load_font("./WorkSans-Regular.ttf", 16)?;

    let video_duration = ictx.duration();

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
            if !frames_lock.is_empty() {
                // Get the smallest PTS (first in order)
                if let Some((&pts, frames)) = frames_lock.iter().next() {
                    if let Some(video_frame) = frames.get(0) {
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
                            .unwrap();

                        canvas.clear();
                        canvas.copy(&texture, None, None).unwrap();

                        let video_duration = video_duration / ffmpeg::ffi::AV_TIME_BASE as i64;
                        let pts_in_seconds = pts as f64 * time_base.numerator() as f64
                            / time_base.denominator() as f64;

                        // Calculate progress
                        let progress = pts_in_seconds as f64 / video_duration as f64 * 100.0;

                        // println!("pts_in_sec : {}", pts_in_seconds);
                        // println!("duration : {}", video_duration);
                        // println!("progress: {}", progress);

                        let width = decoder.width() as f64 / 100.0 * (progress as f64);

                        let progress_rect =
                            Rect::new(0, decoder.height() as i32 - 25, width as u32, 5);
                        canvas.set_draw_color(sdl2::pixels::Color::RGB(0, 255, 0));
                        canvas.fill_rect(progress_rect)?;

                        // Render duration text
                        let surface = font
                            .render(&format_duration(video_duration as f64))
                            .blended(sdl2::pixels::Color::RGB(255, 255, 255))?;

                        let texture = texture_creator.create_texture_from_surface(&surface)?;
                        canvas.copy(
                            &texture,
                            None,
                            Some(Rect::new(
                                decoder.width() as i32 - 100,
                                decoder.height() as i32 - 20,
                                100,
                                20,
                            )),
                        )?;

                        canvas.present();

                        // Remove the frame after it's been displayed or move to the next if there are multiple at the same PTS
                        if frames.len() == 1 {
                            frames_lock.remove(&pts);
                        } else {
                            let frames = frames_lock.get_mut(&pts).unwrap();
                            frames.remove(0);
                            if frames.is_empty() {
                                frames_lock.remove(&pts);
                            }
                        }
                    }
                }
            }
        }
    }

    println!("Playback Finished!");
    Ok(())
}
