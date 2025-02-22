use program::Program;

mod context;
mod pipeline;
mod program;
mod surface;
mod ui;
mod util;
mod window_texture;

use tracing_setup::tracing::debug;

fn main() {
    let _ = setup_tracing();
    debug!("Starting app");

    let program = pollster::block_on(Program::new());
    program.run();
}

fn setup_tracing() -> Option<tracing_setup::tracing_appender::non_blocking::WorkerGuard> {
    let config = tracing_setup::TracingConfig {
        tracing_mode: tracing_setup::TracingMode::Console,
        env_filter: Some("egui_blur_demo=trace".to_string()),
        json: false,                   // we don't want json formatting
        log_dir: "./logs".to_string(), // logs will be created here
        ansi_file: false,              // we don't want ANSI colors in the file logs
        ansi_console: true,            // we want ANSI colors in the console logs
        lossy_file: true,
    };

    tracing_setup::init_tracing(config)
}
