#![forbid(unsafe_code)]

use clap::Parser;
use geesefs_rs::{
    backend::s3::S3,
    config::{Config, normalize_fstab},
    filesystem::Filesystem,
    fuse::Fuse,
};
use std::{sync::Arc, time::Duration};

#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse_from(normalize_fstab(std::env::args_os().collect()));
    config.validate()?;
    if !config.foreground {
        return Err("background daemonization is unsupported; specify -f".into());
    }
    if !config.mountpoint.is_dir() {
        return Err("mountpoint must be an existing directory".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if config.no_verify_ssl {
        eprintln!("WARNING: S3 TLS certificate verification is disabled by --no-verify-ssl");
    }
    let store =
        Arc::new(runtime.block_on(async { S3::from_config(&config).await?.initialize().await })?);
    // Match reference best-effort startup cleanup, confined to the mount prefix.
    // Report permission/protocol/timeout failures rather than hiding them.
    if let Err(error) = runtime.block_on(store.expire_multipart(std::time::SystemTime::now())) {
        eprintln!("multipart startup cleanup failed: {error}");
    }
    let mountpoint = config.mountpoint.clone();
    let fs = Filesystem::new(config, store.clone())?;
    let stats = if let Some(path) = &fs.config.stats_file {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        let stats_fs = fs.clone();
        Some(runtime.spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_millis(50));
            loop {
                timer.tick().await;
                let allocated = tikv_jemalloc_ctl::epoch::advance()
                    .and_then(|_| tikv_jemalloc_ctl::stats::allocated::read())
                    .ok();
                let sample = serde_json::json!({
                    "unix_time_ns": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                    "monotonic_source": "interval-50ms",
                    "allocator": "jemalloc",
                    "allocated_bytes": allocated,
                    "effective_budgets": stats_fs.effective_budgets(),
                });
                if writeln!(file, "{sample}").is_err() {
                    break;
                }
            }
        }))
    } else {
        None
    };
    let adapter = Fuse::new(fs.clone(), runtime.handle().clone());
    let notifier = adapter.notifier.clone();
    let shutdown_error = adapter.shutdown_error.clone();
    let mut options = fuser::Config::default();
    options
        .mount_options
        .push(fuser::MountOption::FSName("geesefs-rs".into()));
    options
        .mount_options
        .push(fuser::MountOption::DefaultPermissions);
    for option in &fs.config.mount_options {
        options.mount_options.push(match option.as_str() {
            "ro" => fuser::MountOption::RO,
            "rw" => fuser::MountOption::RW,
            "nosuid" => fuser::MountOption::NoSuid,
            "nodev" => fuser::MountOption::NoDev,
            "noexec" => fuser::MountOption::NoExec,
            "default_permissions" => continue,
            _ => return Err("unsupported mount option".into()),
        });
    }
    let mut session = fuser::Session::new(adapter, &mountpoint, &options)?;
    *notifier.lock().unwrap() = Some(session.notifier());
    let mut unmounter = session.unmount_callable();
    let signals = runtime.spawn(async move {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM registration");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
        let _ = tokio::task::spawn_blocking(move || unmounter.unmount()).await;
    });
    let background = runtime.spawn(async move {
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
    });
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

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("geesefs-rs: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
