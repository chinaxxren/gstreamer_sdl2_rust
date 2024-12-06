//
// 使用 Rust 和 SDL2 处理视频、使用标准音频处理音频的媒体播放器。
//
// 待办事项：
// *保持纵横比
// *按照某种“游戏”设计重新设计。
// *通过重新设计，可以显示音量变化和静音等内容。
//
use std::env;
use std::path::Path;
use std::process;
use url::Url;
use sdl2::pixels::Color;
use sdl2::pixels::PixelFormatEnum;
use sdl2::video::FullscreenType;
use sdl2::rect::Rect;
use std::time::{Duration, Instant};
use gstreamer::prelude::*;

// 导入必要的标准库和外部依赖

// 定义窗口的默认宽度和高度
const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

// 定义一个宏来简化SDL2的Rect创建
// 将x, y, width, height转换为适当的类型并创建一个新的Rect
macro_rules! rect(
    ($x:expr, $y:expr, $w:expr, $h:expr) => (
        Rect::new($x as i32, $y as i32, $w as u32, $h as u32)
    )
);

fn main() {
    // 获取命令行参数
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Missing filename");
        process::exit(-1);
    }

    // 获取输入文件或URL
    let input = &args[1];
    // 根据输入类型构建合适的source字符串
    let source =
        if let Ok(url) = Url::parse(input) {
            let host = url.host_str().unwrap();
            println!("host: {}", host);
            // 如果是YouTube链接，使用特殊处理
            if host.contains("youtu") {
                format!("urisourcebin uri={}", input)
            } else {
                format!("urisourcebin uri={}", input)
            }
        } else if Path::new(input).exists() {
            // 如果是本地文件
            format!("filesrc location={}", input)
        } else {
            println!("Cannot open {}", input);
            process::exit(-1);
        };

    // 初始化SDL2及其子系统
    let sdl_context = sdl2::init().unwrap();
    // 初始化视频子系统 
    let video_subsystem = sdl_context.video().unwrap();
    // 初始化字体子系统
    let ttf_context = sdl2::ttf::init().unwrap();
    // 获取事件泵
    let mut event_pump = sdl_context.event_pump().unwrap();

    // 创建窗口
    let window = video_subsystem.window(&args[0], WIDTH, HEIGHT)
        .position_centered()// 居中 
        .resizable()// 可调整大小
        .build()// 创建窗口
        .unwrap();

    // 创建渲染器
    let mut canvas = window.into_canvas()
        .accelerated()// 使用硬件加速
        .present_vsync()// 垂直同步
        .build()// 创建渲染器
        .unwrap();

    // 创建纹理生成器
    let texture_creator = canvas.texture_creator();

    // 初始化FPS显示相关的组件
    let mut fps = format!("   FPS");
    // 加载字体
    let font = ttf_context.load_font("sansfont.ttf", 14).unwrap();
    // 渲染FPS文本
    let surface = font
        .render(&fps)
        .blended(Color::RGBA(255, 255, 255, 255))
        .unwrap();
    
    // 创建FPS纹理
    let mut fps_tex = texture_creator.create_texture_from_surface(&surface).unwrap();
    // 获取FPS纹理的查询结果
    let tex_query = fps_tex.query();
    // 设置FPS纹理的目标矩形
    let mut fps_dst = rect!(0, 0, tex_query.width, tex_query.height);

    // 初始化GStreamer
    gstreamer::init().unwrap();

    // 构建GStreamer管道字符串
    // 使用decodebin解码视频流
    // 使用autovideoconvert将视频转换为I420格式
    // 使用appsink将视频帧发送到Rust
    let pipeline_str = format!("{} ! \
                               decodebin name=dmux \
                               dmux. ! queue ! autovideoconvert ! video/x-raw,format=I420 ! appsink name=sink \
                               dmux. ! queue ! audioconvert ! volume name=volume ! autoaudiosink",
                               source);
    // 创建解析上下文
    let mut context = gstreamer::ParseContext::new();
    
    // 创建并解析GStreamer管道
    let pipeline =
        // 解析管道
        match gstreamer::parse_launch_full(&pipeline_str, Some(&mut context), gstreamer::ParseFlags::empty()) {
            Ok(pipeline) => pipeline,
            Err(err) => {
                // 如果缺少元素，打印缺少的元素
                if let Some(gstreamer::ParseError::NoSuchElement) = err.kind::<gstreamer::ParseError>() {
                    println!("Missing element(s): {:?}", context.missing_elements());
                } else {
                    // 如果解析失败，打印错误信息
                    println!("Failed to parse pipeline: {}", err);
                }
                // 退出程序
                process::exit(-1)
            }
        };

    // 获取管道和相关元素
    let pipeline = pipeline.dynamic_cast::<gstreamer::Pipeline>().unwrap();
    // 获取sink元素
    let sink = pipeline.by_name("sink").unwrap();
    // 获取volume元素
    let volume = pipeline.by_name("volume").unwrap();
    // 获取appsink元素
    let appsink = sink.dynamic_cast::<gstreamer_app::AppSink>().unwrap();

    // 启动管道
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");

    println!("Pipeline playing...");

    // 获取管道的消息总线
    let bus = pipeline.bus().unwrap();
    // 初始化播放标志   
    let mut playing = true;
    // 初始化帧计数
    let mut frames: u32 = 0;
    // 初始化视频尺寸
    let mut width = WIDTH;
    // 初始化视频高度
    let mut height = HEIGHT;
    // 创建视频纹理
    let mut tex = texture_creator.create_texture_streaming(PixelFormatEnum::IYUV, width, height).unwrap();
    // 获取当前时间
    let mut start = Instant::now();

    // 主循环
    'running: loop {
        // 处理GStreamer消息
        for msg in bus.iter() {
            // 使用MessageView匹配消息类型
            use gstreamer::MessageView;
            // 匹配消息类型
            match msg.view() {
                // 流结束
                MessageView::Eos(..) => break 'running, 
                // 错误处理
                    MessageView::Error(err) => {   
                    println!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    break 'running;
                }
                // 其他情况
                _ => (),
            }
        }

        // 处理SDL2事件
        for event in event_pump.poll_iter() {
            use sdl2::event::Event;
            use sdl2::keyboard::Keycode;

            match event {
                // 退出事件处理
                Event::Quit {..} |
                Event::KeyDown { keycode: Some(Keycode::Q), .. } |
                Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                    break 'running
                },
                // 静音控制
                Event::KeyDown { keycode: Some(Keycode::M), .. } => {
                    // 按M键将音量设置为0（静音）
                    let v: f64 = 0.0;
                    volume.set_property("volume", &v);
                },
                // 音量增加
                Event::KeyDown { keycode: Some(Keycode::PageUp), .. } => {
                    // 按PageUp键增加音量（每次增加0.1，最大值为1.0）
                    let mut v: f64 = volume.property_value("volume").get().unwrap();
                    v = (v + 0.1).clamp(0.0, 1.0);
                    volume.set_property("volume", &v);
                },
                // 音量减少
                Event::KeyDown { keycode: Some(Keycode::PageDown), .. } => {
                    let mut v: f64 = volume.property_value("volume").get().unwrap();
                    v = (v - 0.1).clamp(0.0, 1.0);
                    volume.set_property("volume", &v);
                },
                // 全屏切换
                Event::KeyDown { keycode: Some(Keycode::F), .. } => {
                    let window = canvas.window_mut();
                    match window.fullscreen_state() {
                        FullscreenType::True |
                        FullscreenType::Desktop => window.set_fullscreen(FullscreenType::Off).unwrap(),
                        FullscreenType::Off => window.set_fullscreen(FullscreenType::True).unwrap(),
                    }
                },
                // 播放/暂停切换
                Event::KeyDown { keycode: Some(Keycode::Space), .. } => {
                    if playing {
                        playing = false;
                        pipeline
                        .set_state(gstreamer::State::Paused)
                        .expect("Unable to set the pipeline to the `Paused` state");
                        println!("Pipeline paused...");
                    } else {
                        playing = true;
                        pipeline
                        .set_state(gstreamer::State::Playing)
                        .expect("Unable to set the pipeline to the `Playing` state");
                        println!("Pipeline playing...");
                    }
                },
                _ => {}
            }
        }

        // 如果暂停，继续下一次循环
        if !playing {
            continue 'running;
        }

        // 尝试获取视频样本并处理
        match appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(40)) {
            Some(sample) => {
                // 获取视频帧数据
                let buffer = sample.buffer().unwrap();
                // 获取样本的caps
                let caps = sample.caps().expect("Sample without caps");
                // 从caps中解析视频信息
                let info = gstreamer_video::VideoInfo::from_caps(caps).expect("Failed to parse caps");
                // 从缓冲区中创建视频帧
                let frame = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).unwrap();

                // 如果视频尺寸改变，更新纹理
                if frame.width() != width || frame.height() != height {
                    println!("Video negotiated {}x{}", frame.width(), frame.height());
                    println!("{} planes", frame.n_planes());

                    width = frame.width();
                    height = frame.height();
                    // 创建新的纹理
                    tex = texture_creator.create_texture_streaming(PixelFormatEnum::IYUV, width, height).unwrap();
                }

                // 更新视频帧
                if width > 0 && height > 0 {
                    // 更新YUV纹理数据
                    tex.update_yuv(None,
                                   frame.plane_data(0).unwrap(),
                                   frame.plane_stride()[0] as usize,
                                   frame.plane_data(1).unwrap(),
                                   frame.plane_stride()[1] as usize,
                                   frame.plane_data(2).unwrap(),
                                   frame.plane_stride()[2] as usize)
                        .unwrap();
                    // 清除画布并绘制新帧
                    canvas.clear();
                    // 绘制视频帧
                    canvas.copy(&tex, None, None).unwrap();
                    // 绘制FPS文本
                    canvas.copy(&fps_tex, None, Some(fps_dst)).unwrap();
                    // 显示绘制结果
                    canvas.present();
                    // 增加帧计数
                    frames = frames + 1;
                }
            },
            None => {
                // 检查是否到达流的末尾
                if appsink.is_eos() {
                    break 'running;
                }
            },
        };

        // 更新FPS显示
        let elapsed = start.elapsed();
        // 如果时间超过1秒
        if elapsed >= Duration::new(1, 0) {
            fps = format!("{} FPS", frames as u64 / elapsed.as_secs());
            // 渲染FPS文本
            let surface = font
                .render(&fps)
                .blended(Color::RGBA(255, 255, 255, 255))
                .unwrap();
            // 创建FPS纹理
            fps_tex = texture_creator.create_texture_from_surface(&surface).unwrap();
            // 获取FPS纹理的查询结果
            let tex_query = fps_tex.query();
            // 设置FPS纹理的目标矩形
            fps_dst = rect!(0, 0, tex_query.width, tex_query.height);
            // 重置时间
            start = Instant::now();
            // 重置帧计数
            frames = 0;
        }
    };

    // 关闭管道
    pipeline
        .set_state(gstreamer::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");

    println!("Pipeline stopped...");
}
