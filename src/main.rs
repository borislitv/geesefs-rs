#![forbid(unsafe_code)]

use clap::Parser;
use geesefs_rs::{
    backend::s3::S3,
    config::{Config, normalize_fstab},
    filesystem::Filesystem,
    fuse::Fuse,
    metrics::{MetricsServer, allocation_stats},
};
use std::{sync::Arc, time::Duration};

#[global_allocator]
#[cfg(not(feature = "system-allocator"))]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[global_allocator]
#[cfg(feature = "system-allocator")]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn spawn_stats(
    runtime: &tokio::runtime::Runtime,
    fs: &Arc<Filesystem>,
) -> std::io::Result<Option<tokio::task::JoinHandle<()>>> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let Some(path) = &fs.config.stats_file else {
        return Ok(None);
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let fs = fs.clone();
    Ok(Some(runtime.spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(50));
        loop {
            timer.tick().await;
            let mut sample = allocation_stats();
            sample["unix_time_ns"] = serde_json::json!(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            sample["monotonic_source"] = "interval-50ms".into();
            sample["effective_budgets"] = serde_json::json!(fs.effective_budgets());
            if writeln!(file, "{sample}").is_err() {
                break;
            }
        }
    })))
}

fn spawn_metrics(
    runtime: &tokio::runtime::Runtime,
    fs: &Arc<Filesystem>,
) -> std::io::Result<Option<MetricsServer>> {
    let Some(address) = fs.config.metrics_listen else {
        return Ok(None);
    };
    let server = runtime.block_on(MetricsServer::bind(address, fs.clone()))?;
    eprintln!("metrics listening on http://{}/metrics", server.address());
    Ok(Some(server))
}

fn kernel_mount_option(option: &str) -> Result<fuser::MountOption, Box<dyn std::error::Error>> {
    Ok(match option {
        "ro" => fuser::MountOption::RO,
        "rw" => fuser::MountOption::RW,
        "nosuid" => fuser::MountOption::NoSuid,
        "nodev" => fuser::MountOption::NoDev,
        "noexec" => fuser::MountOption::NoExec,
        _ => return Err("unsupported mount option".into()),
    })
}

fn mount_options(config: &Config) -> Result<fuser::Config, Box<dyn std::error::Error>> {
    let mut options = fuser::Config::default();
    options
        .mount_options
        .push(fuser::MountOption::FSName("geesefs-rs".into()));
    options
        .mount_options
        .push(fuser::MountOption::DefaultPermissions);
    for option in &config.mount_options {
        match option.as_str() {
            "allow_other" => options.acl = fuser::SessionACL::All,
            "default_permissions" => {}
            option => options.mount_options.push(kernel_mount_option(option)?),
        }
    }
    Ok(options)
}

fn spawn_signals(
    runtime: &tokio::runtime::Runtime,
    mut unmounter: fuser::SessionUnmounter,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM registration");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
        let _ = tokio::task::spawn_blocking(move || unmounter.unmount()).await;
    })
}

fn spawn_flush(
    runtime: &tokio::runtime::Runtime,
    fs: Arc<Filesystem>,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            if fs.is_closing() {
                break;
            }
            if let Err(error) = fs.flush_all().await {
                eprintln!("background flush failed (dirty data retained): {error}");
            }
        }
    })
}

fn configuration() -> Result<Config, Box<dyn std::error::Error>> {
    let config = Config::parse_from(normalize_fstab(std::env::args_os().collect()));
    config.validate()?;
    if !config.foreground {
        return Err("background daemonization is unsupported; specify -f".into());
    }
    if !config.mountpoint.is_dir() {
        return Err("mountpoint must be an existing directory".into());
    }
    Ok(config)
}

fn connect_store(
    runtime: &tokio::runtime::Runtime,
    config: &Config,
) -> Result<Arc<S3>, Box<dyn std::error::Error>> {
    if config.no_verify_ssl {
        eprintln!("WARNING: S3 TLS certificate verification is disabled by --no-verify-ssl");
    }
    let store =
        Arc::new(runtime.block_on(async { S3::from_config(config).await?.initialize().await })?);
    // Match reference best-effort startup cleanup, confined to the mount prefix.
    // Report permission/protocol/timeout failures rather than hiding them.
    if let Err(error) = runtime.block_on(store.expire_multipart(std::time::SystemTime::now())) {
        eprintln!("multipart startup cleanup failed: {error}");
    }
    Ok(store)
}

fn serve(
    runtime: &tokio::runtime::Runtime,
    fs: Arc<Filesystem>,
    store: Arc<S3>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mountpoint = fs.config.mountpoint.clone();
    let metrics = spawn_metrics(runtime, &fs)?;
    let stats = spawn_stats(runtime, &fs)?;
    let options = mount_options(&fs.config)?;
    let adapter = Fuse::new(fs.clone(), runtime.handle().clone());
    let notifier = adapter.notifier.clone();
    let shutdown_error = adapter.shutdown_error.clone();
    let mut session = fuser::Session::new(adapter, &mountpoint, &options)?;
    *notifier.lock().unwrap() = Some(session.notifier());
    let signals = spawn_signals(runtime, session.unmount_callable());
    let background = spawn_flush(runtime, fs);
    eprintln!(
        "geesefs-rs: experimental read/write mount; compatibility/performance acceptance pending"
    );
    let result = session.run();
    background.abort();
    let cleanup = runtime.block_on(async {
        let _ = background.await;
        store.drain_cleanup().await
    });
    signals.abort();
    if let Some(metrics) = metrics {
        runtime.block_on(metrics.shutdown());
    }
    if let Some(stats) = stats {
        stats.abort();
    }
    result?;
    cleanup?;
    if let Some(error) = shutdown_error.lock().unwrap().take() {
        return Err(error.into());
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = configuration()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let store = connect_store(&runtime, &config)?;
    let fs = Filesystem::new(config, store.clone())?;
    serve(&runtime, fs, store)
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("geesefs-rs: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
