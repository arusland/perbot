pub fn init() {
    let log_dir = std::env::var("LOG_DIR").unwrap_or_else(|_| "logs".to_string());
    flexi_logger::Logger::try_with_env_or_str("info")
        .expect("Failed to initialize logger")
        .log_to_file(flexi_logger::FileSpec::default().directory(&log_dir))
        .rotate(
            flexi_logger::Criterion::Age(flexi_logger::Age::Day),
            flexi_logger::Naming::Timestamps,
            flexi_logger::Cleanup::KeepLogFiles(365),
        )
        .format_for_files(log_format)
        .format_for_stdout(log_format)
        .duplicate_to_stdout(flexi_logger::Duplicate::All)
        .start()
        .expect("Failed to start logger");
}

fn log_format(
    w: &mut dyn std::io::Write,
    now: &mut flexi_logger::DeferredNow,
    record: &log::Record,
) -> std::io::Result<()> {
    write!(
        w,
        "[{}] {:5} [{}:{}] {}",
        now.format("%Y-%m-%d %H:%M:%S"),
        record.level(),
        record.module_path().unwrap_or("<unknown>"),
        record.line().unwrap_or(0),
        record.args()
    )
}
