mod app;
mod camera;
mod capture;
mod h264_preview;
mod photo;
mod recorder;

use anyhow::Result;
use std::io::Write;
use std::sync::Mutex;

struct TeeWriter {
    console: std::io::Stderr,
    file: Mutex<std::fs::File>,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.console.write_all(buf);
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = self.console.flush();
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    // [TIMING] 启动总计时起点
    let t_start = std::time::Instant::now();

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("camcap-run.log")
        .ok();

    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("debug"),
    );

    if let Some(f) = log_file {
        builder.target(env_logger::Target::Pipe(Box::new(TeeWriter {
            console: std::io::stderr(),
            file: Mutex::new(f),
        })));
    }

    builder.init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("CamCap"),
        ..Default::default()
    };

    log::info!("[TIMING] main 入口到 run_native 前: {:?}", t_start.elapsed());

    eframe::run_native(
        "CamCap",
        options,
        Box::new(move |cc| {
            let t_ui = std::time::Instant::now();
            let app = app::CamCapApp::new(cc);
            log::info!("[TIMING] CamCapApp::new: {:?}", t_ui.elapsed());
            log::info!(
                "[TIMING] 启动总耗时（main → UI 就绪）: {:?}",
                t_start.elapsed()
            );
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}